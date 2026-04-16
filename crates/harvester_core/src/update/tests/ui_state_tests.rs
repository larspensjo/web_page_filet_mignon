use super::support::*;
use super::*;
use harvester_engine::llm::prompt::PromptId;

#[test]
fn left_tab_selected_jobs_updates_tab_and_dirty() {
    init_logging();
    let state = AppState::new();
    assert_eq!(state.left_tab(), LeftTab::Jobs);
    let (state, effects) = update(
        state,
        Msg::LeftTabSelected {
            tab: LeftTab::TriageReview,
        },
    );
    assert!(effects.is_empty());
    assert_eq!(state.left_tab(), LeftTab::TriageReview);
    assert!(state.view().dirty);
}

#[test]
fn left_tab_selected_triage_results_updates_tab() {
    init_logging();
    let (state, _) = update(
        AppState::new(),
        Msg::LeftTabSelected {
            tab: LeftTab::TriageResults,
        },
    );
    assert_eq!(state.left_tab(), LeftTab::TriageResults);
}

#[test]
fn job_list_scope_set_to_since_checkpoint_updates_state() {
    init_logging();
    let state = AppState::new();
    assert_eq!(state.job_list_scope(), JobListScope::SinceCheckpoint);
    let (state, effects) = update(
        state,
        Msg::JobListScopeSet {
            scope: JobListScope::All,
        },
    );
    assert!(effects.is_empty());
    assert_eq!(state.job_list_scope(), JobListScope::All);
    assert!(state.view().dirty);
}

#[test]
fn job_list_scope_set_same_value_is_noop() {
    init_logging();
    let state = AppState::new();
    let view_before = state.view();
    let (state, _) = update(
        state,
        Msg::JobListScopeSet {
            scope: JobListScope::SinceCheckpoint,
        },
    );
    assert!(
        !state.view().dirty,
        "setting same scope must not mark dirty"
    );
    let _ = view_before;
}

#[test]
fn job_list_scope_persists_across_tab_switches() {
    init_logging();
    let state = AppState::new();
    let (state, _) = update(
        state,
        Msg::JobListScopeSet {
            scope: JobListScope::SinceCheckpoint,
        },
    );
    let (state, _) = update(
        state,
        Msg::LeftTabSelected {
            tab: LeftTab::TriageReview,
        },
    );
    assert_eq!(state.job_list_scope(), JobListScope::SinceCheckpoint);
    let (state, _) = update(
        state,
        Msg::LeftTabSelected {
            tab: LeftTab::TriageResults,
        },
    );
    assert_eq!(state.job_list_scope(), JobListScope::SinceCheckpoint);
    let (state, _) = update(state, Msg::LeftTabSelected { tab: LeftTab::Jobs });
    assert_eq!(state.job_list_scope(), JobListScope::SinceCheckpoint);
}

#[test]
fn prompt_lab_close_restores_jobs_tab() {
    init_logging();
    let mut state = AppState::new();
    state.open_prompt_lab();
    assert_eq!(state.left_tab(), LeftTab::PromptLab);
    let (state, _) = update(state, Msg::PromptLabCloseRequested);
    assert_eq!(state.left_tab(), LeftTab::Jobs);
}

#[test]
fn triage_clicked_switches_to_triage_results_tab_when_triage_can_start() {
    init_logging();
    let state = add_completed_job_for_test(AppState::new(), "https://example.com/1");
    let (state, request_id) = tick_until_dispatch(state);
    let (state, _) = update(
        state,
        Msg::TriageArticlesLoaded {
            request_id,
            articles: loaded_triage_articles(1),
        },
    );
    let (state, _) = update(state, Msg::LeftTabSelected { tab: LeftTab::Jobs });
    let (state, _) = update(state, Msg::TriageClicked);
    assert_eq!(state.left_tab(), LeftTab::TriageResults);
}

#[test]
fn ai_availability_defaults_to_available_before_startup_evidence_arrives() {
    init_logging();
    let state = AppState::new();
    assert_eq!(state.ai_availability(), &crate::AiAvailability::Available);
    assert!(state.view().ai_unavailable_message.is_none());
    assert!(state.view().ai_warning_banner.is_none());
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
    assert!(!view.briefing_can_start);
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
    assert_eq!(
        view.right_pane.briefing_markdown.as_deref(),
        Some(
            "AI setup required\n\nBriefing is disabled because `OPENAI_API_KEY` is not set.\n\nSet `OPENAI_API_KEY` in the launch environment and restart the app to enable briefing generation."
        )
    );

    let pre_triage_before = state.pre_triage().resolved_included_urls().to_vec();
    let (state, triage_effects) = update(state, Msg::TriageClicked);
    assert!(
        triage_effects.is_empty(),
        "blocked triage must dispatch nothing"
    );
    assert_eq!(state.left_tab(), LeftTab::TriageResults);
    assert_eq!(
        state.pre_triage().resolved_included_urls(),
        pre_triage_before
    );

    let (state, briefing_effects) = update(state, Msg::GenerateBriefingClicked);
    assert!(
        briefing_effects.is_empty(),
        "blocked briefing must dispatch nothing"
    );
    assert_eq!(state.active_tab(), AppTab::Summary);
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
