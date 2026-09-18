use super::*;
use crate::WorkspaceView;
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
    assert!(effects.is_empty());
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
    assert!(!state.view().dirty, "setting same mode must not mark dirty");
    let _ = view_before;
}

#[test]
fn job_list_mode_persists_across_workspace_switches() {
    init_logging();
    let state = AppState::new();
    let (state, _) = update(
        state,
        Msg::JobListModeSet {
            mode: crate::JobListMode::Last24Hours,
        },
    );
    let (state, _) = update(
        state,
        Msg::WorkspaceViewSet {
            view: WorkspaceView::Trends,
        },
    );
    assert_eq!(state.job_list_mode(), crate::JobListMode::Last24Hours);
    let (state, _) = update(
        state,
        Msg::WorkspaceViewSet {
            view: WorkspaceView::Blacklist,
        },
    );
    assert_eq!(state.job_list_mode(), crate::JobListMode::Last24Hours);
    let (state, _) = update(state, Msg::JobsSearchRevealRequested);
    assert_eq!(state.job_list_mode(), crate::JobListMode::Last24Hours);
}

#[test]
fn jobs_search_query_changed_updates_state() {
    init_logging();
    assert!(
        !AppState::new().view().dirty,
        "new state should start clean so this dirty assertion is load-bearing"
    );
    let (state, effects) = update(
        AppState::new(),
        Msg::JobsSearchQueryChanged("kube".to_string()),
    );

    assert!(effects.is_empty());
    assert_eq!(state.jobs_search_query(), "kube");
    assert!(state.view().dirty);
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
    assert!(state.view().dirty);
}

#[test]
fn jobs_search_query_persists_across_workspace_switch() {
    init_logging();
    let (state, _) = update(
        AppState::new(),
        Msg::JobsSearchQueryChanged("rust".to_string()),
    );
    let (state, _) = update(
        state,
        Msg::WorkspaceViewSet {
            view: WorkspaceView::Trends,
        },
    );
    let (state, _) = update(state, Msg::JobsSearchRevealRequested);

    assert_eq!(state.jobs_search_query(), "rust");
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
fn prompt_lab_close_clears_visibility() {
    init_logging();
    let mut state = AppState::new();
    state.open_prompt_lab();
    assert!(state.prompt_lab().is_visible());
    let (state, _) = update(state, Msg::PromptLabCloseRequested);
    assert!(!state.prompt_lab().is_visible());
}

#[test]
fn triage_clicked_during_run_keeps_desktop_workspace_stable_after_legacy_navigation() {
    init_logging();
    let state = add_completed_job_for_test(AppState::new(), "https://example.com/1");
    let (state, request_id) = tick_until_dispatch(state);
    let state = update(
        state,
        Msg::TriageArticlesLoaded {
            request_id,
            articles: loaded_triage_articles(1),
        },
    )
    .0;
    let state = prime_llm_metadata(state);
    let state = update(
        state,
        Msg::WorkspaceViewSet {
            view: WorkspaceView::Trends,
        },
    )
    .0;
    let state = update(state, Msg::PipelineRunRequested).0;

    let (state, effects) = update(state, Msg::TriageClicked);

    assert!(effects.iter().any(|effect| matches!(
        effect,
        Effect::RequestLlmCompletion {
            prompt_id: PromptId::ArticleTriage,
            ..
        }
    )));
    assert_eq!(state.workspace_view(), WorkspaceView::Trends);
}

#[test]
fn job_selected_during_run_keeps_desktop_workspace_stable_after_legacy_navigation() {
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
        Msg::WorkspaceViewSet {
            view: WorkspaceView::Trends,
        },
    )
    .0;
    let state = update(state, Msg::PipelineRunRequested).0;

    let (state, _) = update(state, Msg::JobSelected { job_id });

    assert_eq!(state.workspace_view(), WorkspaceView::Trends);
}

#[test]
fn ai_availability_defaults_to_available_before_startup_evidence_arrives() {
    init_logging();
    let state = AppState::new();
    assert_eq!(state.ai_availability(), &crate::AiAvailability::Available);
    assert!(state.view().ai_unavailable_message.is_none());
    assert!(state.view().ai_warning_banner.is_none());
}

fn complete_triage_with_settled_summaries(count: usize) -> AppState {
    let mut state = complete_triage_state_for_test(count);
    state = with_summary_metadata(state);
    seed_summaries_for_triage_hashes(&mut state, count);
    state
}

#[test]
fn briefing_generate_enabled_false_when_triage_incomplete_or_corpus_empty() {
    init_logging();
    let view = AppState::new().view();

    assert!(!view.briefing_generate_enabled);
    assert!(!view.summaries_can_start);
}

#[test]
fn briefing_generate_enabled_false_when_summaries_not_settled() {
    init_logging();
    let state = with_summary_metadata(complete_triage_state_for_test(2));

    let view = state.view();
    assert!(!view.briefing_generate_enabled);
    assert!(view.summaries_can_start);
}

#[test]
fn briefing_generate_enabled_false_when_signal_scoring_in_progress() {
    init_logging();
    let mut state = complete_triage_with_settled_summaries(2);
    state = with_signal_candidate_metadata(state);
    let url = "https://triage-complete.com/0".to_string();
    state.signal_candidate_mut().enqueue(url.clone());
    state.signal_candidate_mut().mark_scoring(&url, 99);

    let view = state.view();
    assert!(!view.briefing_generate_enabled);
    assert!(view.summaries_can_start);
}

#[test]
fn briefing_generate_enabled_true_when_summaries_settled_and_signal_idle() {
    init_logging();
    let state = complete_triage_with_settled_summaries(2);

    let view = state.view();
    assert!(view.briefing_generate_enabled);
    assert!(view.summaries_can_start);
}

#[test]
fn briefing_generate_enabled_false_while_briefing_is_running() {
    init_logging();
    let state = complete_triage_with_settled_summaries(1);
    let (state, effects) = update(state, Msg::GenerateBriefingClicked);
    assert!(effects.iter().any(|effect| matches!(
        effect,
        Effect::RequestLlmCompletion {
            prompt_id: PromptId::BriefingExecutiveSummary,
            ..
        }
    )));

    let view = state.view();
    assert!(!view.briefing_generate_enabled);
    assert!(!state.briefing().can_start());
}

#[test]
fn summaries_can_start_false_when_triage_incomplete() {
    init_logging();
    let view = AppState::new().view();

    assert!(!view.summaries_can_start);
}

#[test]
fn summaries_can_start_false_when_ai_unavailable() {
    init_logging();
    let state = with_summary_metadata(complete_triage_state_for_test(1));
    let (state, _) = update(
        state,
        Msg::AiAvailabilityDetected {
            availability: crate::AiAvailability::Unavailable {
                reason: crate::AiUnavailableReason::MissingApiKey,
            },
        },
    );

    let view = state.view();
    assert!(!view.briefing_generate_enabled);
    assert!(!view.summaries_can_start);
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
    assert!(!view.triage_can_start);
    assert!(!view.briefing_generate_enabled);
    assert!(!view.summaries_can_start);
    assert_eq!(
        view.ai_unavailable_message.as_deref(),
        Some("AI features unavailable: OPENAI_API_KEY is not set")
    );
    assert_eq!(
        view.ai_warning_banner,
        Some(crate::InlineWarningView {
            title: "AI features are disabled".to_string(),
            body: "Set OPENAI_API_KEY in the launch environment and restart to enable triage and briefing.".to_string(),
        })
    );
    assert_eq!(
        view.right_pane.triage_markdown.as_deref(),
        Some(
            "AI setup required\n\nTriage is disabled because `OPENAI_API_KEY` is not set.\n\nSet `OPENAI_API_KEY` in the launch environment and restart the app to enable article triage."
        )
    );

    let pre_triage_before = state.pre_triage().resolved_included_urls().to_vec();
    let (state, triage_effects) = update(state, Msg::TriageClicked);
    assert!(
        triage_effects.is_empty(),
        "blocked triage must dispatch nothing"
    );
    assert_eq!(
        state.pre_triage().resolved_included_urls(),
        pre_triage_before
    );

    let (state, briefing_effects) = update(state, Msg::GenerateBriefingClicked);
    assert!(
        briefing_effects.is_empty(),
        "blocked briefing must dispatch nothing"
    );

    let (_state, summary_effects) = update(state, Msg::PrepareSummariesClicked);
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
            templates: std::collections::HashMap::new(),
        },
    );

    assert_eq!(
        state.ai_availability(),
        &crate::AiAvailability::Unavailable {
            reason: crate::AiUnavailableReason::NoTriageModel,
        }
    );
    assert!(state.view().ai_warning_banner.is_none());
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
            templates: std::collections::HashMap::new(),
        },
    );

    assert_eq!(
        state.ai_availability(),
        &crate::AiAvailability::Unavailable {
            reason: crate::AiUnavailableReason::MissingApiKey,
        }
    );
}
