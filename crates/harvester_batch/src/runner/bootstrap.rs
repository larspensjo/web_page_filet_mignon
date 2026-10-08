use super::{batch_host_llm_defaults, BATCH_EMPTY_API_KEY_WARNING, BATCH_MISSING_API_KEY_WARNING};
use crate::cli::Args;
use engine_logging::engine_info;
use harvester_core::signal_candidate::DEFAULT_SELECTION_THRESHOLD;
use harvester_core::{update, AiAvailability, AppState, Effect, Msg};
use harvester_io::{
    host_bootstrap::{build_effect_runner, hydrate_state_from_disk},
    EffectRunner, NoOpPlatformHandler, PersistenceWorker, RuntimePaths,
};
use std::sync::mpsc;

pub(crate) fn apply_model_budget(state: &mut AppState, args: &Args) {
    state.set_llm_max_in_flight(args.llm_concurrency);
}

pub(crate) fn apply_signal_candidate_selection_settings(state: &mut AppState, args: &Args) {
    state.set_signal_candidate_threshold(
        args.signal_candidate_threshold
            .unwrap_or(DEFAULT_SELECTION_THRESHOLD),
    );
}

pub(crate) fn apply_llm_availability(state: AppState, availability: AiAvailability) -> AppState {
    update(state, Msg::AiAvailabilityDetected { availability }).0
}

pub(crate) fn prepare_runtime(
    paths: &RuntimePaths,
    args: &Args,
    msg_tx: mpsc::Sender<Msg>,
) -> Result<(AppState, EffectRunner), String> {
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
    let (effect_runner, _, availability) = build_effect_runner(
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
    state = apply_llm_availability(state, availability);
    if !startup_effects.is_empty() {
        effect_runner.enqueue(startup_effects);
    }

    Ok((state, effect_runner))
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
    fn synchronous_batch_defaults_to_worker_cap() {
        let args = Args::parse_from(&["harvester_batch"]);
        let mut state = AppState::new();
        apply_model_budget(&mut state, &args);
        assert_eq!(
            state.llm_max_in_flight(),
            harvester_engine::llm::MAX_LLM_CONCURRENT_REQUESTS
        );
    }

    #[test]
    fn missing_and_blank_keys_disarm_full_and_resume_even_after_metadata() {
        let values = [
            Err(std::env::VarError::NotPresent),
            Ok(String::new()),
            Ok(" \t ".to_string()),
        ];
        for value in values {
            let availability = harvester_io::host_bootstrap::host_ai_environment_from_value(
                value,
                "test missing warning",
                Some("test empty warning"),
            )
            .availability;
            for scope in [
                harvester_core::PipelineRunScope::Full,
                harvester_core::PipelineRunScope::Resume,
            ] {
                let state = apply_llm_availability(AppState::new(), availability.clone());
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
                        reason: harvester_core::AiUnavailableReason::MissingApiKey
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
}
