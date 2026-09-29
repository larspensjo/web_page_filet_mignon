use super::batch_runtime::BatchRuntime;
use super::{batch_host_llm_defaults, BATCH_EMPTY_API_KEY_WARNING, BATCH_MISSING_API_KEY_WARNING};
use crate::cli::Args;
use engine_logging::engine_info;
use harvester_core::signal_candidate::DEFAULT_SELECTION_THRESHOLD;
use harvester_core::{
    update, AiAvailability, AiUnavailableReason, AppState, Effect, Msg, PipelineWavePolicy,
};
use harvester_engine::llm::LlmQuotas;
use harvester_io::{
    host_bootstrap::{build_effect_runner, hydrate_state_from_disk},
    EffectRunner, NoOpPlatformHandler, PersistenceWorker, RuntimePaths,
};
use std::sync::mpsc;

pub(crate) fn apply_model_budget(state: &mut AppState, args: &Args) {
    state.set_llm_max_in_flight(args.llm_concurrency);
    if args.batch_api_enabled() {
        let session_limit = LlmQuotas::default()
            .max_calls_per_session
            .map(|limit| limit as usize)
            .unwrap_or(crate::batch_coordinator::MAX_BATCH_LINES);
        state.set_llm_deferred_allowance(session_limit);
    }
    let wave_policy = if args.drain {
        PipelineWavePolicy::Disabled
    } else if args.batch_api_enabled() {
        PipelineWavePolicy::AfterDownloadsSettle
    } else {
        PipelineWavePolicy::Overlap
    };
    state.set_pipeline_wave_policy(wave_policy);
}

pub(crate) fn apply_signal_candidate_selection_settings(state: &mut AppState, args: &Args) {
    state.set_signal_candidate_threshold(
        args.signal_candidate_threshold
            .unwrap_or(DEFAULT_SELECTION_THRESHOLD),
    );
}

pub(crate) fn apply_llm_availability(state: AppState, has_runtime: bool) -> AppState {
    if has_runtime {
        state
    } else {
        update(
            state,
            Msg::AiAvailabilityDetected {
                availability: AiAvailability::Unavailable {
                    reason: AiUnavailableReason::MissingApiKey,
                },
            },
        )
        .0
    }
}

pub(crate) fn prepare_runtime(
    paths: &RuntimePaths,
    args: &Args,
    msg_tx: mpsc::Sender<Msg>,
) -> Result<(AppState, EffectRunner, Option<BatchRuntime>), String> {
    // Hydrate state
    engine_info!("[batch] Hydrating state from disk");
    let (mut state, startup_effects) = hydrate_batch_state(paths, args);

    if let Some(reason) = state.result_store_failure() {
        eprintln!("AI features unavailable: {reason}");
    }

    // Build EffectRunner (with optional LLM support based on OPENAI_API_KEY)
    engine_info!("[batch] Building EffectRunner");
    let platform_handler = Box::new(NoOpPlatformHandler);
    let defaults = batch_host_llm_defaults();
    let (effect_runner, _, runtime) = build_effect_runner(
        paths,
        msg_tx,
        args.llm_concurrency,
        &defaults,
        platform_handler,
        Box::new(PersistenceWorker::new(
            paths.state_path.clone(),
            paths.blacklist_path.clone(),
        )),
        BATCH_MISSING_API_KEY_WARNING,
        Some(BATCH_EMPTY_API_KEY_WARNING),
    )?;
    state = apply_llm_availability(state, runtime.is_some());
    let batch_runtime = if args.batch_api_enabled() && state.result_store_failure().is_none() {
        match runtime {
            Some(runtime) => Some(BatchRuntime::new(runtime.provider, runtime.config, paths)?),
            None => None,
        }
    } else {
        None
    };
    if !startup_effects.is_empty() {
        effect_runner.enqueue(startup_effects);
    }

    Ok((state, effect_runner, batch_runtime))
}

pub(crate) fn hydrate_batch_state(paths: &RuntimePaths, args: &Args) -> (AppState, Vec<Effect>) {
    let mut state = AppState::new();
    apply_model_budget(&mut state, args);
    apply_signal_candidate_selection_settings(&mut state, args);
    hydrate_state_from_disk(state, paths)
}

#[cfg(test)]
mod model_budget_tests {
    use super::*;

    #[test]
    fn batch_api_sets_session_allowance_without_overwriting_sync_budget() {
        let args = Args::parse_from(&["harvester_batch", "--batch-api", "--llm-concurrency", "2"]);
        let mut state = AppState::new();
        apply_model_budget(&mut state, &args);
        assert_eq!(state.llm_max_in_flight(), 2);
        assert_eq!(
            state.pipeline_wave_policy(),
            PipelineWavePolicy::AfterDownloadsSettle
        );
        assert_eq!(
            state.llm_deferred_allowance(),
            LlmQuotas::default()
                .max_calls_per_session
                .map(|n| n as usize)
        );
    }

    #[test]
    fn synchronous_batch_defaults_to_worker_cap_without_deferred_allowance() {
        let args = Args::parse_from(&["harvester_batch"]);
        let mut state = AppState::new();
        apply_model_budget(&mut state, &args);
        assert_eq!(
            state.llm_max_in_flight(),
            harvester_engine::llm::MAX_LLM_CONCURRENT_REQUESTS
        );
        assert_eq!(state.llm_deferred_allowance(), None);
        assert_eq!(state.pipeline_wave_policy(), PipelineWavePolicy::Overlap);
    }

    #[test]
    fn drain_bootstrap_disables_unarmed_intake_refreshes() {
        let args = Args::parse_from(&["harvester_batch", "--drain"]);
        let mut state = AppState::new();
        apply_model_budget(&mut state, &args);
        assert_eq!(state.pipeline_wave_policy(), PipelineWavePolicy::Disabled);
    }

    #[test]
    fn missing_key_disarms_full_and_resume_even_after_metadata() {
        for scope in [
            harvester_core::PipelineRunScope::Full,
            harvester_core::PipelineRunScope::Resume,
        ] {
            let state = apply_llm_availability(AppState::new(), false);
            let (state, _) = update(
                state,
                Msg::LlmMetadataLoaded {
                    active_versions: [(harvester_engine::llm::PromptId::ArticleTriage, 1)]
                        .into_iter()
                        .collect(),
                    effective_models: [(
                        harvester_engine::llm::PromptId::ArticleTriage,
                        "test-model".into(),
                    )]
                    .into_iter()
                    .collect(),
                },
            );
            assert_eq!(
                state.ai_availability(),
                &AiAvailability::Unavailable {
                    reason: AiUnavailableReason::MissingApiKey
                }
            );
            let (state, effects) = update(state, Msg::PipelineRunRequested { scope });
            assert!(!state.pipeline_run_armed());
            assert!(effects.iter().all(|effect| !matches!(
                effect,
                harvester_core::Effect::LoadProcessingConfiguration { .. }
                    | harvester_core::Effect::RequestLlmCompletion { .. }
            )));
        }
    }
}
