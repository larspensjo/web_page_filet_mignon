use super::batch_runtime::BatchRuntime;
use crate::cli::Args;
use chrono::Utc;
use engine_logging::{engine_info, engine_warn};
use harvester_core::signal_candidate::DEFAULT_SELECTION_THRESHOLD;
use harvester_core::{update, AppState, Msg};
use harvester_engine::llm::prompt::PromptId;
use harvester_engine::llm::prompts::register_defaults;
use harvester_engine::llm::{
    LlmConfig, LlmHandle, LlmQuotas, ModelId, OpenAiProvider, PricingRegistry, PromptRegistry,
    ProviderKind, DEFAULT_BRIEFING_MODEL, DEFAULT_SUMMARY_MODEL, DEFAULT_TRIAGE_MODEL,
    OPENAI_MODEL_GPT_4O_MINI,
};
use harvester_io::{
    load_blacklist, load_completed_jobs, load_signal_candidate_cache,
    load_signal_candidate_overrides, load_summary_cache, load_triage_cache, EffectRunner,
    NoOpPlatformHandler, RuntimePaths,
};
use std::collections::HashMap;
use std::sync::mpsc;
use std::sync::Arc;
use std::sync::RwLock;

fn effective_model_map(config: &LlmConfig) -> HashMap<PromptId, String> {
    let mut map = HashMap::new();

    let triage_model = config
        .triage_model
        .as_ref()
        .unwrap_or(&config.default_model)
        .model_name()
        .to_string();
    map.insert(PromptId::ArticleTriage, triage_model);

    let summary_model = config
        .summary_model
        .as_ref()
        .unwrap_or(&config.default_model)
        .model_name()
        .to_string();
    map.insert(PromptId::ArticleSummary, summary_model);

    let signal_candidate_model = config
        .signal_candidate_model
        .as_ref()
        .or(config.summary_model.as_ref())
        .unwrap_or(&config.default_model)
        .model_name()
        .to_string();
    map.insert(PromptId::ArticleSignalCandidate, signal_candidate_model);

    let briefing_model = config
        .briefing_model
        .as_ref()
        .unwrap_or(&config.default_model)
        .model_name()
        .to_string();
    map.insert(PromptId::AggregateBriefing, briefing_model.clone());
    map.insert(PromptId::BriefingExecutiveSummary, briefing_model.clone());
    map.insert(PromptId::BriefingNextItem, briefing_model);

    map
}

pub(crate) fn apply_signal_candidate_selection_settings(state: &mut AppState, args: &Args) {
    state.set_signal_candidate_threshold(
        args.signal_candidate_threshold
            .unwrap_or(DEFAULT_SELECTION_THRESHOLD),
    );
}

pub(crate) fn build_effect_runner(
    paths: &RuntimePaths,
    msg_tx: mpsc::Sender<Msg>,
    llm_concurrency: usize,
    platform_handler: Box<NoOpPlatformHandler>,
    batch_api: bool,
) -> Result<(EffectRunner, Option<BatchRuntime>), String> {
    if let Ok(api_key) = std::env::var("OPENAI_API_KEY") {
        if api_key.trim().is_empty() {
            engine_warn!("[batch] OPENAI_API_KEY is empty; AI triage/summary features disabled");
            return Ok((
                EffectRunner::new(paths.clone(), msg_tx, platform_handler),
                None,
            ));
        }
        let batch_provider = OpenAiProvider::new(api_key);
        let provider: Arc<dyn harvester_engine::llm::provider::LlmProvider> =
            Arc::new(batch_provider.clone());
        let provider_clone = Arc::clone(&provider);
        let mut registry = PromptRegistry::new();
        register_defaults(&mut registry);
        let registry = Arc::new(RwLock::new(registry));
        let config = LlmConfig {
            provider,
            default_model: ModelId::new(ProviderKind::OpenAi, OPENAI_MODEL_GPT_4O_MINI),
            triage_model: Some(ModelId::new(ProviderKind::OpenAi, DEFAULT_TRIAGE_MODEL)),
            summary_model: Some(ModelId::new(ProviderKind::OpenAi, DEFAULT_SUMMARY_MODEL)),
            signal_candidate_model: None,
            briefing_model: Some(ModelId::new(ProviderKind::OpenAi, DEFAULT_BRIEFING_MODEL)),
            registry: Arc::clone(&registry),
            quotas: LlmQuotas::default(),
            output_dir: paths.output_dir.clone(),
            pricing: PricingRegistry::with_defaults(),
            max_input_bytes: 100_000,
            #[allow(deprecated)]
            max_input_chars: 0,
            timestamp_utc: Arc::new(|| Utc::now().to_rfc3339()),
            session_id: format!("batch-{}", Utc::now().format("%Y%m%d-%H%M%S")),
            replay_cache: None,
            max_concurrent_requests: llm_concurrency,
        };
        let model_map = effective_model_map(&config);
        let batch_runtime = if batch_api {
            Some(BatchRuntime::new(batch_provider, config.clone(), paths)?)
        } else {
            None
        };
        let handle = LlmHandle::new(config);
        Ok((
            EffectRunner::new_with_llm(
                paths.clone(),
                msg_tx,
                handle,
                100_000,
                Arc::clone(&registry),
                model_map,
                provider_clone,
                ProviderKind::OpenAi,
                platform_handler,
            ),
            batch_runtime,
        ))
    } else {
        engine_warn!("[batch] OPENAI_API_KEY not set; AI triage/summary features disabled");
        Ok((
            EffectRunner::new(paths.clone(), msg_tx, platform_handler),
            None,
        ))
    }
}

pub(crate) fn is_ai_orchestration_enabled() -> bool {
    std::env::var("OPENAI_API_KEY")
        .map(|value| !value.trim().is_empty())
        .unwrap_or(false)
}

/// Drain must never orchestrate. Restored completed jobs feed the pre-triage
/// session, so orchestration would dispatch triage over the whole corpus and
/// submit fresh batches — exactly the new work drain exists to avoid.
pub(crate) fn should_enable_ai_orchestration_for_mode(
    api_key_available: bool,
    drain: bool,
) -> bool {
    api_key_available && !drain
}

#[allow(clippy::type_complexity)]
pub(crate) fn prepare_runtime(
    paths: &RuntimePaths,
    args: &Args,
    msg_tx: mpsc::Sender<Msg>,
) -> Result<(AppState, EffectRunner, Option<BatchRuntime>, bool), String> {
    // Hydrate state
    engine_info!("[batch] Hydrating state from disk");
    let mut state = AppState::new();
    if args.batch_api_enabled() {
        let session_limit = LlmQuotas::default()
            .max_calls_per_session
            .map(|limit| limit as usize)
            .unwrap_or(crate::batch_coordinator::MAX_BATCH_LINES);
        state.set_deferred_batch_max_in_flight(session_limit);
    } else {
        state.set_triage_max_in_flight(args.llm_concurrency);
        state.set_summary_max_in_flight(args.llm_concurrency);
    }
    apply_signal_candidate_selection_settings(&mut state, args);

    // Restore completed jobs
    let completed_jobs = load_completed_jobs(&paths.state_path);
    if !completed_jobs.is_empty() {
        engine_info!("[batch] Restoring {} completed jobs", completed_jobs.len());
        let (new_state, effects) = update(state, Msg::RestoreCompletedJobs(completed_jobs));
        state = new_state;
        // Effects from restore are cache loads which will be executed shortly
        for effect in effects {
            engine_info!("[batch] Restore effect queued: {:?}", effect);
        }
    } else {
        engine_info!("[batch] No previous state found, starting fresh");
    }

    // Hydrate domain blacklist.
    let blacklist = load_blacklist(&paths.blacklist_path);
    if !blacklist.is_empty() {
        let (new_state, _) = update(state, Msg::BlacklistHydrated { state: blacklist });
        state = new_state;
    }

    // Build EffectRunner (with optional LLM support based on OPENAI_API_KEY)
    engine_info!("[batch] Building EffectRunner");
    let enable_ai_orchestration =
        should_enable_ai_orchestration_for_mode(is_ai_orchestration_enabled(), args.drain);
    let platform_handler = Box::new(NoOpPlatformHandler);
    let (effect_runner, batch_runtime) = build_effect_runner(
        paths,
        msg_tx,
        args.llm_concurrency,
        platform_handler,
        args.batch_api_enabled(),
    )?;
    effect_runner.enqueue(vec![
        harvester_core::Effect::LoadPromptTemplateFiles,
        harvester_core::Effect::LoadLlmMetadata,
    ]);

    // Trigger reducer-owned metadata hydration.
    let (new_state, startup_effects) = update(state, Msg::StartupHydrationRequested);
    state = new_state;
    if !startup_effects.is_empty() {
        effect_runner.enqueue(startup_effects);
    }

    // Hydrate persistent caches for triage/summary reuse.
    let summary_cache = load_summary_cache(&paths.summary_cache_path);
    if !summary_cache.is_empty() {
        let (new_state, effects) = update(
            state,
            Msg::SummaryCacheHydrated {
                cache: summary_cache,
            },
        );
        state = new_state;
        if !effects.is_empty() {
            effect_runner.enqueue(effects);
        }
    }
    let triage_cache = load_triage_cache(&paths.triage_cache_path);
    if !triage_cache.is_empty() {
        let (new_state, effects) = update(
            state,
            Msg::TriageCacheHydrated {
                cache: triage_cache,
            },
        );
        state = new_state;
        if !effects.is_empty() {
            effect_runner.enqueue(effects);
        }
    }
    match load_signal_candidate_cache(&paths.signal_candidate_cache_path) {
        Ok(signal_candidate_cache) if !signal_candidate_cache.is_empty() => {
            let (new_state, effects) = update(
                state,
                Msg::SignalCandidateCacheLoaded {
                    cache: signal_candidate_cache,
                },
            );
            state = new_state;
            if !effects.is_empty() {
                effect_runner.enqueue(effects);
            }
        }
        Ok(_) => {}
        Err(err) => engine_warn!(
            "[signal-cache] failed to hydrate {}: {}",
            paths.signal_candidate_cache_path.display(),
            err
        ),
    }
    match load_signal_candidate_overrides(&paths.signal_candidate_overrides_path) {
        Ok(signal_candidate_overrides) if !signal_candidate_overrides.is_empty() => {
            let (new_state, effects) = update(
                state,
                Msg::SignalCandidateOverridesLoaded {
                    overrides: signal_candidate_overrides,
                },
            );
            state = new_state;
            if !effects.is_empty() {
                effect_runner.enqueue(effects);
            }
        }
        Ok(_) => {}
        Err(err) => engine_warn!(
            "[signal-overrides] failed to hydrate {}: {}",
            paths.signal_candidate_overrides_path.display(),
            err
        ),
    }

    Ok((state, effect_runner, batch_runtime, enable_ai_orchestration))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn drain_never_orchestrates_so_no_new_batches_are_submitted() {
        // Restored completed jobs feed pre-triage, so leaving orchestration on
        // would let a drain dispatch triage and submit fresh batch work.
        assert!(!should_enable_ai_orchestration_for_mode(true, true));
        assert!(should_enable_ai_orchestration_for_mode(true, false));
        assert!(!should_enable_ai_orchestration_for_mode(false, false));
    }
}
