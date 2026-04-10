use std::collections::HashMap;
use std::sync::Once;

use super::summary_cache_support::summary_cache_model_ids_compatible;
use super::*;
use crate::briefing::{ArticleSummaryState, BriefingPhase, LoadedArticle};
use crate::LlmResultKind;
use harvester_engine::llm::prompt::PromptId;
use harvester_engine::llm::{
    run_metadata::{CacheStatus, LlmRunMetadata, LlmRunMetadataInit},
    DEFAULT_BRIEFING_MODEL, OPENAI_MODEL_GPT_4O, OPENAI_MODEL_GPT_4O_MINI,
};

fn init_logging() {
    static INIT: Once = Once::new();
    INIT.call_once(engine_logging::initialize_for_tests);
}

fn loaded_articles() -> (Vec<LoadedArticle>, String) {
    fn long_text(prefix: &str) -> String {
        format!(
            "{prefix} {}",
            std::iter::repeat_n("content", 220)
                .collect::<Vec<_>>()
                .join(" ")
        )
    }
    let articles = vec![
        LoadedArticle {
            url: "https://example.com/a".to_string(),
            source_title: Some("Article A".to_string()),
            prepared_text: long_text("Article A text"),
            content_hash: "hash-a".to_string(),
            fetched_utc: None,
        },
        LoadedArticle {
            url: "https://example.com/b".to_string(),
            source_title: Some("Article B".to_string()),
            prepared_text: long_text("Article B text"),
            content_hash: "hash-b".to_string(),
            fetched_utc: None,
        },
    ];
    (articles, "Collection text".to_string())
}

fn loaded_single_article() -> (Vec<LoadedArticle>, String) {
    let articles = vec![LoadedArticle {
        url: "https://example.com/a".to_string(),
        source_title: Some("Article A".to_string()),
        prepared_text: format!(
            "Article A text {}",
            std::iter::repeat_n("content", 220)
                .collect::<Vec<_>>()
                .join(" ")
        ),
        content_hash: "hash-a".to_string(),
        fetched_utc: None,
    }];
    (articles, "Collection text".to_string())
}

fn with_summary_metadata(state: AppState) -> AppState {
    let mut active_versions = HashMap::new();
    active_versions.insert(PromptId::ArticleTriage, 1);
    active_versions.insert(PromptId::ArticleSummary, 1);
    let mut effective_models = HashMap::new();
    effective_models.insert(PromptId::ArticleTriage, "test-triage-model".to_string());
    effective_models.insert(PromptId::ArticleSummary, "test-model".to_string());
    let (state, _) = update(
        state,
        Msg::LlmMetadataLoaded {
            active_versions,
            effective_models,
            templates: HashMap::new(),
        },
    );
    state
}

fn summary_json(title: &str) -> String {
    format!("{{\"title\":\"{title}\",\"summary\":\"Summary\",\"key_points\":[\"p1\"]}}")
}

fn briefing_json(article_count: u32) -> String {
    format!(
        "{{\"executive_summary\":\"Exec\",\"top_stories\":[{{\"headline\":\"Story\",\"body\":\"Desc\"}}],\"article_count\":{article_count}}}"
    )
}

fn aggregate_briefing_metadata(
    model: &str,
    input_tokens: u32,
    output_tokens: u32,
) -> LlmRunMetadata {
    LlmRunMetadata::new(LlmRunMetadataInit {
        prompt_id: PromptId::AggregateBriefing,
        prompt_version: 1,
        resolved_model: model.to_string(),
        input_bytes: 100,
        input_tokens,
        output_tokens,
        cached_input_tokens: 0,
        cost_microdollars: 0,
        wall_ms: 10,
        parse_ok: true,
        validation_error: None,
        cache_status: CacheStatus::Miss,
        timestamp_utc: "2026-01-01T00:00:00Z".to_string(),
    })
}

fn request_id_for_prompt(effects: &[Effect], prompt_id: PromptId) -> Option<u64> {
    effects.iter().find_map(|effect| match effect {
        Effect::RequestLlmCompletion {
            request_id,
            prompt_id: pid,
            ..
        } if *pid == prompt_id => Some(*request_id),
        _ => None,
    })
}

fn start_briefing_after_triage(state: AppState, articles: Vec<LoadedArticle>) -> AppState {
    let (state, _) = update(state, Msg::GenerateBriefingClicked);
    let state = with_summary_metadata(state);
    let (mut state, effects) = update(
        state,
        Msg::BriefingPrereqArticlesLoaded {
            articles: articles.clone(),
        },
    );
    let mut triage_request_id =
        request_id_for_prompt(&effects, PromptId::ArticleTriage).expect("triage request");

    for idx in 0..articles.len() {
        let priority = (articles.len() - idx + 1) as u8;
        let (next_state, next_effects) = update(
            state,
            Msg::LlmCompleted {
                request_id: triage_request_id,
                result: LlmResultKind::Success {
                    output_json: format!(
                        "{{\"category\":\"security\",\"priority\":{},\"tags\":[\"tag\"],\"rationale\":\"reason\"}}",
                        priority
                    ),
                    input_tokens: 10,
                    output_tokens: 5,
                    prompt_version: 1,
                    model_id: "test-model".to_string(),
                },
                metadata: None,
            },
        );
        state = next_state;
        if idx + 1 < articles.len() {
            triage_request_id = request_id_for_prompt(&next_effects, PromptId::ArticleTriage)
                .expect("next triage request");
        } else {
            assert!(
                next_effects
                    .iter()
                    .any(|effect| matches!(effect, Effect::LoadArticlesForBriefing { .. })),
                "final triage completion should trigger filtered briefing load"
            );
        }
    }
    state
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

fn make_state_with_summarized_job_for_update() -> AppState {
    use crate::briefing::{ArticleSummaryResult, LoadedArticle};
    let mut state = AppState::new();
    let url = "https://open-browser.example/article".to_string();
    state.restore_completed_jobs(vec![crate::CompletedJobSnapshot {
        url: url.clone(),
        tokens: None,
        bytes: None,
        links: vec![],
        fetched_utc: None,
    }]);
    // Set up briefing with completed summary for this URL
    let mut briefing = crate::briefing::BriefingSession::new_loading(None);
    briefing.set_articles(
        vec![LoadedArticle {
            url: url.clone(),
            source_title: None,
            prepared_text: "text".to_string(),
            content_hash: "hash".to_string(),
            fetched_utc: None,
        }],
        "collection".to_string(),
    );
    briefing.transition_to_summarizing();
    briefing.start_article(0, 1);
    briefing.complete_article(
        0,
        ArticleSummaryResult {
            title: "Article".to_string(),
            summary: "Summary".to_string(),
            key_points: vec![],
            input_tokens: 10,
            output_tokens: 5,
            entities: Default::default(),
        },
    );
    state.set_briefing(briefing);
    // Select the job
    let job_id = state.view().jobs.first().map(|j| j.job_id).unwrap_or(1);
    state.select_job(job_id);
    state
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

// ── Triage reducer integration tests ─────────────────────────────────────

fn loaded_triage_articles(count: usize) -> Vec<LoadedArticle> {
    (0..count)
        .map(|i| LoadedArticle {
            url: format!("https://example.com/{i}"),
            source_title: None,
            prepared_text: std::iter::repeat_n(format!("article-{i}-content"), 220)
                .collect::<Vec<_>>()
                .join(" "),
            content_hash: format!("hash-{i}"),
            fetched_utc: None,
        })
        .collect()
}

fn triage_json() -> String {
    r#"{"category":"news","priority":3,"tags":["tag"],"rationale":"ok"}"#.to_string()
}

fn triage_success(request_id: u64) -> Msg {
    Msg::LlmCompleted {
        request_id,
        result: LlmResultKind::Success {
            output_json: triage_json(),
            input_tokens: 10,
            output_tokens: 5,
            prompt_version: 1,
            model_id: "test-model".to_string(),
        },
        metadata: None,
    }
}

fn start_triage_for_test(state: AppState, articles: Vec<LoadedArticle>) -> (AppState, Vec<Effect>) {
    let mut active_versions = std::collections::HashMap::new();
    active_versions.insert(PromptId::ArticleTriage, 1);
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
    let (state, _) = update(
        state,
        Msg::PromptContextsLoaded {
            contexts: std::collections::HashMap::new(),
        },
    );
    // Bypass the coordinator quiet window: directly set up the in-flight state
    // so TriageArticlesLoaded is applied immediately. These tests focus on the
    // triage LLM dispatch behavior, not the pre-triage scheduling policy.
    let mut state = state;
    let triage_request_id = state.alloc_triage_request_id();
    state.set_triage_in_flight(triage_request_id);
    let (state, _) = update(
        state,
        Msg::TriageArticlesLoaded {
            request_id: triage_request_id,
            articles,
        },
    );
    update(state, Msg::TriageClicked)
}

#[test]
fn triage_clicked_emits_load_effects() {
    init_logging();
    let state = AppState::new();
    let (_state, effects) = update(state, Msg::TriageClicked);
    assert!(effects.is_empty());
}

#[test]
fn summary_cache_model_id_compatibility_accepts_resolved_suffix() {
    assert!(summary_cache_model_ids_compatible(
        OPENAI_MODEL_GPT_4O_MINI,
        "gpt-4o-mini-2024-07-18",
    ));
    assert!(!summary_cache_model_ids_compatible(
        OPENAI_MODEL_GPT_4O_MINI,
        OPENAI_MODEL_GPT_4O,
    ));
}

#[test]
fn briefing_blocked_when_triage_in_progress() {
    init_logging();
    let mut state = AppState::new();
    state.set_triage(crate::triage::TriageSession::new_loading(None));
    let (next_state, effects) = update(state.clone(), Msg::GenerateBriefingClicked);
    assert!(effects.is_empty());
    assert_eq!(next_state.briefing().phase(), state.briefing().phase());
}

#[test]
fn triage_click_blocked_when_briefing_owns_triage() {
    init_logging();
    let state = AppState::new();
    let (state, _) = update(state, Msg::GenerateBriefingClicked);
    let (_state, effects) = update(state, Msg::TriageClicked);
    assert!(effects.is_empty());
}

#[test]
fn triage_articles_loaded_dispatches_up_to_limit_requests() {
    init_logging();
    let mut state = AppState::new();
    state.set_triage_max_in_flight(2);
    let (state, effects) = start_triage_for_test(state, loaded_triage_articles(3));
    let llm_effects: Vec<_> = effects
        .iter()
        .filter(|e| matches!(e, Effect::RequestLlmCompletion { .. }))
        .collect();
    assert_eq!(
        llm_effects.len(),
        2,
        "should dispatch 2 requests for limit=2"
    );
    assert_eq!(state.triage().in_progress_count(), 2);
    assert_eq!(state.triage().pending_count(), 1);
}

#[test]
fn triage_completion_backfills_one_slot() {
    init_logging();
    let mut state = AppState::new();
    state.set_triage_max_in_flight(2);
    let (state, _) = start_triage_for_test(state, loaded_triage_articles(3));
    assert_eq!(state.triage().in_progress_count(), 2);

    // Complete request_id=1 → should backfill 1 more slot
    let (state, effects) = update(state, triage_success(1));
    let llm_effects: Vec<_> = effects
        .iter()
        .filter(|e| matches!(e, Effect::RequestLlmCompletion { .. }))
        .collect();
    assert_eq!(llm_effects.len(), 1, "backfill dispatches 1 new request");
    assert_eq!(state.triage().in_progress_count(), 2);
    assert_eq!(state.triage().completed_count(), 1);
}

#[test]
fn triage_out_of_order_completion_routes_correctly() {
    init_logging();
    let mut state = AppState::new();
    state.set_triage_max_in_flight(3);
    let (state, _) = start_triage_for_test(state, loaded_triage_articles(3));
    assert_eq!(state.triage().in_progress_count(), 3);

    // Complete in reverse order: 3, then 1, then 2
    let (state, _) = update(state, triage_success(3));
    let (state, _) = update(state, triage_success(1));
    let (state, _) = update(state, triage_success(2));

    assert_eq!(state.triage().completed_count(), 3);
    assert_eq!(state.triage().failed_count(), 0);
    assert!(matches!(
        state.triage().phase(),
        crate::triage::TriagePhase::Complete
    ));
}

#[test]
fn triage_progress_text_counts_settled_articles() {
    init_logging();
    let mut state = AppState::new();
    state.set_triage_max_in_flight(1);
    let (state, _) = start_triage_for_test(state, loaded_triage_articles(3));
    let text = state.triage().progress_text().unwrap();
    assert!(
        text.contains("0/3"),
        "initial progress shows 0 settled: got '{text}'"
    );

    let (state, _) = update(state, triage_success(1));
    let text = state.triage().progress_text().unwrap();
    assert!(
        text.contains("1/3"),
        "after 1 complete shows 1 settled: got '{text}'"
    );
}

#[test]
fn triage_quota_exhausted_fails_all_pending() {
    init_logging();
    let mut state = AppState::new();
    state.set_triage_max_in_flight(1);
    let (state, _) = start_triage_for_test(state, loaded_triage_articles(3));

    // Quota exhausted on request_id=1 → all pending should fail
    let (state, _) = update(
        state,
        Msg::LlmCompleted {
            request_id: 1,
            result: LlmResultKind::QuotaExhausted {
                reason: "too many calls".to_string(),
            },
            metadata: None,
        },
    );
    assert_eq!(state.triage().failed_count(), 3); // 1 from quota + 2 pending
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

fn prepare_type_url_snapshot(state: &mut AppState, snapshot: &str) {
    state
        .prompt_lab_mut()
        .select_input_source(crate::prompt_lab::PromptLabInputSource::TypeUrl);
    state
        .prompt_lab_mut()
        .set_url_input("https://example.com".to_string());
    let resolve_id = state.allocate_next_prompt_lab_resolve_id();
    state.prompt_lab_mut().begin_url_resolution(resolve_id);
    state
        .prompt_lab_mut()
        .finish_url_resolution(resolve_id, Ok(snapshot.to_string()));
}

fn dispatch_lab_run(state: AppState) -> (AppState, u64) {
    let mut state = state;
    prepare_type_url_snapshot(&mut state, "article content");
    let (state, effects) = update(state, Msg::PromptLabRunRequested);
    let request_id = effects
        .iter()
        .find_map(|e| {
            if let Effect::RequestLlmCompletion { request_id, .. } = e {
                Some(*request_id)
            } else {
                None
            }
        })
        .expect("expected RequestLlmCompletion effect");
    (state, request_id)
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

#[test]
fn pre_triage_refresh_dispatch_includes_briefing_checkpoint_since_utc() {
    init_logging();
    let since = chrono::DateTime::parse_from_rfc3339("2026-03-21T18:17:00Z")
        .unwrap()
        .with_timezone(&chrono::Utc);
    let mut state = add_completed_job_for_test(AppState::new(), "https://example.com/1");
    state.set_briefing_since_utc(Some(since));

    for _ in 0..200 {
        let (next, effects) = update(state, Msg::Tick);
        state = next;
        if let Some(effect_since_utc) = effects.iter().find_map(|e| match e {
            Effect::LoadArticlesForTriage { since_utc, .. } => *since_utc,
            _ => None,
        }) {
            assert_eq!(effect_since_utc, since);
            return;
        }
    }

    panic!("no LoadArticlesForTriage dispatch within 200 ticks");
}

#[test]
fn archive_clicked_emits_open_dialog_with_request_id_and_article_count() {
    init_logging();
    let state = add_completed_job_for_test(AppState::new(), "https://example.com/1");
    let (state, effects) = update(state, Msg::ArchiveClicked);
    assert_eq!(state.archive_request_id(), 1);
    let effect = effects
        .into_iter()
        .find(|effect| matches!(effect, Effect::OpenArchiveDialog { .. }))
        .expect("OpenArchiveDialog effect expected");
    match effect {
        Effect::OpenArchiveDialog {
            request_id,
            article_count,
            since_utc,
            default_basename,
            ..
        } => {
            assert_eq!(request_id, 1);
            assert_eq!(article_count, 0);
            assert!(since_utc.is_none());
            assert_eq!(default_basename, "archive.md");
        }
        _ => unreachable!(),
    }
}

#[test]
fn archive_clicked_with_triage_complete_and_pre_triage_ready_sets_pending_count() {
    init_logging();
    // Build state: TriageComplete (1 article) + PreTriageReady (1 new article).
    let state = complete_triage_state_for_test(1);
    let url = "https://pending.com/1";
    let state = add_completed_job_for_test(state, url);
    let (state, request_id) = tick_until_dispatch(state);
    let (state, _) = update(
        state,
        Msg::TriageArticlesLoaded {
            request_id,
            articles: loaded_pre_triage_articles(&[url]),
        },
    );
    // Sanity: live working corpus is PreTriageReady (pre-triage takes precedence).
    assert_eq!(
        state.current_working_corpus().source(),
        crate::working_corpus::CurrentWorkingCorpusSource::PreTriageReady,
        "working corpus should be PreTriageReady — archive corpus is different"
    );

    let (_, effects) = update(state, Msg::ArchiveClicked);
    let (article_count, pending_count) = effects
        .iter()
        .find_map(|e| {
            if let Effect::OpenArchiveDialog {
                article_count,
                pending_pre_triage_count,
                ..
            } = e
            {
                Some((*article_count, *pending_pre_triage_count))
            } else {
                None
            }
        })
        .expect("expected OpenArchiveDialog effect");
    assert_eq!(
        article_count, 1,
        "archive must use TriageComplete (1 article), not pre-triage"
    );
    assert!(
        pending_count > 0,
        "pending_pre_triage_count must be > 0, got {}",
        pending_count
    );
}

#[test]
fn archive_clicked_with_only_pre_triage_ready_has_zero_article_count() {
    init_logging();
    // No triage done — only pre-triage ready. Archive must show 0, Export disabled.
    let state = ready_pre_triage_state(&["https://example.com/a", "https://example.com/b"]);
    let (_, effects) = update(state, Msg::ArchiveClicked);
    let (article_count, pending_count) = effects
        .iter()
        .find_map(|e| {
            if let Effect::OpenArchiveDialog {
                article_count,
                pending_pre_triage_count,
                ..
            } = e
            {
                Some((*article_count, *pending_pre_triage_count))
            } else {
                None
            }
        })
        .expect("expected OpenArchiveDialog effect");
    assert_eq!(
        article_count, 0,
        "no triage done → archive article_count must be 0"
    );
    assert_eq!(
        pending_count, 2,
        "both pre-triage articles must appear in pending count"
    );
}

#[test]
fn archive_clicked_with_triage_complete_and_no_pre_triage_has_zero_pending_count() {
    init_logging();
    // Triage done, pre-triage idle — no pending articles, no warning.
    let state = complete_triage_state_for_test(2);
    let (_, effects) = update(state, Msg::ArchiveClicked);
    let (article_count, pending_count) = effects
        .iter()
        .find_map(|e| {
            if let Effect::OpenArchiveDialog {
                article_count,
                pending_pre_triage_count,
                ..
            } = e
            {
                Some((*article_count, *pending_pre_triage_count))
            } else {
                None
            }
        })
        .expect("expected OpenArchiveDialog effect");
    assert_eq!(
        article_count, 2,
        "TriageComplete corpus must supply 2 articles"
    );
    assert_eq!(
        pending_count, 0,
        "no pre-triage ready → pending count must be 0"
    );
}

/// Helper: prime LLM metadata so `TriageClicked` dispatches immediately
/// (skipping the `LoadPromptContexts`/`LoadLlmMetadata` round-trip).
fn prime_llm_metadata(state: AppState) -> AppState {
    let mut active_versions = std::collections::HashMap::new();
    active_versions.insert(PromptId::ArticleTriage, 1);
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
    let (state, _) = update(
        state,
        Msg::PromptContextsLoaded {
            contexts: std::collections::HashMap::new(),
        },
    );
    state
}

/// Helper: complete all pending LLM triage requests in `effects`.
/// Returns the state after all completions.
fn complete_all_triage_llm_requests(mut state: AppState, effects: Vec<Effect>) -> AppState {
    let request_ids: Vec<u64> = effects
        .iter()
        .filter_map(|e| match e {
            Effect::RequestLlmCompletion { request_id, .. } => Some(*request_id),
            _ => None,
        })
        .collect();
    for rid in request_ids {
        let (next, _) = update(state, triage_success(rid));
        state = next;
    }
    state
}

// ── consume_interactive_pre_triage_articles_for_triage unit tests ────────

#[test]
fn consume_interactive_pre_triage_articles_for_triage_rejects_non_interactive_phase() {
    init_logging();
    // Pre-triage is Idle (default) — consume must return None.
    let mut state = AppState::new();
    let result = state.consume_interactive_pre_triage_articles_for_triage();
    assert!(result.is_none(), "Idle phase must return None");
    assert!(
        matches!(
            state.pre_triage().phase(),
            crate::pre_triage_filter::PreTriagePhase::Idle
        ),
        "phase must remain Idle after failed consume"
    );
}

#[test]
fn consume_interactive_pre_triage_articles_for_triage_returns_articles_and_resets_to_idle() {
    init_logging();
    // Setup: ReadyToTriage state with articles
    let urls = &["https://example.com/a", "https://example.com/b"];
    let mut state = ready_pre_triage_state(urls);
    assert!(matches!(
        state.pre_triage().phase(),
        crate::pre_triage_filter::PreTriagePhase::ReadyToTriage
    ));

    // Action: consume articles
    let articles = state.consume_interactive_pre_triage_articles_for_triage();

    // Assert: returns Some with the articles
    let articles = articles.expect("should return Some in ReadyToTriage with articles");
    assert_eq!(
        articles.len(),
        urls.len(),
        "should return all resolved articles"
    );

    // Assert: pre-triage is now Idle
    assert!(
        matches!(
            state.pre_triage().phase(),
            crate::pre_triage_filter::PreTriagePhase::Idle
        ),
        "phase must be Idle after consuming"
    );

    // Assert: resolved URLs are now empty
    assert!(
        state.pre_triage().resolved_included_urls().is_empty(),
        "no URLs should remain in pre-triage after consuming"
    );
}

#[test]
fn triage_clicked_consumes_reviewing_pre_triage_into_triage_session() {
    init_logging();
    let review_content: String = std::iter::repeat_n("longword", 100)
        .collect::<Vec<_>>()
        .join(" ");
    let url1 = "https://review-handoff.com/1";
    let url2 = "https://review-handoff.com/2";
    let state = add_completed_job_for_test(AppState::new(), url1);
    let state = add_completed_job_for_test(state, url2);
    let (state, request_id) = tick_until_dispatch(state);
    let articles = vec![
        LoadedArticle {
            url: url1.to_string(),
            source_title: None,
            prepared_text: review_content.clone(),
            content_hash: format!("hash-{url1}"),
            fetched_utc: None,
        },
        LoadedArticle {
            url: url2.to_string(),
            source_title: None,
            prepared_text: review_content,
            content_hash: format!("hash-{url2}"),
            fetched_utc: None,
        },
    ];
    let (state, _) = update(
        state,
        Msg::TriageArticlesLoaded {
            request_id,
            articles,
        },
    );
    let key = state.pre_triage().entries()[0].key.clone();
    let (state, _) = update(
        state,
        Msg::PreTriageDecisionSet {
            key,
            decision: crate::pre_triage_filter::ManualDecision::Exclude,
        },
    );
    assert!(
        matches!(
            state.pre_triage().phase(),
            crate::pre_triage_filter::PreTriagePhase::Reviewing
        ),
        "one unresolved review item should keep pre-triage in Reviewing"
    );
    assert!(
        state.view().triage_can_start,
        "Reviewing phase with tentative included articles must allow triage start"
    );

    let state = prime_llm_metadata(state);
    let (state, effects) = update(state, Msg::TriageClicked);

    assert!(!effects.is_empty(), "triage should dispatch from Reviewing");
    assert!(
        matches!(
            state.pre_triage().phase(),
            crate::pre_triage_filter::PreTriagePhase::Idle
        ),
        "pre-triage must reset to Idle after TriageClicked"
    );
    assert_eq!(
        state.triage().articles().len(),
        1,
        "only the tentatively included article should be handed to triage"
    );
    assert_eq!(state.triage().articles()[0].url, url2);
}

// ── consume handoff integration tests ─────────────────────────────────────

#[test]
fn triage_clicked_consumes_ready_pre_triage_into_triage_session() {
    init_logging();
    let urls = &["https://handoff.com/1", "https://handoff.com/2"];
    let state = ready_pre_triage_state(urls);
    let state = prime_llm_metadata(state);

    // Sanity: pre-triage is ReadyToTriage before click.
    assert!(matches!(
        state.pre_triage().phase(),
        crate::pre_triage_filter::PreTriagePhase::ReadyToTriage
    ));

    // Effects (LLM requests) are not the focus of this test — state transitions are.
    let (state, _effects) = update(state, Msg::TriageClicked);

    // After click: pre-triage must be Idle and have no resolved URLs.
    assert!(
        matches!(
            state.pre_triage().phase(),
            crate::pre_triage_filter::PreTriagePhase::Idle
        ),
        "pre-triage must reset to Idle after TriageClicked"
    );
    assert!(
        state.pre_triage().resolved_included_urls().is_empty(),
        "pre-triage must have no resolved URLs after consume"
    );

    // Triage session must have the articles that were in pre-triage.
    assert_eq!(
        state.triage().articles().len(),
        urls.len(),
        "triage session must hold all pre-triage articles"
    );
    let triage_urls: Vec<&str> = state
        .triage()
        .articles()
        .iter()
        .map(|a| a.url.as_str())
        .collect();
    for url in urls {
        assert!(
            triage_urls.contains(url),
            "triage session must contain pre-triage URL {url}"
        );
    }
}

#[test]
fn triage_clicked_sets_current_working_corpus_to_unavailable_until_triage_completes() {
    init_logging();
    let urls = &["https://corpus-src.com/1"];
    let state = ready_pre_triage_state(urls);
    let state = prime_llm_metadata(state);

    // Before click: corpus source is PreTriageReady.
    assert_eq!(
        state.current_working_corpus().source(),
        crate::working_corpus::CurrentWorkingCorpusSource::PreTriageReady,
        "source must be PreTriageReady before TriageClicked"
    );

    // After click: pre-triage is Idle and triage is in-flight → Unavailable.
    let (state, effects) = update(state, Msg::TriageClicked);
    assert_eq!(
        state.current_working_corpus().source(),
        crate::working_corpus::CurrentWorkingCorpusSource::Unavailable,
        "source must be Unavailable while triage is in-flight"
    );

    // Complete triage.
    let state = complete_all_triage_llm_requests(state, effects);

    assert_eq!(
        state.current_working_corpus().source(),
        crate::working_corpus::CurrentWorkingCorpusSource::TriageComplete,
        "source must be TriageComplete after triage finishes"
    );
}

#[test]
fn archive_clicked_after_triage_start_has_zero_pending_pre_triage_count() {
    init_logging();
    // Drive the real reducer handoff path:
    //   pre-triage ReadyToTriage → TriageClicked (consume) → triage complete → ArchiveClicked.
    let urls = &["https://archive-handoff.com/1"];
    let state = ready_pre_triage_state(urls);
    let state = prime_llm_metadata(state);

    let (state, effects) = update(state, Msg::TriageClicked);

    // Complete triage.
    let state = complete_all_triage_llm_requests(state, effects);

    let (_, archive_effects) = update(state, Msg::ArchiveClicked);
    let pending_count = archive_effects
        .iter()
        .find_map(|e| {
            if let Effect::OpenArchiveDialog {
                pending_pre_triage_count,
                ..
            } = e
            {
                Some(*pending_pre_triage_count)
            } else {
                None
            }
        })
        .expect("expected OpenArchiveDialog effect");

    assert_eq!(
        pending_count, 0,
        "pending_pre_triage_count must be 0 after reducer handoff path"
    );
}

#[test]
fn pre_triage_refresh_after_triage_start_repopulates_pre_triage_without_mutating_active_triage() {
    init_logging();
    let urls = &["https://repopulate.com/1"];
    let state = ready_pre_triage_state(urls);
    let state = prime_llm_metadata(state);

    // Start triage — pre-triage becomes Idle.
    let (state, _effects) = update(state, Msg::TriageClicked);
    assert!(
        matches!(
            state.pre_triage().phase(),
            crate::pre_triage_filter::PreTriagePhase::Idle
        ),
        "pre-triage must be Idle after TriageClicked"
    );

    // Snapshot triage session state before the new pre-triage refresh.
    let triage_article_count_before = state.triage().articles().len();
    let triage_phase_before = state.triage().phase().clone();

    // Simulate a new pre-triage refresh arriving (new articles from another poll).
    let new_url = "https://repopulate.com/new-article";
    let state = add_completed_job_for_test(state, new_url);
    let (state, request_id) = tick_until_dispatch(state);
    let new_articles = loaded_pre_triage_articles(&[new_url]);
    let (state, _) = update(
        state,
        Msg::TriageArticlesLoaded {
            request_id,
            articles: new_articles,
        },
    );

    // Triage session must be unchanged.
    assert_eq!(
        state.triage().articles().len(),
        triage_article_count_before,
        "triage session article count must not change after pre-triage refresh"
    );
    assert_eq!(
        state.triage().phase(),
        &triage_phase_before,
        "triage session phase must not change after pre-triage refresh"
    );

    // Pre-triage must have the new article.
    assert!(
        matches!(
            state.pre_triage().phase(),
            crate::pre_triage_filter::PreTriagePhase::ReadyToTriage
        ),
        "pre-triage must be ReadyToTriage after new refresh"
    );
    assert!(
        state
            .pre_triage()
            .resolved_included_urls()
            .iter()
            .any(|u| u == new_url),
        "pre-triage must contain the newly refreshed article"
    );
}

#[test]
fn archive_clicked_with_pre_triage_reviewing_has_zero_pending_count() {
    init_logging();
    // Reviewing phase: resolved_included_urls() returns empty because only ReadyToTriage
    // exposes a committed pre-triage URL set. pending_pre_triage_count must be 0 — no warning
    // shown while user is mid-review.
    //
    // Build reviewing state: load two "review-verdict" articles (word count ~100 words,
    // in SmallMediumContent band), then set a manual decision on one. The other stays
    // unresolved → Reviewing phase.
    let state = complete_triage_state_for_test(1);
    let review_content: String = std::iter::repeat_n("longword", 100)
        .collect::<Vec<_>>()
        .join(" ");
    let url1 = "https://review.com/1";
    let url2 = "https://review.com/2";
    let state = add_completed_job_for_test(state, url1);
    let state = add_completed_job_for_test(state, url2);
    let (state, request_id) = tick_until_dispatch(state);
    let articles = vec![
        LoadedArticle {
            url: url1.to_string(),
            source_title: None,
            prepared_text: review_content.clone(),
            content_hash: format!("hash-{url1}"),
            fetched_utc: None,
        },
        LoadedArticle {
            url: url2.to_string(),
            source_title: None,
            prepared_text: review_content,
            content_hash: format!("hash-{url2}"),
            fetched_utc: None,
        },
    ];
    let (state, _) = update(
        state,
        Msg::TriageArticlesLoaded {
            request_id,
            articles,
        },
    );
    assert!(
        matches!(
            state.pre_triage().phase(),
            crate::pre_triage_filter::PreTriagePhase::Reviewing
        ),
        "unresolved review articles should derive Reviewing immediately after load"
    );
    // Set a manual decision on the first article; the second remains unresolved.
    let key = state.pre_triage().entries()[0].key.clone();
    let (state, _) = update(
        state,
        Msg::PreTriageDecisionSet {
            key,
            decision: crate::pre_triage_filter::ManualDecision::Include,
        },
    );
    assert!(
        matches!(
            state.pre_triage().phase(),
            crate::pre_triage_filter::PreTriagePhase::Reviewing
        ),
        "must be Reviewing after first decision with second still unresolved"
    );

    let (_, effects) = update(state, Msg::ArchiveClicked);
    let pending_count = effects
        .iter()
        .find_map(|e| {
            if let Effect::OpenArchiveDialog {
                pending_pre_triage_count,
                ..
            } = e
            {
                Some(*pending_pre_triage_count)
            } else {
                None
            }
        })
        .expect("expected OpenArchiveDialog effect");
    assert_eq!(
        pending_count, 0,
        "Reviewing phase → resolved_included_urls() is empty → pending count must be 0"
    );
}

#[test]
fn archive_dialog_ready_ignores_stale_request() {
    init_logging();
    let state = AppState::new();
    let (state, effects) = update(
        state,
        Msg::ArchiveDialogReady {
            request_id: 99,
            article_count: 0,
            since_utc: None,
            default_basename: "archive.md".to_string(),
            default_file_exists: false,
            export_dir: std::path::PathBuf::from("/tmp"),
            pending_pre_triage_count: 0,
        },
    );
    assert!(effects.is_empty());
    assert_eq!(state.archive_request_id(), 0);
}

#[test]
fn archive_dialog_ready_emits_show_dialog_for_current_request() {
    init_logging();
    let state = add_completed_job_for_test(AppState::new(), "https://example.com/1");
    let (state, _) = update(state, Msg::ArchiveClicked);
    let request_id = state.archive_request_id();
    let (state, effects) = update(
        state,
        Msg::ArchiveDialogReady {
            request_id,
            article_count: 1,
            since_utc: None,
            default_basename: "archive.md".to_string(),
            default_file_exists: false,
            export_dir: std::path::PathBuf::from("/tmp"),
            pending_pre_triage_count: 0,
        },
    );
    assert_eq!(state.archive_request_id(), 1);
    let effect = effects
        .into_iter()
        .find(|effect| matches!(effect, Effect::ShowArchiveDialog { .. }))
        .expect("ShowArchiveDialog effect expected");
    match effect {
        Effect::ShowArchiveDialog {
            request_id,
            article_count,
            since_utc,
            default_basename,
            default_file_exists,
            export_dir,
            ..
        } => {
            assert_eq!(request_id, 1);
            assert_eq!(article_count, 1);
            assert!(since_utc.is_none());
            assert_eq!(default_basename, "archive.md");
            assert!(!default_file_exists);
            assert_eq!(export_dir, std::path::PathBuf::from("/tmp"));
        }
        _ => unreachable!(),
    }
}

#[test]
fn archive_dialog_submitted_validates_basename_and_checkpoint_flag() {
    init_logging();
    let since = chrono::DateTime::parse_from_rfc3339("2026-03-21T18:17:00Z")
        .unwrap()
        .with_timezone(&chrono::Utc);
    let state = add_completed_job_for_test(AppState::new(), "https://example.com/1");
    let (state, _) = update(state, Msg::ArchiveClicked);
    let request_id = state.archive_request_id();

    let (_state, effects) = update(
        state,
        Msg::ArchiveDialogSubmitted {
            request_id,
            basename: "custom-archive.md".to_string(),
            set_checkpoint: true,
            submitted_at: since,
        },
    );
    let effect = effects
        .into_iter()
        .find(|effect| matches!(effect, Effect::ArchiveRequested { .. }))
        .expect("ArchiveRequested effect expected");
    match effect {
        Effect::ArchiveRequested {
            request_id,
            basename,
            ordered_urls,
            since_utc,
            requested_checkpoint,
        } => {
            assert_eq!(request_id, 1);
            assert_eq!(basename, "custom-archive.md");
            assert_eq!(ordered_urls.len(), 0);
            assert!(since_utc.is_none());
            assert_eq!(requested_checkpoint, Some(since));
        }
        _ => unreachable!(),
    }
}

#[test]
fn archive_dialog_submitted_with_only_pre_triage_ready_exports_zero_urls() {
    init_logging();
    let since = chrono::DateTime::parse_from_rfc3339("2026-03-21T18:17:00Z")
        .unwrap()
        .with_timezone(&chrono::Utc);
    let state = ready_pre_triage_state(&["https://example.com/1"]);
    let (state, _) = update(state, Msg::ArchiveClicked);
    let request_id = state.archive_request_id();

    let (_state, effects) = update(
        state,
        Msg::ArchiveDialogSubmitted {
            request_id,
            basename: "archive.md".to_string(),
            set_checkpoint: false,
            submitted_at: since,
        },
    );
    let effect = effects
        .into_iter()
        .find(|effect| matches!(effect, Effect::ArchiveRequested { .. }))
        .expect("ArchiveRequested effect expected");
    match effect {
        Effect::ArchiveRequested {
            ordered_urls,
            since_utc,
            requested_checkpoint,
            ..
        } => {
            assert_eq!(ordered_urls, Vec::<String>::new());
            assert!(since_utc.is_none());
            assert_eq!(requested_checkpoint, None);
        }
        _ => unreachable!(),
    }
}

#[test]
fn archive_dialog_submitted_rejects_invalid_basename() {
    init_logging();
    let state = add_completed_job_for_test(AppState::new(), "https://example.com/1");
    let (state, _) = update(state, Msg::ArchiveClicked);
    let (_, effects) = update(
        state,
        Msg::ArchiveDialogSubmitted {
            request_id: 1,
            basename: "../bad.md".to_string(),
            set_checkpoint: true,
            submitted_at: chrono::Utc::now(),
        },
    );
    assert!(effects.is_empty());
}

#[test]
fn archive_export_completed_only_saves_checkpoint_for_current_request() {
    init_logging();
    let state = add_completed_job_for_test(AppState::new(), "https://example.com/1");
    let (state, _) = update(state, Msg::ArchiveClicked);
    let checkpoint = chrono::DateTime::parse_from_rfc3339("2026-03-22T00:00:00Z")
        .unwrap()
        .with_timezone(&chrono::Utc);
    let (state, effects) = update(
        state,
        Msg::ArchiveExportCompleted {
            request_id: 1,
            path: std::path::PathBuf::from("archive.md"),
            doc_count: 1,
            requested_checkpoint: Some(checkpoint),
        },
    );
    assert_eq!(
        effects,
        vec![Effect::SaveBriefingCheckpoint {
            save_id: 1,
            since_utc: Some(checkpoint)
        }]
    );
    assert_eq!(state.briefing_since_utc(), Some(checkpoint));
    assert_eq!(
        state.briefing_checkpoint_status_message(),
        Some("Checkpoint saving...")
    );
}

#[test]
fn briefing_checkpoint_save_success_clears_pending_status() {
    init_logging();
    let checkpoint = chrono::DateTime::parse_from_rfc3339("2026-03-22T00:00:00Z")
        .unwrap()
        .with_timezone(&chrono::Utc);
    let (state, effects) = update(
        AppState::new(),
        Msg::BriefingCheckpointSet(Some("2026-03-22T00:00:00Z".to_string())),
    );
    assert_eq!(
        effects,
        vec![Effect::SaveBriefingCheckpoint {
            save_id: 1,
            since_utc: Some(checkpoint)
        }]
    );
    assert_eq!(
        state.pending_briefing_checkpoint_save(),
        Some(crate::state::PendingBriefingCheckpointSaveSnapshot {
            save_id: 1,
            previous_since_utc: None,
            pending_since_utc: Some(checkpoint),
        })
    );

    let (state, follow_up) = update(state, Msg::BriefingCheckpointSaveSucceeded { save_id: 1 });
    assert!(follow_up.is_empty());
    assert_eq!(state.briefing_since_utc(), Some(checkpoint));
    assert_eq!(state.pending_briefing_checkpoint_save(), None);
    assert_eq!(state.briefing_checkpoint_status_message(), None);
}

#[test]
fn briefing_checkpoint_save_failure_reverts_in_memory_state() {
    init_logging();
    let initial = chrono::DateTime::parse_from_rfc3339("2026-03-20T00:00:00Z")
        .unwrap()
        .with_timezone(&chrono::Utc);
    let mut state = AppState::new();
    state.set_briefing_since_utc(Some(initial));
    let (state, effects) = update(
        state,
        Msg::BriefingCheckpointSet(Some("2026-03-22T00:00:00Z".to_string())),
    );
    assert!(matches!(
        effects.as_slice(),
        [Effect::SaveBriefingCheckpoint { save_id: 1, .. }]
    ));
    let (state, follow_up) = update(
        state,
        Msg::BriefingCheckpointSaveFailed {
            save_id: 1,
            reason: "disk full".to_string(),
        },
    );
    assert!(follow_up.is_empty());
    assert_eq!(state.briefing_since_utc(), Some(initial));
    assert_eq!(state.pending_briefing_checkpoint_save(), None);
    assert_eq!(
        state.briefing_checkpoint_status_message(),
        Some("Checkpoint save failed: disk full")
    );
}

#[test]
fn briefing_checkpoint_loaded_clears_pending_save_tracking() {
    init_logging();
    let checkpoint = chrono::DateTime::parse_from_rfc3339("2026-03-22T00:00:00Z")
        .unwrap()
        .with_timezone(&chrono::Utc);
    let (state, _) = update(
        AppState::new(),
        Msg::BriefingCheckpointSet(Some("2026-03-22T00:00:00Z".to_string())),
    );
    assert!(state.pending_briefing_checkpoint_save().is_some());

    let (state, follow_up) = update(
        state,
        Msg::BriefingCheckpointLoaded {
            since_utc: Some("2026-03-25T00:00:00Z".to_string()),
        },
    );
    assert!(follow_up.is_empty());
    assert_eq!(
        state.briefing_since_utc(),
        Some(
            chrono::DateTime::parse_from_rfc3339("2026-03-25T00:00:00Z")
                .unwrap()
                .with_timezone(&chrono::Utc)
        )
    );
    assert_eq!(state.pending_briefing_checkpoint_save(), None);
    assert_eq!(state.briefing_checkpoint_status_message(), None);
    assert_ne!(state.briefing_since_utc(), Some(checkpoint));
}

#[test]
fn stale_checkpoint_save_failure_is_ignored_when_newer_save_is_pending() {
    init_logging();
    let first_checkpoint = chrono::DateTime::parse_from_rfc3339("2026-03-20T00:00:00Z")
        .unwrap()
        .with_timezone(&chrono::Utc);
    let latest_checkpoint = chrono::DateTime::parse_from_rfc3339("2026-03-22T00:00:00Z")
        .unwrap()
        .with_timezone(&chrono::Utc);
    let (state, _) = update(
        AppState::new(),
        Msg::BriefingCheckpointSet(Some("2026-03-20T00:00:00Z".to_string())),
    );
    let (state, effects) = update(
        state,
        Msg::BriefingCheckpointSet(Some("2026-03-22T00:00:00Z".to_string())),
    );
    assert!(matches!(
        effects.as_slice(),
        [Effect::SaveBriefingCheckpoint { save_id: 2, .. }]
    ));

    let (state, follow_up) = update(
        state,
        Msg::BriefingCheckpointSaveFailed {
            save_id: 1,
            reason: "stale failure".to_string(),
        },
    );
    assert!(follow_up.is_empty());
    assert_eq!(state.briefing_since_utc(), Some(latest_checkpoint));
    assert_eq!(
        state.pending_briefing_checkpoint_save(),
        Some(crate::state::PendingBriefingCheckpointSaveSnapshot {
            save_id: 2,
            previous_since_utc: Some(first_checkpoint),
            pending_since_utc: Some(latest_checkpoint),
        })
    );
}

// ── Archive: pinned corpus tests (9 & 10) ────────────────────────────────

/// Test 9: archive open and submit use identical pinned corpus.
///
/// Verifies that `ArchiveClicked` pins the triage corpus at open time and that
/// `ArchiveDialogSubmitted` exports exactly the same URLs without re-selecting.
#[test]
fn archive_open_and_submit_use_identical_pinned_corpus() {
    init_logging();
    let since = chrono::DateTime::parse_from_rfc3339("2026-03-22T00:00:00Z")
        .unwrap()
        .with_timezone(&chrono::Utc);
    // Use TriageComplete state with 2 articles — archive corpus uses triage-only.
    let state = complete_triage_state_for_test(2);

    // Open the archive dialog — triage corpus is pinned here.
    let (state, open_effects) = update(state, Msg::ArchiveClicked);
    let request_id = state.archive_request_id();

    // Verify article_count in OpenArchiveDialog reflects the pinned corpus.
    let open_effect = open_effects
        .into_iter()
        .find(|e| matches!(e, Effect::OpenArchiveDialog { .. }))
        .expect("OpenArchiveDialog effect expected");
    let open_count = match open_effect {
        Effect::OpenArchiveDialog { article_count, .. } => article_count,
        _ => unreachable!(),
    };
    assert_eq!(
        open_count, 2,
        "open dialog should report 2 pinned triage articles"
    );

    // Submit — should export the same 2 triage URLs.
    let (_state, submit_effects) = update(
        state,
        Msg::ArchiveDialogSubmitted {
            request_id,
            basename: "archive.md".to_string(),
            set_checkpoint: false,
            submitted_at: since,
        },
    );
    let submit_effect = submit_effects
        .into_iter()
        .find(|e| matches!(e, Effect::ArchiveRequested { .. }))
        .expect("ArchiveRequested effect expected");
    match submit_effect {
        Effect::ArchiveRequested { ordered_urls, .. } => {
            assert_eq!(
                ordered_urls,
                vec![
                    "https://triage-complete.com/0".to_string(),
                    "https://triage-complete.com/1".to_string()
                ],
                "submitted export must use the pinned triage corpus"
            );
        }
        _ => unreachable!(),
    }
}

/// Test 10: refresh between open and submit still uses the pinned snapshot.
///
/// Simulates a pre-triage refresh arriving while the archive dialog is open.
/// The submitted export must still use the corpus that was pinned at open time
/// (triage-only), not the updated live state (which includes pre-triage).
#[test]
fn refresh_between_open_and_submit_uses_pinned_snapshot() {
    init_logging();
    let since = chrono::DateTime::parse_from_rfc3339("2026-03-22T00:00:00Z")
        .unwrap()
        .with_timezone(&chrono::Utc);

    // Start with TriageComplete (1 article) and no pre-triage.
    let triage_url = "https://triage-complete.com/0"; // URL produced by complete_triage_state_for_test(1)
    let state = complete_triage_state_for_test(1);

    // Open the dialog — triage corpus (1 URL) is pinned.
    let (state, open_effects) = update(state, Msg::ArchiveClicked);
    let request_id = state.archive_request_id();
    let open_count = open_effects
        .into_iter()
        .find_map(|e| match e {
            Effect::OpenArchiveDialog { article_count, .. } => Some(article_count),
            _ => None,
        })
        .expect("OpenArchiveDialog effect expected");
    assert_eq!(open_count, 1, "open dialog must see 1 triage article");

    // Simulate a pre-triage refresh that adds a new article while the dialog is open.
    let pre_triage_url = "https://pretriage.com/1";
    let state = add_completed_job_for_test(state, pre_triage_url);
    let (state, request_id2) = tick_until_dispatch(state);
    let (state, _) = update(
        state,
        Msg::TriageArticlesLoaded {
            request_id: request_id2,
            articles: loaded_pre_triage_articles(&[pre_triage_url]),
        },
    );
    // Pre-triage is now ReadyToTriage; live working corpus has 1 pre-triage article.
    // But the archive corpus (triage-only) still has 1 triage article.
    assert_eq!(
        state.archive_corpus().count(),
        1,
        "archive corpus must still see 1 triage article after pre-triage refresh"
    );

    // Submit the dialog — must export exactly the 1 triage URL pinned at open time.
    let (_state, submit_effects) = update(
        state,
        Msg::ArchiveDialogSubmitted {
            request_id,
            basename: "archive.md".to_string(),
            set_checkpoint: false,
            submitted_at: since,
        },
    );
    let submit_effect = submit_effects
        .into_iter()
        .find(|e| matches!(e, Effect::ArchiveRequested { .. }))
        .expect("ArchiveRequested effect expected");
    match submit_effect {
        Effect::ArchiveRequested { ordered_urls, .. } => {
            assert_eq!(
                ordered_urls,
                vec![triage_url.to_string()],
                "submit must use pinned triage corpus from open time, not pre-triage"
            );
        }
        _ => unreachable!(),
    }
}

// ── Corpus selector parity tests ─────────────────────────────────────────

/// Helper: build a state where triage has completed with `n` articles (all at priority 3).
///
/// Pre-triage is left in its default idle state so the corpus source is `TriageComplete`.
/// This bypasses the reducer path for triage loading to avoid triggering the pre-triage
/// coordinator (which would set `PreTriageReady` and shadow the triage corpus).
///
/// Articles are assigned priority 3, which is above the default `cutoff_exclusive` of 1,
/// ensuring all articles appear in the `TriageSelectionPolicy` result. If the default
/// cutoff is changed, update this helper accordingly.
fn complete_triage_state_for_test(n: usize) -> AppState {
    assert!(n > 0, "n must be > 0 for a useful complete triage state");
    let mut session = crate::triage::TriageSession::new_loading(None);
    let articles: Vec<_> = (0..n)
        .map(|i| LoadedArticle {
            url: format!("https://triage-complete.com/{i}"),
            source_title: None,
            prepared_text: std::iter::repeat_n("triage-content", 220)
                .collect::<Vec<_>>()
                .join(" "),
            content_hash: format!("hash-tc-{i}"),
            fetched_utc: None,
        })
        .collect();
    session.set_articles(articles);
    session.transition_to_triaging();
    for i in 0..n {
        session.complete_article(
            i,
            crate::triage::ArticleTriageResult {
                category: "tech".to_string(),
                priority: 3,
                tags: vec![],
                rationale: "r".to_string(),
                input_tokens: 0,
                output_tokens: 0,
            },
        );
    }
    session.complete();
    assert!(
        matches!(session.phase(), crate::triage::TriagePhase::Complete),
        "triage must be Complete after completing all {n} articles"
    );

    let mut state = AppState::new();
    state.set_triage(session);
    state
}

/// Parity test A: for a `PreTriageReady` state with N articles (no triage done),
/// the archive article_count must be 0 (pre-triage excluded from archive),
/// and the pending_pre_triage_count must equal N.
#[test]
fn parity_a_pre_triage_ready_archive_count_is_zero_pending_count_is_nonzero() {
    init_logging();
    let urls = &["https://parity-a.com/1", "https://parity-a.com/2"];
    let state = ready_pre_triage_state(urls);
    let since = chrono::DateTime::parse_from_rfc3339("2026-03-22T00:00:00Z")
        .unwrap()
        .with_timezone(&chrono::Utc);

    // Assert live corpus properties before any action.
    let corpus = state.current_working_corpus();
    assert_eq!(
        corpus.source(),
        crate::working_corpus::CurrentWorkingCorpusSource::PreTriageReady,
        "source must be PreTriageReady"
    );

    let (state, open_effects) = update(state, Msg::ArchiveClicked);
    let request_id = state.archive_request_id();
    let (archive_count, pending_count) = open_effects
        .into_iter()
        .find_map(|e| match e {
            Effect::OpenArchiveDialog {
                article_count,
                pending_pre_triage_count,
                ..
            } => Some((article_count, pending_pre_triage_count)),
            _ => None,
        })
        .expect("OpenArchiveDialog effect expected");
    assert_eq!(
        archive_count, 0,
        "archive must be empty when only pre-triage is ready"
    );
    assert_eq!(
        pending_count,
        urls.len(),
        "pending count must equal the number of pre-triage ready articles"
    );

    let (_state, submit_effects) = update(
        state,
        Msg::ArchiveDialogSubmitted {
            request_id,
            basename: "archive.md".to_string(),
            set_checkpoint: false,
            submitted_at: since,
        },
    );
    let submit_urls = submit_effects
        .into_iter()
        .find_map(|e| match e {
            Effect::ArchiveRequested { ordered_urls, .. } => Some(ordered_urls),
            _ => None,
        })
        .expect("ArchiveRequested effect expected");
    assert_eq!(
        submit_urls,
        Vec::<String>::new(),
        "submit must export zero URLs when only pre-triage is ready"
    );
}

/// Parity test B: for a `TriageComplete` state with N articles (no ready pre-triage) the
/// visible corpus count, the `OpenArchiveDialog` article_count, and the `ArchiveRequested`
/// URL count are all N and all sourced from the same `TriageComplete` corpus.
#[test]
fn parity_b_triage_complete_corpus_count_dialog_count_urls_match() {
    init_logging();
    let state = complete_triage_state_for_test(2);
    let since = chrono::DateTime::parse_from_rfc3339("2026-03-22T00:00:00Z")
        .unwrap()
        .with_timezone(&chrono::Utc);

    // Assert live corpus properties before any action.
    let corpus = state.current_working_corpus();
    assert_eq!(
        corpus.source(),
        crate::working_corpus::CurrentWorkingCorpusSource::TriageComplete,
        "source must be TriageComplete when pre-triage is idle"
    );
    let expected_count = corpus.count();
    let expected_urls: Vec<String> = corpus.ordered_urls().to_vec();
    assert!(
        expected_count > 0,
        "corpus must be non-empty for a meaningful parity test"
    );

    // Drive ArchiveClicked → assert OpenArchiveDialog article_count == expected_count.
    let (state, open_effects) = update(state, Msg::ArchiveClicked);
    let request_id = state.archive_request_id();
    let dialog_count = open_effects
        .into_iter()
        .find_map(|e| match e {
            Effect::OpenArchiveDialog { article_count, .. } => Some(article_count),
            _ => None,
        })
        .expect("OpenArchiveDialog effect expected");
    assert_eq!(
        dialog_count, expected_count,
        "OpenArchiveDialog article_count must equal corpus.count()"
    );

    // Drive ArchiveDialogSubmitted → assert ArchiveRequested URLs match corpus.ordered_urls().
    let (_state, submit_effects) = update(
        state,
        Msg::ArchiveDialogSubmitted {
            request_id,
            basename: "archive.md".to_string(),
            set_checkpoint: false,
            submitted_at: since,
        },
    );
    let submit_urls = submit_effects
        .into_iter()
        .find_map(|e| match e {
            Effect::ArchiveRequested { ordered_urls, .. } => Some(ordered_urls),
            _ => None,
        })
        .expect("ArchiveRequested effect expected");
    assert_eq!(
        submit_urls.len(),
        expected_count,
        "ArchiveRequested url count must equal corpus.count()"
    );
    assert_eq!(
        submit_urls, expected_urls,
        "ArchiveRequested URLs must match corpus.ordered_urls()"
    );
}

/// Test 8: checkpoint-scoped corpus with non-zero visible count.
///
/// When a briefing checkpoint (`since_utc`) is set, the corpus count (number of articles
/// available for export) must be independent of the checkpoint timestamp. The checkpoint
/// controls the archive time window, NOT which articles are in the corpus.  This test
/// verifies that the dialog's `article_count` equals `corpus.count()` even when a
/// checkpoint is active.
#[test]
fn checkpoint_set_does_not_reduce_corpus_count_to_zero() {
    init_logging();
    // Build a TriageComplete state with 2 articles.
    let mut state = complete_triage_state_for_test(2);

    // Set a briefing checkpoint so since_utc is non-None.
    let checkpoint = chrono::DateTime::parse_from_rfc3339("2026-01-01T00:00:00Z")
        .unwrap()
        .with_timezone(&chrono::Utc);
    state.set_briefing_since_utc(Some(checkpoint));
    assert_eq!(
        state.briefing_since_utc(),
        Some(checkpoint),
        "checkpoint must be set before the test"
    );

    // Assert corpus is non-empty and TriageComplete despite the checkpoint.
    let corpus = state.current_working_corpus();
    assert_eq!(
        corpus.source(),
        crate::working_corpus::CurrentWorkingCorpusSource::TriageComplete,
        "corpus source must be TriageComplete"
    );
    assert!(
        corpus.count() > 0,
        "corpus count must be non-zero even when briefing checkpoint is set"
    );
    let expected_count = corpus.count();
    let expected_urls: Vec<String> = corpus.ordered_urls().to_vec();

    // Drive ArchiveClicked → the dialog must report the same non-zero count.
    let (state, open_effects) = update(state, Msg::ArchiveClicked);
    let dialog_since_utc = open_effects
        .iter()
        .find_map(|e| match e {
            Effect::OpenArchiveDialog { since_utc, .. } => Some(*since_utc),
            _ => None,
        })
        .expect("OpenArchiveDialog effect expected");
    let dialog_count = open_effects
        .into_iter()
        .find_map(|e| match e {
            Effect::OpenArchiveDialog { article_count, .. } => Some(article_count),
            _ => None,
        })
        .expect("OpenArchiveDialog effect (article_count) expected");

    // The checkpoint is passed through as since_utc for the archive time window.
    assert_eq!(
        dialog_since_utc,
        Some(checkpoint),
        "OpenArchiveDialog must propagate the briefing checkpoint as since_utc"
    );
    // The corpus count is NOT affected by the checkpoint.
    assert_eq!(
        dialog_count, expected_count,
        "OpenArchiveDialog article_count must equal corpus.count() — checkpoint must not reduce it"
    );
    assert!(
        dialog_count > 0,
        "article_count must be non-zero when checkpoint is set and triage has completed articles"
    );

    // Submit and confirm URLs also match.
    let request_id = state.archive_request_id();
    let submit_since = chrono::DateTime::parse_from_rfc3339("2026-03-22T00:00:00Z")
        .unwrap()
        .with_timezone(&chrono::Utc);
    let (_state, submit_effects) = update(
        state,
        Msg::ArchiveDialogSubmitted {
            request_id,
            basename: "archive.md".to_string(),
            set_checkpoint: false,
            submitted_at: submit_since,
        },
    );
    let submit_urls = submit_effects
        .into_iter()
        .find_map(|e| match e {
            Effect::ArchiveRequested { ordered_urls, .. } => Some(ordered_urls),
            _ => None,
        })
        .expect("ArchiveRequested effect expected");
    assert_eq!(
        submit_urls.len(),
        expected_count,
        "ArchiveRequested url count must equal corpus.count() even with checkpoint set"
    );
    assert_eq!(
        submit_urls, expected_urls,
        "submit URLs must match corpus URLs even with checkpoint set"
    );
}

// ── Pre-triage refresh coordinator: shared test helpers ──────────────────

/// Advance ticks (up to 200) until the coordinator dispatches a
/// `LoadArticlesForTriage` effect. Panics if no dispatch occurs.
fn tick_until_dispatch(mut state: AppState) -> (AppState, u64) {
    for _ in 0..200 {
        let (next, effects) = update(state, Msg::Tick);
        state = next;
        if let Some(request_id) = effects.iter().find_map(|e| match e {
            Effect::LoadArticlesForTriage { request_id, .. } => Some(*request_id),
            _ => None,
        }) {
            return (state, request_id);
        }
    }
    panic!("no LoadArticlesForTriage dispatch within 200 ticks");
}

fn add_completed_job_for_test(state: AppState, url: &str) -> AppState {
    use crate::JobResultKind;
    let (state, effects) = update(state, Msg::InputChanged(format!("{url}\n")));
    let (state, effects2) = update(state, Msg::UrlsSubmitted);
    let job_id = effects
        .into_iter()
        .chain(effects2)
        .find_map(|e| match e {
            Effect::EnqueueUrl { job_id, .. } => Some(job_id),
            _ => None,
        })
        .expect("EnqueueUrl effect expected");
    let (state, _) = update(
        state,
        Msg::JobDone {
            job_id,
            result: JobResultKind::Success,
            content_preview: None,
            extracted_links: Vec::new(),
            fetched_utc: None,
        },
    );
    apply_pending_pre_triage_refresh_evaluation(state)
}

fn apply_pending_pre_triage_refresh_evaluation(mut state: AppState) -> AppState {
    if let Some(triggered_by_job_done) = state.take_pre_triage_refresh_evaluation_request() {
        let ordered_urls = state.ordered_completed_job_urls_snapshot();
        let (next, _) = update(
            state,
            Msg::EvaluatePreTriageRefresh {
                ordered_urls,
                triggered_by_job_done,
            },
        );
        next
    } else {
        state
    }
}

fn loaded_pre_triage_articles(urls: &[&str]) -> Vec<LoadedArticle> {
    urls.iter()
        .map(|url| LoadedArticle {
            url: (*url).to_string(),
            source_title: None,
            prepared_text: std::iter::repeat_n("pre-triage-content", 220)
                .collect::<Vec<_>>()
                .join(" "),
            content_hash: format!("hash-{url}"),
            fetched_utc: None,
        })
        .collect()
}

fn ready_pre_triage_state(urls: &[&str]) -> AppState {
    let mut state = AppState::new();
    for url in urls {
        state = add_completed_job_for_test(state, url);
    }
    let (state, request_id) = tick_until_dispatch(state);
    let (state, _) = update(
        state,
        Msg::TriageArticlesLoaded {
            request_id,
            articles: loaded_pre_triage_articles(urls),
        },
    );
    assert!(matches!(
        state.pre_triage().phase(),
        crate::pre_triage_filter::PreTriagePhase::ReadyToTriage
    ));
    state
}

mod import_tests;
mod pre_triage_refresh_tests;
mod prompt_lab_tests;
