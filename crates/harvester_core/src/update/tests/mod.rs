use super::*;
use crate::briefing::{ArticleSummaryState, BriefingPhase, LoadedArticle};
use crate::LlmResultKind;
use harvester_engine::llm::prompt::PromptId;
use harvester_engine::llm::{run_metadata::LlmRunMetadata, DEFAULT_BRIEFING_MODEL};

mod support;
use support::*;

#[test]
fn prompt_context_load_failure_keeps_triage_metadata_unready() {
    init_logging();
    let mut active_versions = std::collections::HashMap::new();
    active_versions.insert(PromptId::ArticleTriage, 4);
    let mut effective_models = std::collections::HashMap::new();
    effective_models.insert(PromptId::ArticleTriage, "test-model".to_string());
    let mut contexts = std::collections::HashMap::new();
    contexts.insert(
        PromptId::ArticleTriage,
        vec![("policy".to_string(), "test policy".to_string())],
    );

    let (state, _) = update(
        AppState::new(),
        Msg::LlmMetadataLoaded {
            active_versions,
            effective_models,
            templates: std::collections::HashMap::new(),
        },
    );
    let (state, _) = update(state, Msg::PromptContextsLoaded { contexts });
    assert!(state.triage_metadata_ready());

    let (state, _) = update(
        state,
        Msg::PromptContextsLoadFailed {
            reason: "required ArticleTriage context file missing".to_string(),
        },
    );
    assert!(!state.triage_metadata_ready());

    let mut active_versions = std::collections::HashMap::new();
    active_versions.insert(PromptId::ArticleTriage, 4);
    let mut effective_models = std::collections::HashMap::new();
    effective_models.insert(PromptId::ArticleTriage, "test-model".to_string());
    let (state, _) = update(
        state,
        Msg::LlmMetadataLoaded {
            active_versions,
            effective_models,
            templates: std::collections::HashMap::new(),
        },
    );
    assert!(!state.triage_metadata_ready());
}

#[test]
fn generate_briefing_emits_load_effect() {
    init_logging();
    let state = AppState::new();
    let (state, effects) = update(state, Msg::GenerateBriefingClicked);

    assert_eq!(
        effects,
        vec![
            Effect::LoadPromptContexts,
            Effect::LoadPromptTemplateFiles,
            Effect::LoadLlmMetadata,
            Effect::LoadArticlesForBriefingPrereq {
                ordered_urls: Vec::new(),
                since_utc: None,
            }
        ]
    );
    assert_eq!(state.briefing().phase(), &BriefingPhase::WaitingForTriage);
    assert_eq!(state.active_tab(), AppTab::Briefing);
}

#[test]
fn articles_loaded_dispatches_first_summary() {
    init_logging();
    let state = AppState::new();
    let state = start_briefing_after_triage(state, loaded_articles().0.clone());
    let (articles, collection_text) = loaded_articles();

    let (state, effects) = update(
        state,
        Msg::ArticlesLoaded {
            articles,
            collection_text,
        },
    );

    assert_eq!(effects.len(), 1);
    let summary_req_id =
        request_id_for_prompt(&effects, PromptId::ArticleSummary).expect("summary request");
    assert!(matches!(
        &effects[0],
        Effect::RequestLlmCompletion {
            prompt_id: PromptId::ArticleSummary,
            prompt_version: None,
            model_override: None,
            input_content,
            context,
            template_override: None,
            ..
        } if input_content.starts_with("Article A text") && context.is_empty()
    ));
    assert!(matches!(
        state.briefing().phase(),
        BriefingPhase::Summarizing
    ));
    assert!(matches!(
        state.briefing().articles()[0].summary_state,
        ArticleSummaryState::InProgress { request_id } if request_id == summary_req_id
    ));
}

#[test]
fn summary_completion_advances_and_generates_briefing() {
    init_logging();
    let state = AppState::new();
    let state = start_briefing_after_triage(state, loaded_articles().0.clone());
    let (articles, collection_text) = loaded_articles();

    // Capture the Article A summary request ID from the effect so the test
    // does not depend on prior allocation counts inside the setup helpers.
    let (state, articles_effects) = update(
        state,
        Msg::ArticlesLoaded {
            articles,
            collection_text,
        },
    );
    let req_a = request_id_for_prompt(&articles_effects, PromptId::ArticleSummary)
        .expect("Article A summary request");

    let (state, effects) = update(
        state,
        Msg::LlmCompleted {
            request_id: req_a,
            result: LlmResultKind::Success {
                output_json: summary_json("Article A"),
                input_tokens: 10,
                output_tokens: 5,
                prompt_version: 1,
                model_id: "test-model".to_string(),
            },
            metadata: None,
        },
    );

    // effects[0] = UpsertEntityIndexEntry for Article A
    // effects[1] = RequestLlmCompletion for Article B
    assert_eq!(effects.len(), 2);
    let req_b = request_id_for_prompt(&effects, PromptId::ArticleSummary)
        .expect("Article B summary request");
    assert_ne!(req_b, req_a, "each summary request must have a distinct id");
    assert!(matches!(
        &effects[1],
        Effect::RequestLlmCompletion {
            prompt_id: PromptId::ArticleSummary,
            prompt_version: None,
            model_override: None,
            input_content,
            context,
            template_override: None,
            ..
        } if input_content.starts_with("Article B text") && context.is_empty()
    ));

    let (state, effects) = update(
        state,
        Msg::LlmCompleted {
            request_id: req_b,
            result: LlmResultKind::Success {
                output_json: summary_json("Article B"),
                input_tokens: 10,
                output_tokens: 5,
                prompt_version: 1,
                model_id: "test-model".to_string(),
            },
            metadata: None,
        },
    );

    // effects[0] = UpsertEntityIndexEntry for Article B
    // effects[1] = RequestLlmCompletion for AggregateBriefing
    assert_eq!(effects.len(), 2);
    let req_c = request_id_for_prompt(&effects, PromptId::AggregateBriefing)
        .expect("aggregate briefing request");
    assert_ne!(
        req_c, req_b,
        "briefing request must have a distinct id from last summary"
    );
    match &effects[1] {
        Effect::RequestLlmCompletion {
            prompt_id,
            prompt_version,
            model_override,
            input_content,
            context,
            template_override,
            extra_template_vars,
            ..
        } => {
            assert_eq!(*prompt_id, PromptId::AggregateBriefing);
            assert_eq!(*prompt_version, None);
            assert_eq!(*model_override, None);
            assert_eq!(input_content, "Collection text");
            assert!(context.is_empty());
            assert!(template_override.is_none());
            assert!(extra_template_vars
                .iter()
                .any(|(k, v)| k == "previous_briefings" && v == "(none)"));
            assert!(extra_template_vars.iter().any(|(k, v)| {
                k == "briefing_time_window" && v.contains("All available articles")
            }));
        }
        other => panic!("expected aggregate briefing request, got {other:?}"),
    }

    let (state, effects) = update(
        state,
        Msg::LlmCompleted {
            request_id: req_c,
            result: LlmResultKind::Success {
                output_json: briefing_json(2),
                input_tokens: 20,
                output_tokens: 8,
                prompt_version: 1,
                model_id: "test-model".to_string(),
            },
            metadata: None,
        },
    );

    assert!(effects
        .iter()
        .any(|e| matches!(e, Effect::PersistSummaryCache { .. })));
    assert!(effects
        .iter()
        .any(|e| matches!(e, Effect::SaveBriefingHistory { .. })));
    assert_eq!(state.briefing().phase(), &BriefingPhase::Complete);
    assert!(state.briefing().briefing_result().is_some());
}

#[test]
fn summary_store_uses_run_frozen_metadata_when_completion_model_differs() {
    init_logging();
    let state = AppState::new();
    let state = start_briefing_after_triage(state, loaded_single_article().0.clone());
    let (articles, collection_text) = loaded_single_article();
    let (state, _) = update(
        state,
        Msg::ArticlesLoaded {
            articles,
            collection_text,
        },
    );

    let (state, _) = update(
        state,
        Msg::LlmCompleted {
            request_id: 2,
            result: LlmResultKind::Success {
                output_json: summary_json("Article A"),
                input_tokens: 10,
                output_tokens: 5,
                prompt_version: 77,
                model_id: "test-model-2024-07-18".to_string(),
            },
            metadata: None,
        },
    );

    let keys: Vec<_> = state
        .summary_cache()
        .iter()
        .map(|(key, _)| key.clone())
        .collect();
    assert_eq!(keys.len(), 1);
    assert_eq!(keys[0].prompt_version, 1);
    assert_eq!(keys[0].model_id, "test-model");
}

#[test]
fn aggregate_briefing_failure_surfaces_reason_in_briefing_ui() {
    init_logging();
    let state = AppState::new();
    let state = start_briefing_after_triage(state, loaded_single_article().0.clone());
    let (articles, collection_text) = loaded_single_article();
    let (state, _) = update(
        state,
        Msg::ArticlesLoaded {
            articles,
            collection_text,
        },
    );

    let (state, effects) = update(
        state,
        Msg::LlmCompleted {
            request_id: 2,
            result: LlmResultKind::Success {
                output_json: summary_json("Article A"),
                input_tokens: 10,
                output_tokens: 5,
                prompt_version: 1,
                model_id: "test-model".to_string(),
            },
            metadata: None,
        },
    );

    let aggregate_request_id = request_id_for_prompt(&effects, PromptId::AggregateBriefing)
        .expect("aggregate briefing request");
    let (state, _) = update(
        state,
        Msg::LlmCompleted {
            request_id: aggregate_request_id,
            result: LlmResultKind::Failed {
                reason: "request timed out".to_string(),
            },
            metadata: None,
        },
    );

    assert_eq!(
        state.briefing().phase(),
        &BriefingPhase::Failed {
            reason: "request timed out".to_string()
        }
    );
    let view = state.view();
    assert!(view
        .right_pane
        .briefing_markdown
        .as_deref()
        .unwrap_or("")
        .contains("request timed out"));
}

#[test]
fn aggregate_briefing_success_records_usage_for_status_bar() {
    init_logging();
    let state = AppState::new();
    let state = start_briefing_after_triage(state, loaded_single_article().0.clone());
    let (articles, collection_text) = loaded_single_article();
    let (state, _) = update(
        state,
        Msg::ArticlesLoaded {
            articles,
            collection_text,
        },
    );

    let (state, effects) = update(
        state,
        Msg::LlmCompleted {
            request_id: 2,
            result: LlmResultKind::Success {
                output_json: summary_json("Article A"),
                input_tokens: 10,
                output_tokens: 5,
                prompt_version: 1,
                model_id: "test-model".to_string(),
            },
            metadata: None,
        },
    );

    let aggregate_request_id = request_id_for_prompt(&effects, PromptId::AggregateBriefing)
        .expect("aggregate briefing request");
    let (state, _) = update(
        state,
        Msg::LlmCompleted {
            request_id: aggregate_request_id,
            result: LlmResultKind::Success {
                output_json: briefing_json(1),
                input_tokens: 123,
                output_tokens: 45,
                prompt_version: 1,
                model_id: DEFAULT_BRIEFING_MODEL.to_string(),
            },
            metadata: Some(aggregate_briefing_metadata(DEFAULT_BRIEFING_MODEL, 123, 45)),
        },
    );

    let view = state.view();
    assert_eq!(view.llm_usage_by_model.len(), 1);
    assert_eq!(view.llm_usage_by_model[0].model, DEFAULT_BRIEFING_MODEL);
    assert_eq!(view.llm_usage_by_model[0].input_tokens, 123);
    assert_eq!(view.llm_usage_by_model[0].output_tokens, 45);
}

#[test]
fn second_run_reuses_cached_summary_with_configured_model_key() {
    init_logging();
    let state = AppState::new();
    let state = start_briefing_after_triage(state, loaded_single_article().0.clone());
    let (articles, collection_text) = loaded_single_article();
    let (state, _) = update(
        state,
        Msg::ArticlesLoaded {
            articles,
            collection_text,
        },
    );
    let (state, effects) = update(
        state,
        Msg::LlmCompleted {
            request_id: 2,
            result: LlmResultKind::Success {
                output_json: summary_json("Article A"),
                input_tokens: 10,
                output_tokens: 5,
                prompt_version: 88,
                model_id: "test-model-2024-07-18".to_string(),
            },
            metadata: None,
        },
    );
    // effects[0] = UpsertEntityIndexEntry for Article A
    // effects[1] = RequestLlmCompletion for AggregateBriefing
    assert_eq!(effects.len(), 2);
    match &effects[1] {
        Effect::RequestLlmCompletion {
            request_id,
            prompt_id,
            prompt_version,
            model_override,
            input_content,
            context,
            template_override,
            extra_template_vars,
        } => {
            assert_eq!(*request_id, 3);
            assert_eq!(*prompt_id, PromptId::AggregateBriefing);
            assert_eq!(*prompt_version, None);
            assert_eq!(*model_override, None);
            assert_eq!(input_content, "Collection text");
            assert!(context.is_empty());
            assert!(template_override.is_none());
            assert!(extra_template_vars
                .iter()
                .any(|(k, v)| k == "previous_briefings" && v == "(none)"));
            assert!(extra_template_vars.iter().any(|(k, v)| {
                k == "briefing_time_window" && v.contains("All available articles")
            }));
        }
        other => panic!("expected aggregate briefing request, got {other:?}"),
    }
    let (state, _) = update(
        state,
        Msg::LlmCompleted {
            request_id: 3,
            result: LlmResultKind::Success {
                output_json: briefing_json(1),
                input_tokens: 10,
                output_tokens: 4,
                prompt_version: 1,
                model_id: "test-model".to_string(),
            },
            metadata: None,
        },
    );

    let (state, effects) = update(state, Msg::GenerateBriefingClicked);
    assert_eq!(
        effects,
        vec![
            Effect::LoadPromptContexts,
            Effect::LoadPromptTemplateFiles,
            Effect::LoadLlmMetadata,
            Effect::LoadArticlesForBriefingPrereq {
                ordered_urls: Vec::new(),
                since_utc: None,
            }
        ]
    );
    let state = with_summary_metadata(state);
    let (state, effects) = update(
        state,
        Msg::BriefingPrereqArticlesLoaded {
            articles: loaded_single_article().0,
        },
    );
    assert!(effects
        .iter()
        .any(|effect| matches!(effect, Effect::LoadArticlesForBriefing { .. })));
    let (articles, collection_text) = loaded_single_article();
    let (_state, effects) = update(
        state,
        Msg::ArticlesLoaded {
            articles,
            collection_text,
        },
    );

    // effects[0] = UpsertEntityIndexEntry (summary cache hit for Article A)
    // effects[1] = RequestLlmCompletion for AggregateBriefing
    assert_eq!(effects.len(), 2);
    assert!(matches!(
        &effects[1],
        Effect::RequestLlmCompletion {
            request_id: 4,
            prompt_id: PromptId::AggregateBriefing,
            input_content,
            extra_template_vars,
            ..
        } if input_content == "Collection text"
            // History was set on the first run — previous_briefings must now be non-empty
            && extra_template_vars.iter().any(|(k, v)| k == "previous_briefings" && v != "(none)")
    ));
}

#[test]
fn splitter_move_preserves_minimum_jobs_width_with_fixed_input_panel() {
    init_logging();
    let state = AppState::new();

    let (state, effects) = update(
        state,
        Msg::SplitterMoved {
            desired_left_width_px: 300,
        },
    );

    assert!(effects.is_empty());
    assert_eq!(
        state.left_panel_width(),
        INPUT_PANEL_FIXED_WIDTH + MIN_JOBS_PANEL_WIDTH
    );
}

#[test]
fn open_in_browser_with_summarized_job_selected_emits_effect() {
    init_logging();
    let state = make_state_with_summarized_job_for_update();
    let (_state, effects) = update(state, Msg::OpenInBrowserClicked);
    assert_eq!(effects.len(), 1);
    assert!(matches!(
        &effects[0],
        Effect::OpenUrlInBrowser { url } if url == "https://open-browser.example/article"
    ));
}

#[test]
fn open_in_browser_with_unsummarized_job_selected_emits_nothing() {
    init_logging();
    let mut state = AppState::new();
    state.restore_completed_jobs(vec![crate::CompletedJobSnapshot {
        url: "https://no-summary.example".to_string(),
        tokens: None,
        bytes: None,
        links: vec![],
        fetched_utc: None,
    }]);
    let job_id = state.view().jobs.first().map(|j| j.job_id).unwrap_or(1);
    state.select_job(job_id);
    let (_state, effects) = update(state, Msg::OpenInBrowserClicked);
    assert!(effects.is_empty());
}

#[test]
fn open_in_browser_with_no_selection_emits_nothing() {
    init_logging();
    let state = AppState::new();
    let (_state, effects) = update(state, Msg::OpenInBrowserClicked);
    assert!(effects.is_empty());
}

mod archive_tests;
mod briefing_history_tests;
mod entity_index_tests;
mod import_tests;
mod pre_triage_refresh_tests;
mod prompt_lab_tests;
mod signal_candidate_tests;
mod triage_tests;
mod ui_state_tests;
