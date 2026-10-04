use super::*;
use harvester_engine::llm::prompt::PromptId;

#[test]
fn job_list_mode_set_updates_state() {
    init_logging();
    let state = AppState::new();
    assert_eq!(state.job_list_mode(), crate::JobListMode::SinceCheckpoint);
    let (state, effects) = update(
        state,
        Msg::JobListModeSet {
            mode: crate::JobListMode::Last24Hours,
        },
    );
    assert!(
        matches!(effects.as_slice(), [Effect::PersistRuntimeState { snapshot }] if snapshot.job_list_mode == Some(crate::JobListMode::Last24Hours))
    );
    assert_eq!(state.job_list_mode(), crate::JobListMode::Last24Hours);
}

#[test]
fn job_list_mode_set_same_value_is_noop() {
    init_logging();
    let state = AppState::new();
    let view_before = state.view();
    let (state, _) = update(
        state,
        Msg::JobListModeSet {
            mode: crate::JobListMode::SinceCheckpoint,
        },
    );
    assert!(
        !{
            let mut snapshot = state.clone();
            snapshot.consume_dirty()
        },
        "setting same mode must not mark dirty"
    );
    let _ = view_before;
}

#[test]
fn jobs_search_query_changed_updates_state() {
    init_logging();
    assert!(
        !AppState::new().consume_dirty(),
        "new state should start clean so this dirty assertion is load-bearing"
    );
    let (state, effects) = update(
        AppState::new(),
        Msg::JobsSearchQueryChanged("kube".to_string()),
    );

    assert!(effects.is_empty());
    assert_eq!(state.jobs_search_query(), "kube");
    assert!({
        let mut snapshot = state.clone();
        snapshot.consume_dirty()
    });
}

#[test]
fn jobs_search_cleared_resets_query() {
    init_logging();
    let (state, _) = update(
        AppState::new(),
        Msg::JobsSearchQueryChanged("kube".to_string()),
    );
    let mut state = state;
    assert!(state.consume_dirty(), "setup should dirty the state");
    let (state, effects) = update(state, Msg::JobsSearchCleared);

    assert!(effects.is_empty());
    assert_eq!(state.jobs_search_query(), "");
    assert!({
        let mut snapshot = state.clone();
        snapshot.consume_dirty()
    });
}

#[test]
fn jobs_search_query_persists_when_jobs_mutate() {
    init_logging();
    let (state, _) = update(
        AppState::new(),
        Msg::JobsSearchQueryChanged("example".to_string()),
    );
    let state = add_completed_job_for_test(state, "https://example.com/article");

    assert_eq!(state.jobs_search_query(), "example");
    assert_eq!(
        state
            .view()
            .desktop_job_list
            .rows
            .iter()
            .map(|row| row.job_id)
            .collect::<Vec<_>>(),
        vec![1]
    );
}

#[test]
fn duplicate_resume_request_keeps_list_mode_stable_during_run() {
    init_logging();
    let state = add_completed_job_for_test(AppState::new(), "https://example.com/1");
    let (state, request_id) = tick_until_dispatch(state);
    let state = update(
        state,
        Msg::TriageArticlesLoaded {
            request_id,
            delta: harvester_engine::TriageArticleDelta::full_window(
                loaded_triage_articles(1),
                100_000,
            ),
        },
    )
    .0;
    let state = prime_llm_metadata(state);
    let state = update(
        state,
        Msg::JobListModeSet {
            mode: crate::JobListMode::Last24Hours,
        },
    )
    .0;
    let (state, effects) = update(
        state,
        Msg::PipelineRunRequested {
            scope: crate::PipelineRunScope::Resume,
        },
    );

    let (state, duplicate_effects) = update(
        state,
        Msg::PipelineRunRequested {
            scope: crate::PipelineRunScope::Resume,
        },
    );
    assert!(duplicate_effects.is_empty());

    assert!(effects.iter().any(|effect| matches!(
        effect,
        Effect::RequestLlmCompletion {
            prompt_id: PromptId::ArticleTriage,
            ..
        }
    )));
    assert_eq!(state.job_list_mode(), crate::JobListMode::Last24Hours);
}

#[test]
fn job_selection_during_run_preserves_list_mode() {
    init_logging();
    let state = add_completed_job_for_test(AppState::new(), "https://example.com/1");
    let job_id = state
        .view()
        .desktop_job_list
        .rows
        .first()
        .expect("prepared job")
        .job_id;
    let state = update(
        state,
        Msg::JobListModeSet {
            mode: crate::JobListMode::Last24Hours,
        },
    )
    .0;
    let state = update(
        state,
        Msg::PipelineRunRequested {
            scope: crate::PipelineRunScope::Resume,
        },
    )
    .0;

    let (state, _) = update(state, Msg::JobSelected { job_id });

    assert_eq!(state.job_list_mode(), crate::JobListMode::Last24Hours);
    assert_eq!(state.selected_job_id(), Some(job_id));
}

#[test]
fn ai_availability_defaults_to_available_before_startup_evidence_arrives() {
    init_logging();
    let state = AppState::new();
    assert_eq!(state.ai_availability(), &crate::AiAvailability::Available);
    assert!(state.view().ai_unavailable_message.is_none());
}

#[test]
fn run_is_enabled_when_triage_and_corpus_are_empty() {
    init_logging();
    let view = AppState::new().view();

    assert!(view.run_enabled);
}

#[test]
fn run_stays_enabled_but_resume_is_disabled_when_ai_is_unavailable() {
    init_logging();
    let mut state =
        with_signal_candidate_metadata(with_summary_metadata(ready_pre_triage_state(&[
            "https://ai-unavailable.example/article",
        ])));
    state.recompute_unfinished_work();
    assert!(matches!(
        state.unfinished_work(),
        crate::UnfinishedWork::Known(summary) if summary.articles_with_work > 0
    ));
    let (state, _) = update(
        state,
        Msg::AiAvailabilityDetected {
            availability: crate::AiAvailability::Unavailable {
                reason: crate::AiUnavailableReason::MissingApiKey,
            },
        },
    );

    let view = state.view();
    assert!(view.run_enabled);
    assert!(!view.resume_enabled);
    assert_eq!(
        view.resume_disabled_reason.as_deref(),
        Some("AI features unavailable: OPENAI_API_KEY is not set")
    );
}

#[test]
fn missing_api_key_blocks_triage_and_briefing_actions() {
    init_logging();
    let state = ready_pre_triage_state(&["https://blocked.example/1"]);
    let (state, _) = update(
        state,
        Msg::AiAvailabilityDetected {
            availability: crate::AiAvailability::Unavailable {
                reason: crate::AiUnavailableReason::MissingApiKey,
            },
        },
    );

    let view = state.view();
    assert!(view.run_enabled);
    assert!(!view.resume_enabled);
    assert_eq!(
        view.ai_unavailable_message.as_deref(),
        Some("AI features unavailable: OPENAI_API_KEY is not set")
    );

    let pre_triage_before = state.pre_triage().resolved_included_urls().to_vec();
    let (state, triage_effects) = update(
        state,
        Msg::PipelineRunRequested {
            scope: crate::PipelineRunScope::Resume,
        },
    );
    assert!(
        triage_effects.is_empty(),
        "blocked triage must dispatch nothing"
    );
    assert_eq!(
        state.pre_triage().resolved_included_urls(),
        pre_triage_before
    );

    let (_state, summary_effects) = update(
        state,
        Msg::PipelineRunRequested {
            scope: crate::PipelineRunScope::Resume,
        },
    );
    assert!(
        summary_effects.is_empty(),
        "blocked summary preparation must dispatch nothing"
    );
}

#[test]
fn llm_metadata_without_triage_model_sets_ai_unavailable_reason() {
    init_logging();
    let mut active_versions = std::collections::HashMap::new();
    active_versions.insert(PromptId::ArticleSummary, 1);
    let mut effective_models = std::collections::HashMap::new();
    effective_models.insert(PromptId::ArticleSummary, "summary-model".to_string());

    let (state, _) = update(
        AppState::new(),
        Msg::LlmMetadataLoaded {
            active_versions,
            effective_models,
        },
    );

    assert_eq!(
        state.ai_availability(),
        &crate::AiAvailability::Unavailable {
            reason: crate::AiUnavailableReason::NoTriageModel,
        }
    );
    assert!(state
        .view()
        .ai_unavailable_message
        .as_deref()
        .unwrap()
        .contains("no triage model"));
}

#[test]
fn valid_triage_metadata_clears_no_triage_model_unavailable_state() {
    init_logging();
    let (state, _) = update(
        AppState::new(),
        Msg::AiAvailabilityDetected {
            availability: crate::AiAvailability::Unavailable {
                reason: crate::AiUnavailableReason::NoTriageModel,
            },
        },
    );
    let state = prime_llm_metadata(state);
    assert_eq!(state.ai_availability(), &crate::AiAvailability::Available);
}

#[test]
fn missing_api_key_is_not_overwritten_by_weaker_metadata_reason() {
    init_logging();
    let (state, _) = update(
        AppState::new(),
        Msg::AiAvailabilityDetected {
            availability: crate::AiAvailability::Unavailable {
                reason: crate::AiUnavailableReason::MissingApiKey,
            },
        },
    );

    let (state, _) = update(
        state,
        Msg::LlmMetadataLoaded {
            active_versions: std::collections::HashMap::new(),
            effective_models: std::collections::HashMap::new(),
        },
    );

    assert_eq!(
        state.ai_availability(),
        &crate::AiAvailability::Unavailable {
            reason: crate::AiUnavailableReason::MissingApiKey,
        }
    );
}
