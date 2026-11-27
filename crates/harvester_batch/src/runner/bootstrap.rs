use super::batch_host_llm_defaults;
use crate::cli::Args;
use crate::services::RunnerBuilder;
use engine_logging::engine_info;
use harvester_core::signal_candidate::DEFAULT_SELECTION_THRESHOLD;
use harvester_core::{update, AiAvailability, AppState, Effect, Msg};
#[cfg(test)]
use harvester_io::{credentials::StoredKeyAccess, host_bootstrap::read_startup_key_outcomes};
use harvester_io::{
    credentials::{CredentialEntry, StoredKeyReads},
    host_bootstrap::{hydrate_state_from_disk, HostKeys},
    EffectRunner, RuntimePaths,
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

#[cfg(test)]
fn read_startup_keys(
    access: Option<&StoredKeyAccess>,
    reader: impl FnOnce(&StoredKeyAccess) -> StoredKeyReads,
) -> HostKeys {
    report_startup_keys(read_startup_key_outcomes(access, reader))
}

pub(super) fn report_startup_keys(reads: StoredKeyReads) -> HostKeys {
    println!("{}", reads.openai.status_line(CredentialEntry::OpenAi));
    println!("{}", reads.brave.status_line(CredentialEntry::BraveSearch));
    reads.into()
}

pub(crate) fn prepare_runtime(
    paths: &RuntimePaths,
    args: &Args,
    msg_tx: mpsc::Sender<Msg>,
    keys: HostKeys,
    build_runner: Box<RunnerBuilder>,
) -> Result<(AppState, EffectRunner, Vec<Effect>), String> {
    // Hydrate state
    engine_info!("[batch] Hydrating state from disk");
    let (mut state, mut startup_effects) = hydrate_batch_state(paths, args);
    if let Some(target) = &args.archive_out {
        let (configured, effects) = update(
            state,
            Msg::BatchArchiveConfigured {
                target: target.clone(),
            },
        );
        state = configured;
        startup_effects.extend(effects);
    }

    if let Some(reason) = state.result_store_failure() {
        eprintln!("AI features unavailable: {reason}");
    }

    engine_info!("[batch] Building EffectRunner");
    let (effect_runner, _, availability) =
        build_runner(paths, args, msg_tx, keys, &batch_host_llm_defaults())?;
    state = apply_llm_availability(state, availability);
    Ok((state, effect_runner, startup_effects))
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
    fn keyless_batch_bootstrap_does_not_call_the_credential_reader() {
        use harvester_io::credentials::KeyReadOutcome;
        let reads = std::cell::Cell::new(0);
        let keys = read_startup_keys(None, |_| {
            reads.set(reads.get() + 1);
            StoredKeyReads {
                openai: KeyReadOutcome::NotFound,
                brave: KeyReadOutcome::NotFound,
            }
        });
        assert_eq!(reads.get(), 0);
        assert!(keys.brave.is_none());
        assert!(matches!(keys.openai, KeyReadOutcome::NotRequested));
        assert_eq!(
            harvester_io::host_bootstrap::host_ai_environment_from_key(
                keys.openai,
                "AI triage/summary features disabled",
            )
            .availability,
            AiAvailability::Unavailable {
                reason: harvester_core::AiUnavailableReason::MissingApiKey,
            },
        );
    }

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
    fn unavailable_keys_disarm_full_and_resume_even_after_metadata() {
        use harvester_io::credentials::KeyReadOutcome;
        for outcome in [
            KeyReadOutcome::NotRequested,
            KeyReadOutcome::NotFound,
            KeyReadOutcome::Empty,
        ] {
            let availability = harvester_io::host_bootstrap::host_ai_environment_from_key(
                outcome,
                "AI triage/summary features disabled",
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
