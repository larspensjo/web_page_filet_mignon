use std::collections::HashMap;
use std::sync::{mpsc, Arc, RwLock};

use chrono::Utc;
use engine_logging::{engine_info, engine_warn};
use harvester_core::{update, AiAvailability, AppState, Effect, LlmQuotaLimits, Msg};
use harvester_engine::llm::prompt::PromptId;
use harvester_engine::llm::prompts::register_defaults;
use harvester_engine::llm::{
    LlmConfig, LlmHandle, LlmQuotas, ModelId, OpenAiProvider, PricingRegistry, PromptRegistry,
    ProviderKind, DEFAULT_BRIEFING_MODEL, DEFAULT_SUMMARY_MODEL, DEFAULT_TRIAGE_MODEL,
};

use crate::{
    load_blacklist, load_completed_jobs, load_signal_candidate_cache,
    load_signal_candidate_overrides, load_summary_cache, load_triage_cache, EffectRunner,
    PlatformEffectHandler, RuntimePaths, RuntimePersistenceSink,
};

/// The LLM settings that intentionally differ between executable hosts.
#[derive(Debug, Clone)]
pub struct HostLlmDefaults {
    pub default_model: ModelId,
    pub session_id_prefix: &'static str,
}

pub const DEFAULT_LLM_MAX_CONCURRENT_REQUESTS: usize = 3;
pub const MAX_LLM_CONCURRENT_REQUESTS: usize = 10;

pub fn parse_llm_max_concurrency_requests(raw: Option<&str>) -> usize {
    raw.and_then(|value| value.trim().parse::<usize>().ok())
        .unwrap_or(DEFAULT_LLM_MAX_CONCURRENT_REQUESTS)
        .clamp(1, MAX_LLM_CONCURRENT_REQUESTS)
}

pub fn llm_max_concurrency_requests_from_env() -> usize {
    let raw = std::env::var("LLM_MAX_CONCURRENT_REQUESTS").ok();
    let value = parse_llm_max_concurrency_requests(raw.as_deref());
    if let Some(raw) = raw {
        engine_info!("[llm-concurrency] LLM_MAX_CONCURRENT_REQUESTS='{raw}' -> {value}");
    }
    value
}

/// Seeds shared GUI facts before hydration and returns effects that must be enqueued.
pub fn prepare_desktop_startup_state(
    mut state: AppState,
    paths: &RuntimePaths,
    llm_max_concurrent_requests: usize,
    startup_ai_availability: Option<AiAvailability>,
    llm_quota_limits: Option<LlmQuotaLimits>,
) -> (AppState, Vec<Effect>) {
    let mut startup_effects = Vec::new();
    state.set_triage_max_in_flight(llm_max_concurrent_requests);
    state.set_summary_max_in_flight(llm_max_concurrent_requests);
    for message in [
        startup_ai_availability.map(|availability| Msg::AiAvailabilityDetected { availability }),
        llm_quota_limits.map(|limits| Msg::LlmQuotaConfigured { limits }),
    ]
    .into_iter()
    .flatten()
    {
        let (next, effects) = update(state, message);
        state = next;
        startup_effects.extend(effects);
    }
    let (state, hydration_effects) = hydrate_state_from_disk(state, paths);
    startup_effects.extend(hydration_effects);
    (state, startup_effects)
}

/// LLM construction details a host may need in addition to its effect runner.
///
/// Batch mode uses these to construct its optional deferred-batch runtime. GUI
/// hosts only need the runner and quota limits.
pub struct HostLlmRuntime {
    pub provider: OpenAiProvider,
    pub config: LlmConfig,
}

pub type EffectRunnerBuild = (EffectRunner, Option<LlmQuotaLimits>, Option<HostLlmRuntime>);

pub fn effective_model_map(config: &LlmConfig) -> HashMap<PromptId, String> {
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

pub fn llm_quota_limits_from_engine(quotas: &LlmQuotas) -> LlmQuotaLimits {
    LlmQuotaLimits {
        max_calls_per_session: quotas.max_calls_per_session.map(u64::from),
        max_input_tokens_per_session: quotas.max_input_tokens_per_session,
        max_output_tokens_per_session: quotas.max_output_tokens_per_session,
        max_cost_microdollars_per_session: quotas.max_cost_microdollars_per_session,
    }
}

/// Builds a host's runner while leaving host-specific batch scheduling and UI
/// platform handling with their owning crates.
#[allow(clippy::too_many_arguments)]
pub fn build_effect_runner(
    paths: &RuntimePaths,
    msg_tx: mpsc::Sender<Msg>,
    llm_concurrency: usize,
    defaults: &HostLlmDefaults,
    platform_handler: Box<dyn PlatformEffectHandler>,
    persistence_sink: Box<dyn RuntimePersistenceSink>,
    missing_api_key_warning: &'static str,
    empty_api_key_warning: Option<&'static str>,
) -> Result<EffectRunnerBuild, String> {
    if let Ok(api_key) = std::env::var("OPENAI_API_KEY") {
        if api_key.trim().is_empty() {
            if let Some(warning) = empty_api_key_warning {
                engine_warn!("{}", warning);
                return Ok((
                    EffectRunner::new(paths.clone(), msg_tx, platform_handler, persistence_sink),
                    None,
                    None,
                ));
            }
        }

        let host_provider = OpenAiProvider::new(api_key);
        let provider: Arc<dyn harvester_engine::llm::provider::LlmProvider> =
            Arc::new(host_provider.clone());
        let mut registry = PromptRegistry::new();
        register_defaults(&mut registry);
        let registry = Arc::new(RwLock::new(registry));
        let quotas = LlmQuotas::default();
        let quota_limits = llm_quota_limits_from_engine(&quotas);
        let config = LlmConfig {
            provider,
            default_model: defaults.default_model.clone(),
            triage_model: Some(ModelId::new(ProviderKind::OpenAi, DEFAULT_TRIAGE_MODEL)),
            summary_model: Some(ModelId::new(ProviderKind::OpenAi, DEFAULT_SUMMARY_MODEL)),
            signal_candidate_model: None,
            briefing_model: Some(ModelId::new(ProviderKind::OpenAi, DEFAULT_BRIEFING_MODEL)),
            registry: Arc::clone(&registry),
            quotas,
            output_dir: paths.output_dir.clone(),
            pricing: PricingRegistry::with_defaults(),
            max_input_bytes: 100_000,
            #[allow(deprecated)]
            max_input_chars: 0,
            timestamp_utc: Arc::new(|| Utc::now().to_rfc3339()),
            session_id: format!(
                "{}{}",
                defaults.session_id_prefix,
                Utc::now().format("%Y%m%d-%H%M%S")
            ),
            replay_cache: None,
            max_concurrent_requests: llm_concurrency,
        };
        let model_map = effective_model_map(&config);
        let runtime = HostLlmRuntime {
            provider: host_provider,
            config: config.clone(),
        };
        let runner = EffectRunner::new_with_llm(
            paths.clone(),
            msg_tx,
            LlmHandle::new(config),
            100_000,
            registry,
            model_map,
            platform_handler,
            persistence_sink,
        );
        Ok((runner, Some(quota_limits), Some(runtime)))
    } else {
        engine_warn!("{}", missing_api_key_warning);
        Ok((
            EffectRunner::new(paths.clone(), msg_tx, platform_handler, persistence_sink),
            None,
            None,
        ))
    }
}

/// Hydrates startup state shared by executable hosts. Manual pre-triage
/// overrides are deliberately excluded; their persistence format remains until
/// phase 7 but no host should honour invisible saved decisions.
pub fn hydrate_state_from_disk(
    mut state: AppState,
    paths: &RuntimePaths,
) -> (AppState, Vec<Effect>) {
    let mut startup_effects = vec![Effect::LoadPromptTemplateFiles];

    let (next_state, effects) = update(state, Msg::StartupHydrationRequested);
    state = next_state;
    startup_effects.extend(effects);

    let completed_jobs = load_completed_jobs(&paths.state_path);
    let completed_job_count = completed_jobs.len();
    if !completed_jobs.is_empty() {
        (state, _) = update(state, Msg::RestoreCompletedJobs(completed_jobs));
    }

    let summary_cache = load_summary_cache(&paths.summary_cache_path);
    if !summary_cache.is_empty() {
        let (next_state, effects) = update(
            state,
            Msg::SummaryCacheHydrated {
                cache: summary_cache,
            },
        );
        state = next_state;
        startup_effects.extend(effects);
    }

    let triage_cache = load_triage_cache(&paths.triage_cache_path);
    if !triage_cache.is_empty() {
        let (next_state, effects) = update(
            state,
            Msg::TriageCacheHydrated {
                cache: triage_cache,
            },
        );
        state = next_state;
        startup_effects.extend(effects);
    }

    match load_signal_candidate_cache(&paths.signal_candidate_cache_path) {
        Ok(cache) if !cache.is_empty() => {
            let (next_state, effects) = update(state, Msg::SignalCandidateCacheLoaded { cache });
            state = next_state;
            startup_effects.extend(effects);
        }
        Ok(_) => {}
        Err(err) => engine_warn!(
            "[signal-cache] failed to hydrate {}: {}",
            paths.signal_candidate_cache_path.display(),
            err
        ),
    }

    match load_signal_candidate_overrides(&paths.signal_candidate_overrides_path) {
        Ok(overrides) if !overrides.is_empty() => {
            let (next_state, effects) =
                update(state, Msg::SignalCandidateOverridesLoaded { overrides });
            state = next_state;
            startup_effects.extend(effects);
        }
        Ok(_) => {}
        Err(err) => engine_warn!(
            "[signal-overrides] failed to hydrate {}: {}",
            paths.signal_candidate_overrides_path.display(),
            err
        ),
    }

    let blacklist = load_blacklist(&paths.blacklist_path);
    if !blacklist.is_empty() {
        (state, _) = update(state, Msg::BlacklistHydrated { state: blacklist });
    }

    engine_info!(
        "[host-bootstrap] Hydrated startup state: completed_jobs={} output_dir={}",
        completed_job_count,
        paths.output_dir.display()
    );

    (state, startup_effects)
}

/// Emits the reducer-owned pre-triage refresh evaluation when the state asks
/// the host loop to perform it.
pub fn pump_pre_triage_refresh(mut state: AppState) -> (AppState, Vec<Effect>, bool) {
    let Some(triggered_by_job_done) = state.take_pre_triage_refresh_evaluation_request() else {
        return (state, Vec::new(), false);
    };
    let ordered_urls = state.ordered_completed_job_urls_snapshot();
    let (next_state, effects) = update(
        state,
        Msg::EvaluatePreTriageRefresh {
            ordered_urls,
            triggered_by_job_done,
        },
    );
    (next_state, effects, true)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_llm_max_concurrency_uses_default_when_missing_or_invalid() {
        assert_eq!(
            parse_llm_max_concurrency_requests(None),
            DEFAULT_LLM_MAX_CONCURRENT_REQUESTS
        );
        assert_eq!(
            parse_llm_max_concurrency_requests(Some("not-a-number")),
            DEFAULT_LLM_MAX_CONCURRENT_REQUESTS
        );
        assert_eq!(
            parse_llm_max_concurrency_requests(Some("")),
            DEFAULT_LLM_MAX_CONCURRENT_REQUESTS
        );
    }

    #[test]
    fn parse_llm_max_concurrency_clamps_to_valid_range() {
        assert_eq!(parse_llm_max_concurrency_requests(Some("1")), 1);
        assert_eq!(parse_llm_max_concurrency_requests(Some("3")), 3);
        assert_eq!(
            parse_llm_max_concurrency_requests(Some("999")),
            MAX_LLM_CONCURRENT_REQUESTS
        );
        assert_eq!(parse_llm_max_concurrency_requests(Some(" 2 ")), 2);
    }

    #[test]
    fn hydrate_state_schedules_llm_metadata_load_once() {
        let dir = tempfile::tempdir().expect("tempdir");
        let paths = RuntimePaths::with_defaults(dir.path().to_path_buf());

        let (_, effects) = hydrate_state_from_disk(AppState::new(), &paths);

        assert_eq!(
            effects
                .iter()
                .filter(|effect| matches!(effect, Effect::LoadLlmMetadata))
                .count(),
            1
        );
    }
}
