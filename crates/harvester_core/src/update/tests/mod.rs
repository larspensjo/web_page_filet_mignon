use super::*;
use crate::briefing::{ArticleSummaryState, BriefingPhase, LoadedArticle};
use crate::LlmResultKind;
use harvester_engine::llm::prompt::PromptId;
use harvester_engine::llm::{run_metadata::LlmRunMetadata, DEFAULT_BRIEFING_MODEL};

mod support;
use support::*;

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
    assert_eq!(
        view.briefing_progress.as_deref(),
        Some("Briefing failed: request timed out")
    );
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

#[test]
fn briefing_aggregate_not_dispatched_until_all_articles_settled() {
    init_logging();
    let mut state = AppState::new();
    state.set_summary_max_in_flight(2);
    let state = start_briefing_after_triage(state, loaded_articles().0.clone());
    let (articles, collection_text) = loaded_articles();

    // Load 2 articles with limit=2 → both go in-flight
    let (state, _) = update(
        state,
        Msg::ArticlesLoaded {
            articles,
            collection_text,
        },
    );
    assert_eq!(state.briefing().in_progress_count(), 2);
    assert_eq!(state.briefing().pending_count(), 0);

    // Complete only first article → aggregate should NOT be dispatched yet
    let (state, effects) = update(
        state,
        Msg::LlmCompleted {
            request_id: 3,
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
    let has_aggregate = effects.iter().any(|e| {
        matches!(
            e,
            Effect::RequestLlmCompletion {
                prompt_id: PromptId::AggregateBriefing,
                ..
            }
        )
    });
    assert!(
        !has_aggregate,
        "aggregate must not dispatch while article 2 still in-flight"
    );
    assert_eq!(state.briefing().in_progress_count(), 1);

    // Complete second article → aggregate should now be dispatched
    let (_state, effects) = update(
        state,
        Msg::LlmCompleted {
            request_id: 4,
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
    let has_aggregate = effects.iter().any(|e| {
        matches!(
            e,
            Effect::RequestLlmCompletion {
                prompt_id: PromptId::AggregateBriefing,
                ..
            }
        )
    });
    assert!(
        has_aggregate,
        "aggregate should dispatch after all articles settled"
    );
}

#[test]
fn startup_hydration_emits_load_briefing_history() {
    let state = AppState::new();
    let (_, effects) = update(state, Msg::StartupHydrationRequested);
    assert!(
        effects.contains(&Effect::LoadBriefingHistory),
        "expected LoadBriefingHistory in startup effects, got: {:?}",
        effects
    );
}

#[test]
fn briefing_history_loaded_sets_state() {
    use crate::briefing::BriefingHistoryEntry;
    let state = AppState::new();
    let entry = BriefingHistoryEntry {
        generated_at_utc: "2026-02-21T00:00:00Z".to_string(),
        executive_summary: "Test".to_string(),
        top_stories: vec![],
        article_count: 1,
    };
    let (state, effects) = update(
        state,
        Msg::BriefingHistoryLoaded {
            entries: vec![entry],
        },
    );
    assert_eq!(state.briefing_history().len(), 1);
    assert!(effects.is_empty());
}

// ------------------------------------------------------------------
// Task 11: save history on canonical briefing completion
// ------------------------------------------------------------------

fn run_single_article_briefing_to_completion(state: AppState) -> (AppState, Vec<Effect>) {
    let state = start_briefing_after_triage(state, loaded_single_article().0.clone());
    let (articles, collection_text) = loaded_single_article();
    let (state, _) = update(
        state,
        Msg::ArticlesLoaded {
            articles,
            collection_text,
        },
    );
    // summary completes → fires AggregateBriefing (request_id 3 for fresh state)
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
    // briefing completes
    update(
        state,
        Msg::LlmCompleted {
            request_id: aggregate_request_id,
            result: LlmResultKind::Success {
                output_json: briefing_json(1),
                input_tokens: 20,
                output_tokens: 8,
                prompt_version: 5,
                model_id: "test-model".to_string(),
            },
            metadata: None,
        },
    )
}

#[test]
fn briefing_completion_appends_history_and_emits_save() {
    init_logging();
    let state = AppState::new();
    let (state, effects) = run_single_article_briefing_to_completion(state);
    assert_eq!(
        state.briefing_history().len(),
        1,
        "history should have 1 entry"
    );
    let has_save = effects
        .iter()
        .any(|e| matches!(e, Effect::SaveBriefingHistory { .. }));
    assert!(
        has_save,
        "SaveBriefingHistory effect should be emitted after briefing completion"
    );
}

#[test]
fn prompt_lab_aggregate_completion_does_not_update_history() {
    init_logging();
    use crate::briefing::BriefingHistoryEntry;
    use crate::prompt_lab::PromptLabStage;
    let mut state = AppState::new();
    state.push_briefing_history(BriefingHistoryEntry {
        generated_at_utc: "2026-02-20T10:00:00Z".to_string(),
        executive_summary: "Old summary content.".to_string(),
        top_stories: vec![],
        article_count: 2,
    });
    let history_before = state.briefing_history().to_vec();
    let (state, _) = update(state, Msg::PromptLabOpenRequested);
    let (state, _) = update(
        state,
        Msg::PromptLabStageSelected {
            stage: PromptLabStage::Briefing,
        },
    );
    let mut state = state;
    prepare_type_url_snapshot(&mut state, "article text");
    let (state, effects) = update(state, Msg::PromptLabRunRequested);
    let request_id = request_id_for_prompt(&effects, PromptId::AggregateBriefing)
        .expect("expected prompt-lab aggregate briefing request");
    let (state, completion_effects) = update(
        state,
        Msg::LlmCompleted {
            request_id,
            result: LlmResultKind::Success {
                output_json: briefing_json(1),
                input_tokens: 10,
                output_tokens: 5,
                prompt_version: 1,
                model_id: "test-model".to_string(),
            },
            metadata: Some(LlmRunMetadata::stub()),
        },
    );
    assert!(
        completion_effects
            .iter()
            .all(|e| !matches!(e, Effect::SaveBriefingHistory { .. })),
        "Prompt Lab completion must not emit SaveBriefingHistory"
    );
    assert_eq!(
        state.briefing_history(),
        history_before.as_slice(),
        "Prompt Lab runs must not mutate briefing history"
    );
}

#[test]
fn prompt_lab_aggregate_request_includes_previous_briefings_extra_var() {
    init_logging();
    use crate::briefing::BriefingHistoryEntry;
    use crate::prompt_lab::PromptLabStage;
    let mut state = AppState::new();
    state.push_briefing_history(BriefingHistoryEntry {
        generated_at_utc: "2026-02-20T10:00:00Z".to_string(),
        executive_summary: "Old summary content.".to_string(),
        top_stories: vec![],
        article_count: 2,
    });
    let (state, _) = update(state, Msg::PromptLabOpenRequested);
    let (state, _) = update(
        state,
        Msg::PromptLabStageSelected {
            stage: PromptLabStage::Briefing,
        },
    );
    let mut state = state;
    prepare_type_url_snapshot(&mut state, "article text");
    let (_state, effects) = update(state, Msg::PromptLabRunRequested);
    match effects.into_iter().find(|effect| {
        matches!(
            effect,
            Effect::RequestLlmCompletion {
                prompt_id: PromptId::AggregateBriefing,
                ..
            }
        )
    }) {
        Some(Effect::RequestLlmCompletion {
            extra_template_vars,
            ..
        }) => {
            let previous = extra_template_vars
                .iter()
                .find(|(key, _)| key == "previous_briefings");
            assert!(
                previous.is_some(),
                "missing previous_briefings in Prompt Lab aggregate request"
            );
            let (_, value) = previous.unwrap();
            assert!(
                value.contains("Old summary content."),
                "previous_briefings should contain history snapshot: {value}"
            );

            let window = extra_template_vars
                .iter()
                .find(|(key, _)| key == "briefing_time_window");
            assert!(
                window.is_some(),
                "missing briefing_time_window in Prompt Lab aggregate request"
            );
        }
        _ => panic!("no Prompt Lab AggregateBriefing RequestLlmCompletion effect emitted"),
    }
}

// ------------------------------------------------------------------
// Task 10: inject previous_briefings into aggregate briefing request
// ------------------------------------------------------------------

#[test]
fn format_block_contains_history_content() {
    use crate::briefing::{format_previous_briefings_block, BriefingHistoryEntry};
    let mut state = AppState::new();
    state.push_briefing_history(BriefingHistoryEntry {
        generated_at_utc: "2026-02-20T10:00:00Z".to_string(),
        executive_summary: "Old summary content.".to_string(),
        top_stories: vec![],
        article_count: 2,
    });
    let block = format_previous_briefings_block(state.briefing_history());
    assert!(block.contains("Old summary content."));
    assert!(block.contains("2026-02-20T10:00:00Z"));
}

#[test]
fn aggregate_briefing_effect_includes_previous_briefings_extra_var() {
    init_logging();
    use crate::briefing::BriefingHistoryEntry;
    let mut state = AppState::new();
    // Pre-load a history entry so it shows up in the extra_template_vars
    state.push_briefing_history(BriefingHistoryEntry {
        generated_at_utc: "2026-02-20T10:00:00Z".to_string(),
        executive_summary: "Old summary content.".to_string(),
        top_stories: vec![],
        article_count: 2,
    });
    let state = start_briefing_after_triage(state, loaded_single_article().0.clone());
    let (articles, collection_text) = loaded_single_article();
    let (state, _) = update(
        state,
        Msg::ArticlesLoaded {
            articles,
            collection_text,
        },
    );
    // Single article summary completes → aggregate briefing effect fires
    let (_state, effects) = update(
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
    let completion_effect = effects.iter().find(|e| {
        matches!(
            e,
            Effect::RequestLlmCompletion {
                prompt_id: PromptId::AggregateBriefing,
                ..
            }
        )
    });
    match completion_effect {
        Some(Effect::RequestLlmCompletion {
            extra_template_vars,
            ..
        }) => {
            let pb = extra_template_vars
                .iter()
                .find(|(k, _)| k == "previous_briefings");
            assert!(
                pb.is_some(),
                "missing previous_briefings in extra_template_vars"
            );
            let (_, value) = pb.unwrap();
            assert!(
                value.contains("Old summary content."),
                "previous_briefings value should contain history: {value}"
            );
            let window = extra_template_vars
                .iter()
                .find(|(k, _)| k == "briefing_time_window");
            assert!(
                window.is_some(),
                "missing briefing_time_window in extra_template_vars"
            );
            let (_, window_value) = window.unwrap();
            assert!(
                window_value.contains("All available articles"),
                "briefing_time_window should describe all-time coverage: {window_value}"
            );
        }
        _ => panic!("no AggregateBriefing RequestLlmCompletion effect emitted"),
    }
}

#[test]
fn aggregate_briefing_effect_includes_checkpoint_time_window_extra_var() {
    init_logging();
    let mut state = AppState::new();
    let since = chrono::DateTime::parse_from_rfc3339("2026-02-24T12:34:56Z")
        .unwrap()
        .with_timezone(&chrono::Utc);
    state.set_briefing_since_utc(Some(since));
    let state = start_briefing_after_triage(state, loaded_single_article().0.clone());
    let (articles, collection_text) = loaded_single_article();
    let (state, _) = update(
        state,
        Msg::ArticlesLoaded {
            articles,
            collection_text,
        },
    );
    let (_state, effects) = update(
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

    let completion_effect = effects.iter().find(|e| {
        matches!(
            e,
            Effect::RequestLlmCompletion {
                prompt_id: PromptId::AggregateBriefing,
                ..
            }
        )
    });
    match completion_effect {
        Some(Effect::RequestLlmCompletion {
            extra_template_vars,
            ..
        }) => {
            let window = extra_template_vars
                .iter()
                .find(|(k, _)| k == "briefing_time_window")
                .expect("briefing_time_window extra var");
            assert!(
                window.1.contains("2026-02-24T12:34:56Z"),
                "window label should include checkpoint timestamp: {}",
                window.1
            );
        }
        _ => panic!("no AggregateBriefing RequestLlmCompletion effect emitted"),
    }
}

// ── Entity index / trends reducer tests ──────────────────────────────────

fn make_entity_index_with_company(
    url: &str,
    company: &str,
    fetched_utc: &str,
) -> crate::entity_index::EntityIndex {
    use crate::entity_index::{EntityIndex, EntityIndexEntry};
    use std::collections::BTreeMap;
    let mut entries = BTreeMap::new();
    entries.insert(
        url.to_string(),
        EntityIndexEntry {
            fetched_utc: Some(fetched_utc.to_string()),
            content_hash: Some("abc123".to_string()),
            companies: vec![company.to_string()],
            ..EntityIndexEntry::default()
        },
    );
    EntityIndex {
        schema_version: 1,
        entries,
    }
}

#[test]
fn entity_index_loaded_populates_trend_data() {
    init_logging();
    let state = AppState::default();
    let index =
        make_entity_index_with_company("https://example.com/a", "Nvidia", "2026-02-01T00:00:00Z");
    let (state, effects) = update(state, Msg::EntityIndexLoaded { index });
    assert!(effects.is_empty());
    let trend_data = state
        .entity_trend_data()
        .expect("entity_trend_data should be set");
    assert_eq!(
        trend_data.companies.weeks.len(),
        13,
        "should have 13 week buckets"
    );
}

#[test]
fn entity_index_load_failed_triggers_rebuild() {
    init_logging();
    let state = AppState::default();
    let (_, effects) = update(
        state,
        Msg::EntityIndexLoadFailed {
            reason: "file not found".to_string(),
        },
    );
    assert!(
        effects
            .iter()
            .any(|e| matches!(e, Effect::RebuildEntityIndex)),
        "EntityIndexLoadFailed should emit Effect::RebuildEntityIndex; got: {effects:?}"
    );
}

#[test]
fn trend_category_selected_updates_active_category_no_effects() {
    init_logging();
    let state = AppState::default();
    assert_eq!(
        state.active_trend_category(),
        crate::tabs::TrendCategory::Companies
    );
    let (state, effects) = update(
        state,
        Msg::TrendCategorySelected {
            category: crate::tabs::TrendCategory::Technologies,
        },
    );
    assert!(
        effects.is_empty(),
        "TrendCategorySelected should emit no effects"
    );
    assert_eq!(
        state.active_trend_category(),
        crate::tabs::TrendCategory::Technologies
    );
}

// ── LeftTab / JobListScope reducer tests ─────────────────────────────────

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
    // Already SinceCheckpoint; setting SinceCheckpoint again should not mark dirty.
    let view_before = state.view();
    let (state, _) = update(
        state,
        Msg::JobListScopeSet {
            scope: JobListScope::SinceCheckpoint,
        },
    );
    // dirty starts false; setting same scope should leave it false
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

mod archive_tests;
mod import_tests;
mod pre_triage_refresh_tests;
mod prompt_lab_tests;
mod triage_tests;
