use std::collections::HashMap;

use super::support::*;
use super::*;
use crate::{
    BatchStatus, CollectedEntry, CollectedOutcome, FrozenBatchKey, LlmRequestState, StageKind,
    TriageCacheKey,
};
use harvester_engine::llm::{PromptId, TokenUsage};

fn deferred(request_id: u64) -> Msg {
    Msg::LlmCompleted {
        request_id,
        result: LlmResultKind::DeferredToBatch,
        metadata: None,
    }
}

#[test]
fn deferred_triage_settles_and_rearm_redispatches() {
    init_logging();
    let (state, effects) = start_triage_for_test(AppState::new(), loaded_triage_articles(1));
    let request_id = request_id_for_prompt(&effects, PromptId::ArticleTriage).unwrap();
    let (state, effects) = update(state, deferred(request_id));
    assert!(effects.is_empty());
    assert!(matches!(
        state.llm_request_state(request_id),
        Some(LlmRequestState::Deferred { .. })
    ));
    assert_eq!(state.triage().in_progress_count(), 0);
    assert_eq!(state.triage().failed_count(), 0);
    assert!(matches!(
        state.triage().phase(),
        crate::TriagePhase::AwaitingBatch
    ));
    assert!(!state.triage().is_active());
    assert_eq!(state.batch_status(), BatchStatus::Settled);

    let (state, effects) = update(state, Msg::RearmDeferredBatchStages);
    assert_eq!(state.triage().in_progress_count(), 1);
    assert_eq!(
        effects
            .iter()
            .filter(|effect| matches!(effect, Effect::RequestLlmCompletion { .. }))
            .count(),
        1
    );
}

#[test]
fn deferred_summary_settles_without_failing_article() {
    init_logging();
    let state = start_briefing_after_triage(AppState::new(), loaded_single_article().0.clone());
    let (articles, collection_text) = loaded_single_article();
    let (state, effects) = update(
        state,
        Msg::ArticlesLoaded {
            articles,
            collection_text,
        },
    );
    let request_id = request_id_for_prompt(&effects, PromptId::ArticleSummary).unwrap();
    let (state, _) = update(state, deferred(request_id));
    assert!(matches!(
        state.llm_request_state(request_id),
        Some(LlmRequestState::Deferred { .. })
    ));
    assert_eq!(state.briefing().in_progress_count(), 0);
    assert_eq!(state.briefing().failed_summary_count(), 0);
    assert!(matches!(
        state.briefing().phase(),
        crate::BriefingPhase::AwaitingBatch
    ));
    assert!(!state.briefing().is_active());
    assert_eq!(state.batch_status(), BatchStatus::Settled);
}

fn current_triage_key(content_hash: &str) -> FrozenBatchKey {
    FrozenBatchKey {
        content_hash: content_hash.to_string(),
        prompt_id: PromptId::ArticleTriage,
        prompt_version: 1,
        model_id: "test-model".to_string(),
        context_hash: crate::context_hash(&[]),
        stage: StageKind::Triage,
        url: "https://example.test/article".to_string(),
        rendered_system: "system".to_string(),
        rendered_user: "user".to_string(),
    }
}

fn collected_triage(custom_id: &str, content_hash: &str) -> CollectedEntry {
    CollectedEntry {
        batch_id: "batch-1".to_string(),
        custom_id: custom_id.to_string(),
        stage: StageKind::Triage,
        key: current_triage_key(content_hash),
        created_at_utc: "2026-07-19T00:00:00Z".to_string(),
        outcome: CollectedOutcome::Success {
            raw_output_json: triage_json(),
            usage: TokenUsage::new(20, 8),
            resolved_model: "test-model".to_string(),
        },
    }
}

fn state_with_triage_metadata() -> AppState {
    let mut state = AppState::new();
    state.set_llm_metadata(
        HashMap::from([(PromptId::ArticleTriage, 1)]),
        HashMap::from([(PromptId::ArticleTriage, "test-model".to_string())]),
        HashMap::new(),
    );
    state
}

#[test]
fn collected_successes_insert_frozen_keys_coalesce_persistence_and_do_not_complete_articles() {
    let state = state_with_triage_metadata();
    let entries = vec![
        collected_triage("one", "hash-one"),
        collected_triage("two", "hash-two"),
    ];
    let (state, effects) = update(state, Msg::BatchResultsCollected { entries });

    for hash in ["hash-one", "hash-two"] {
        let key = TriageCacheKey::try_new_with_context_hash(
            hash,
            PromptId::ArticleTriage,
            Some(1),
            Some("test-model"),
            &crate::context_hash(&[]),
        )
        .unwrap();
        assert!(state.triage_cache().lookup(&key).is_some());
    }
    assert_eq!(state.triage().completed_count(), 0);
    assert_eq!(
        effects
            .iter()
            .filter(|effect| matches!(effect, Effect::PersistTriageCache { .. }))
            .count(),
        1
    );
    assert_eq!(state.llm_usage_rows()[0].input_tokens, 40);
    assert_eq!(state.llm_usage_rows()[0].output_tokens, 16);
}

#[test]
fn collected_line_error_and_invalid_output_do_not_write_cache() {
    let state = state_with_triage_metadata();
    let mut invalid = collected_triage("invalid", "hash-invalid");
    invalid.outcome = CollectedOutcome::Success {
        raw_output_json: "{}".to_string(),
        usage: TokenUsage::new(20, 8),
        resolved_model: "test-model".to_string(),
    };
    let mut line_error = collected_triage("line", "hash-line");
    line_error.outcome = CollectedOutcome::LineError {
        detail: "provider rejected line".to_string(),
    };
    let (state, effects) = update(
        state,
        Msg::BatchResultsCollected {
            entries: vec![invalid, line_error],
        },
    );
    assert!(effects.is_empty());
    assert!(state.triage_cache().is_empty());
    assert!(state.llm_usage_rows().is_empty());
}

#[test]
fn collected_summary_rearm_cache_hits_and_runs_post_processing_once() {
    let state = start_briefing_after_triage(AppState::new(), loaded_single_article().0.clone());
    let state = with_signal_candidate_metadata(state);
    let (articles, collection_text) = loaded_single_article();
    let (state, effects) = update(
        state,
        Msg::ArticlesLoaded {
            articles,
            collection_text,
        },
    );
    let request_id = request_id_for_prompt(&effects, PromptId::ArticleSummary).unwrap();
    let key = state.frozen_batch_key_for_request(request_id).unwrap();
    let (state, _) = update(state, deferred(request_id));
    let (state, effects) = update(
        state,
        Msg::BatchResultsCollected {
            entries: vec![CollectedEntry {
                batch_id: "batch-summary".to_string(),
                custom_id: "summary-line".to_string(),
                stage: StageKind::Summary,
                key,
                created_at_utc: "2026-07-19T00:00:00Z".to_string(),
                outcome: CollectedOutcome::Success {
                    raw_output_json: summary_json("Article A"),
                    usage: TokenUsage::new(30, 10),
                    resolved_model: "test-model".to_string(),
                },
            }],
        },
    );
    assert!(effects
        .iter()
        .any(|effect| matches!(effect, Effect::PersistSummaryCache { .. })));

    let (state, effects) = update(state, Msg::RearmDeferredBatchStages);
    assert_eq!(state.briefing().completed_summary_count(), 1);
    assert_eq!(
        effects
            .iter()
            .filter(|effect| matches!(effect, Effect::UpsertEntityIndexEntry { .. }))
            .count(),
        1
    );
    assert_eq!(
        effects
            .iter()
            .filter(|effect| matches!(
                effect,
                Effect::RequestLlmCompletion {
                    prompt_id: PromptId::ArticleSignalCandidate,
                    ..
                }
            ))
            .count(),
        1
    );
    assert!(effects.iter().all(|effect| !matches!(
        effect,
        Effect::RequestLlmCompletion {
            prompt_id: PromptId::ArticleSummary,
            ..
        }
    )));
}
