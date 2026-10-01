use super::test_support::update;
use super::*;
use crate::briefing::{ArticleSummaryState, BriefingPhase, LoadedArticle};
use crate::signal_candidate::{ArchiveSelectionSource, OverrideKey};
use crate::LlmResultKind;
use harvester_engine::llm::dto::{Confidence, SignalCandidateResult, SourceTier};
use harvester_engine::llm::prompt::PromptId;

mod support;
use support::*;

mod delta_tests;
mod summary_settlement_tests;
mod unfinished_work_tests;

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
        },
    );
    assert!(!state.triage_metadata_ready());
}

fn signal_result(score: u8, key: &str) -> SignalCandidateResult {
    SignalCandidateResult {
        signal_score: score,
        signal_key: key.to_string(),
        themes: vec!["theme".to_string()],
        draft_gist: "gist".to_string(),
        source_tier: SourceTier::Tier1,
        confidence: Confidence::High,
        reasoning: "reason".to_string(),
        input_tokens: 1,
        output_tokens: 1,
    }
}

fn seed_summaries_for_triage_hashes(state: &mut AppState, count: usize) {
    for i in 0..count {
        seed_summary_for_content_hash(state, &format!("hash-tc-{i}"));
    }
}

fn complete_signal_candidate(state: &mut AppState, index: usize, score: u8, key: &str) {
    let url = format!("https://triage-complete.com/{index}");
    state
        .signal_candidate_mut()
        .enqueue(url.clone(), "fixture-input".to_string());
    state
        .signal_candidate_mut()
        .mark_scoring(&url, index as u64 + 1);
    state
        .signal_candidate_mut()
        .complete(&url, signal_result(score, key));
}

#[test]
fn archive_selection_loads_archive_final_selection() {
    init_logging();
    let mut state = complete_triage_state_for_test(2);
    state = with_summary_metadata(state);
    seed_summaries_for_triage_hashes(&mut state, 2);

    let selection = state.archive_final_selection();
    assert_eq!(
        selection.source,
        ArchiveSelectionSource::FullCorpusSignalUnavailable
    );
}

#[test]
fn archive_selection_loads_signal_filtered_archive_final_selection() {
    init_logging();
    let mut state = complete_triage_state_for_test(2);
    state = with_summary_metadata(state);
    state = with_signal_candidate_metadata(state);
    seed_summaries_for_triage_hashes(&mut state, 2);
    complete_signal_candidate(&mut state, 0, 80, "cluster-a");
    complete_signal_candidate(&mut state, 1, 30, "cluster-b");

    let selection = state.archive_final_selection();
    assert_eq!(selection.source, ArchiveSelectionSource::SignalFiltered);
    assert_eq!(
        state.archive_corpus().ordered_urls().to_vec(),
        vec![
            "https://triage-complete.com/0".to_string(),
            "https://triage-complete.com/1".to_string()
        ],
        "fixture must distinguish base corpus from signal-narrowed selection"
    );
}

#[test]
fn archive_selection_preserves_signal_order_and_honors_exclusions() {
    init_logging();
    let mut state = complete_triage_state_for_test(3);
    state = with_summary_metadata(state);
    state = with_signal_candidate_metadata(state);
    seed_summaries_for_triage_hashes(&mut state, 3);
    complete_signal_candidate(&mut state, 0, 70, "cluster-a");
    complete_signal_candidate(&mut state, 1, 95, "cluster-b");
    complete_signal_candidate(&mut state, 2, 85, "cluster-c");
    state.signal_candidate_mut().add_exclusion(OverrideKey {
        signal_key: "cluster-b".to_string(),
        prompt_id: "ArticleSignalCandidate".to_string(),
        prompt_version: 1,
    });

    let expected = state.archive_final_selection();
    assert_eq!(expected.source, ArchiveSelectionSource::SignalFiltered);
    assert_eq!(
        expected.ordered_urls,
        vec![
            "https://triage-complete.com/2".to_string(),
            "https://triage-complete.com/0".to_string()
        ],
        "selection should keep score order after removing the excluded cluster"
    );
}

#[test]
fn archive_selection_reuses_cached_summaries_under_any_key() {
    init_logging();
    let mut state = complete_triage_state_for_test(2);
    state = with_summary_metadata(state);
    state = with_signal_candidate_metadata(state);
    seed_summaries_for_triage_hashes(&mut state, 2);
    complete_signal_candidate(&mut state, 0, 80, "cluster-a");
    complete_signal_candidate(&mut state, 1, 30, "cluster-b");

    let urls = state.archive_corpus().ordered_urls().to_vec();
    assert_eq!(urls.len(), 2);
    assert_eq!(state.archive_final_selection().ordered_urls.len(), 1);
    for url in &urls {
        assert!(state.summary_result_for_url(url).is_some());
    }
    assert_eq!(state.archive_token_estimates(&urls).summary_coverage, 2);
    let (_, effects) = update(state, Msg::ArchiveClicked);
    assert!(effects
        .iter()
        .all(|effect| !matches!(effect, Effect::RequestLlmCompletion { .. })));
}

#[test]
fn prepare_summaries_loads_base_corpus() {
    init_logging();
    let state = complete_triage_state_for_test(2);
    let state = with_summary_metadata(state);

    let (state, effects) = update(
        state,
        Msg::PipelineRunRequested {
            scope: crate::PipelineRunScope::Resume,
        },
    );

    assert!(effects
        .iter()
        .all(|e| !matches!(e, Effect::LoadArticlesForTriage { .. })));
    assert_eq!(
        state
            .briefing()
            .articles()
            .iter()
            .map(|a| a.url.clone())
            .collect::<Vec<_>>(),
        state.archive_corpus().ordered_urls().to_vec()
    );
    assert!(matches!(
        state.briefing().phase(),
        BriefingPhase::Summarizing
    ));
}

#[test]
fn articles_loaded_dispatches_first_summary() {
    init_logging();
    let state = AppState::new();
    let state = start_briefing_after_triage(state, loaded_articles().0.clone());
    let (articles, _collection_text) = loaded_articles();

    let (state, effects) = crate::update::test_support::summarize(state, articles);

    assert_eq!(effects.len(), 1);
    let summary_req_id =
        request_id_for_prompt(&effects, PromptId::ArticleSummary).expect("summary request");
    assert!(matches!(
        &effects[0],
        Effect::RequestLlmCompletion {
            prompt_id: PromptId::ArticleSummary,
            prompt_version: None,
            input_content,
            context,
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
fn summary_completion_advances_and_saves_without_aggregate() {
    init_logging();
    let state = AppState::new();
    let state = start_briefing_after_triage(state, loaded_articles().0.clone());
    let (articles, _collection_text) = loaded_articles();

    // Capture the Article A summary request ID from the effect so the test
    // does not depend on prior allocation counts inside the setup helpers.
    let (state, articles_effects) = crate::update::test_support::summarize(state, articles);
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
                resolved_model: "test-model".to_string(),
            },
            metadata: None,
        },
    );

    assert_eq!(effects.len(), 2);
    assert!(matches!(&effects[0], Effect::SaveResults { records } if records.len() == 1));
    let req_b = request_id_for_prompt(&effects, PromptId::ArticleSummary)
        .expect("Article B summary request");
    assert_ne!(req_b, req_a, "each summary request must have a distinct id");
    assert!(matches!(
        &effects[1],
        Effect::RequestLlmCompletion {
            prompt_id: PromptId::ArticleSummary,
            prompt_version: None,
            input_content,
            context,
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
                resolved_model: "test-model".to_string(),
            },
            metadata: None,
        },
    );

    assert!(effects.iter().any(|e| matches!(e, Effect::FlushResults)));
    assert!(effects
        .iter()
        .any(|e| matches!(e, Effect::SaveResults { records } if records.len() == 1)));
    assert!(effects
        .iter()
        .all(|e| !matches!(e, Effect::RequestLlmCompletion { .. })));
    assert_eq!(state.summary_cache().len(), 2);
    assert_eq!(state.briefing().phase(), &BriefingPhase::Complete);
}

#[test]
fn summary_store_uses_run_frozen_metadata_when_completion_model_differs() {
    init_logging();
    let state = AppState::new();
    let state = start_briefing_after_triage(state, loaded_single_article().0.clone());
    let (articles, _collection_text) = loaded_single_article();
    let (state, effects) = crate::update::test_support::summarize(state, articles);
    let summary_request_id =
        request_id_for_prompt(&effects, PromptId::ArticleSummary).expect("summary request");

    let (state, _) = update(
        state,
        Msg::LlmCompleted {
            request_id: summary_request_id,
            result: LlmResultKind::Success {
                output_json: summary_json("Article A"),
                input_tokens: 10,
                output_tokens: 5,
                prompt_version: 77,
                resolved_model: "test-model-2024-07-18".to_string(),
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
fn summary_persisted_with_dated_model_variant_is_cache_hit_after_reload() {
    // Session 1: provider returns a dated model variant; cache must store with canonical alias.
    init_logging();
    let state = start_briefing_after_triage(AppState::new(), loaded_single_article().0.clone());
    let (articles, _collection_text) = loaded_single_article();
    let (state, effects) = crate::update::test_support::summarize(state, articles.clone());
    let summary_request_id = request_id_for_prompt(&effects, PromptId::ArticleSummary)
        .expect("summary request in session 1");

    let (state, _) = update(
        state,
        Msg::LlmCompleted {
            request_id: summary_request_id,
            result: LlmResultKind::Success {
                output_json: summary_json("Article A"),
                input_tokens: 10,
                output_tokens: 5,
                prompt_version: 99,
                resolved_model: "test-model-2024-07-18".to_string(),
            },
            metadata: None,
        },
    );
    let persisted_cache = state.summary_cache().clone();
    assert!(
        !persisted_cache.is_empty(),
        "session 1 must persist a summary cache entry"
    );

    // Session 2: restart with same config. Hydrate cache. ArticlesLoaded must reuse
    // the entry and emit no summary LLM request.
    let state = start_briefing_after_triage(AppState::new(), articles.clone());
    let (state, _) = update(
        state,
        Msg::SummaryCacheHydrated {
            cache: persisted_cache,
        },
    );
    let (_state, effects) = crate::update::test_support::summarize(state, articles);

    assert!(
        effects.iter().all(|e| !matches!(
            e,
            Effect::RequestLlmCompletion {
                prompt_id: PromptId::ArticleSummary,
                ..
            }
        )),
        "session 2 must get a cache hit; no summary LLM request should be emitted"
    );
}

#[test]
fn summary_success_records_usage_for_status_bar() {
    init_logging();
    let state = AppState::new();
    let state = start_briefing_after_triage(state, loaded_single_article().0.clone());
    let (articles, _collection_text) = loaded_single_article();
    let (state, effects) = crate::update::test_support::summarize(state, articles);
    let summary_request_id =
        request_id_for_prompt(&effects, PromptId::ArticleSummary).expect("summary request");

    let (state, effects) = update(
        state,
        Msg::LlmCompleted {
            request_id: summary_request_id,
            result: LlmResultKind::Success {
                output_json: summary_json("Article A"),
                input_tokens: 123,
                output_tokens: 45,
                prompt_version: 1,
                resolved_model: "test-model".to_string(),
            },
            metadata: Some(summary_metadata("test-model", 123, 45)),
        },
    );

    assert_eq!(state.briefing().phase(), &BriefingPhase::Complete);
    assert!(effects
        .iter()
        .all(|e| !matches!(e, Effect::RequestLlmCompletion { .. })));

    assert_eq!(state.llm_usage_rows().len(), 1);
    assert_eq!(state.llm_usage_rows()[0].model, "test-model");
    assert_eq!(state.llm_usage_rows()[0].input_tokens, 123);
    assert_eq!(state.llm_usage_rows()[0].output_tokens, 45);
}

#[test]
fn second_run_reuses_cached_summary_with_configured_model_key() {
    init_logging();
    let state = AppState::new();
    let state = start_briefing_after_triage(state, loaded_single_article().0.clone());
    let (articles, _collection_text) = loaded_single_article();
    let (state, effects) = crate::update::test_support::summarize(state, articles);
    let summary_request_id =
        request_id_for_prompt(&effects, PromptId::ArticleSummary).expect("summary request");
    let (state, effects) = update(
        state,
        Msg::LlmCompleted {
            request_id: summary_request_id,
            result: LlmResultKind::Success {
                output_json: summary_json("Article A"),
                input_tokens: 10,
                output_tokens: 5,
                prompt_version: 88,
                resolved_model: "test-model-2024-07-18".to_string(),
            },
            metadata: None,
        },
    );
    assert!(effects
        .iter()
        .all(|e| !matches!(e, Effect::RequestLlmCompletion { .. })));
    let (state, effects) = super::test_support::summarize(state, loaded_single_article().0);
    assert_eq!(state.briefing().phase(), &BriefingPhase::Complete);
    assert!(effects
        .iter()
        .all(|e| !matches!(e, Effect::RequestLlmCompletion { .. })));
    assert_eq!(state.summary_cache().len(), 1);
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
    let job_id = state
        .view()
        .desktop_job_list
        .rows
        .first()
        .map(|j| j.job_id)
        .unwrap_or(1);
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
mod blacklist_tests;

mod import_tests;
mod pre_triage_refresh_tests;
mod provider_alert_tests;
mod signal_candidate_tests;
mod triage_tests;
mod ui_state_tests;

#[test]
fn summary_completion_emits_exact_record_before_settlement() {
    let (articles, _collection_text) = loaded_articles();
    let state = start_briefing_after_triage(AppState::new(), articles.clone());
    let (state, effects) = crate::update::test_support::summarize(state, articles);
    let request_id = request_id_for_prompt(&effects, PromptId::ArticleSummary).unwrap();
    let (state, effects) = crate::update(
        state,
        Msg::LlmCompleted {
            request_id,
            result: LlmResultKind::Success {
                output_json: summary_json("Article A"),
                input_tokens: 10,
                output_tokens: 5,
                prompt_version: 1,
                resolved_model: "test-model".into(),
            },
            metadata: None,
        },
    );
    assert_eq!(state.briefing().completed_summary_count(), 1);
    assert!(state.briefing().is_active());
    let records: Vec<_> = effects
        .iter()
        .filter_map(|e| match e {
            Effect::SaveResults { records } => Some(records),
            _ => None,
        })
        .flatten()
        .collect();
    assert_eq!(records.len(), 1);
    let crate::SavedResult::Summary(key, entry) = records[0] else {
        panic!("summary record expected")
    };
    assert_eq!(state.summary_cache().lookup(key), Some(entry));
}

#[test]
fn changed_result_provenance_emits_only_the_changed_record() {
    let key = crate::SummaryCacheKey::try_new(
        "paid-hash",
        PromptId::ArticleSummary,
        Some(1),
        Some("model"),
        &[],
    )
    .unwrap();
    let entry = crate::SummaryCacheEntry {
        result: crate::ArticleSummaryResult {
            title: "title".into(),
            summary: "paid summary".into(),
            key_points: vec![],
            input_tokens: 10,
            output_tokens: 5,
            entities: Default::default(),
        },
        created_at_utc: "2026-09-28T00:00:00Z".into(),
    };
    let mut cache = crate::SummaryCache::new();
    cache.insert(key.clone(), entry.clone());
    let (state, hydration) = crate::update(AppState::new(), Msg::SummaryCacheHydrated { cache });
    assert!(!hydration
        .iter()
        .any(|e| matches!(e, Effect::SaveResults { .. })));
    let mut changed = entry;
    changed.created_at_utc = "2026-09-28T01:00:00Z".into();
    let record = crate::SavedResult::Summary(key.clone(), changed.clone());
    let (state, effects) = crate::update(
        state,
        Msg::ValidatedResultReceived {
            record: Box::new(record.clone()),
        },
    );
    assert_eq!(
        effects,
        vec![Effect::SaveResults {
            records: vec![record]
        }]
    );
    assert_eq!(state.summary_cache().lookup(&key), Some(&changed));
}
