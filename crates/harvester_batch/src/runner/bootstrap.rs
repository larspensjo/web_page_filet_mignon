use super::batch_runtime::BatchRuntime;
use super::{batch_host_llm_defaults, BATCH_EMPTY_API_KEY_WARNING, BATCH_MISSING_API_KEY_WARNING};
use crate::cli::Args;
use engine_logging::engine_info;
use harvester_core::signal_candidate::DEFAULT_SELECTION_THRESHOLD;
use harvester_core::{AppState, Msg};
use harvester_engine::llm::LlmQuotas;
use harvester_io::{
    host_bootstrap::{build_effect_runner, hydrate_state_from_disk},
    EffectRunner, NoOpPlatformHandler, PersistenceWorker, RuntimePaths,
};
use std::sync::mpsc;

pub(crate) fn apply_signal_candidate_selection_settings(state: &mut AppState, args: &Args) {
    state.set_signal_candidate_threshold(
        args.signal_candidate_threshold
            .unwrap_or(DEFAULT_SELECTION_THRESHOLD),
    );
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

    let (hydrated_state, startup_effects) = hydrate_state_from_disk(state, paths);
    state = hydrated_state;

    // Build EffectRunner (with optional LLM support based on OPENAI_API_KEY)
    engine_info!("[batch] Building EffectRunner");
    let enable_ai_orchestration =
        should_enable_ai_orchestration_for_mode(is_ai_orchestration_enabled(), args.drain);
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
    let batch_runtime = if args.batch_api_enabled() {
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
