use harvester_core::{update, AppState, Effect, Msg};
use harvester_engine::{SourceId, SourceKind};

#[test]
fn brave_source_poll_completed_enqueues_urls() {
    let state = AppState::new();

    // Start a poll
    let (state, effects) = update(state, Msg::PollSourcesClicked);
    assert!(effects.contains(&Effect::PollAllSources));

    // Simulate SourcePollCompleted from a Brave source
    let (state, effects) = update(
        state,
        Msg::SourcePollCompleted {
            source_id: SourceId::new("brave-test").unwrap(),
            urls: vec![
                "https://example.com/article-1".to_string(),
                "https://example.com/article-2".to_string(),
            ],
            kind: SourceKind::Brave,
            parsed: 2,
            dedup_filtered: 0,
        },
    );

    // Should have enqueued both URLs
    let enqueue_count = effects
        .iter()
        .filter(|e| matches!(e, Effect::EnqueueUrl { .. }))
        .count();
    assert_eq!(enqueue_count, 2);

    // Simulate poll end
    let (state, _) = update(state, Msg::AllSourcesPollEnded);
    assert!(!state.batch_observation().poll_in_progress);
}

#[test]
fn brave_source_dedup_skips_already_seen_urls() {
    let state = AppState::new();
    let (state, _) = update(state, Msg::PollSourcesClicked);

    // First batch — URL is new, should be enqueued
    let (state, effects1) = update(
        state,
        Msg::SourcePollCompleted {
            source_id: SourceId::new("brave-test").unwrap(),
            urls: vec!["https://example.com/article-1".to_string()],
            kind: SourceKind::Brave,
            parsed: 1,
            dedup_filtered: 0,
        },
    );
    let enqueue1 = effects1
        .iter()
        .filter(|e| matches!(e, Effect::EnqueueUrl { .. }))
        .count();
    assert_eq!(enqueue1, 1);

    // Same URL again — ingest_urls should drop it (reducer-level dedup)
    let (_state, effects2) = update(
        state,
        Msg::SourcePollCompleted {
            source_id: SourceId::new("brave-test").unwrap(),
            urls: vec!["https://example.com/article-1".to_string()],
            kind: SourceKind::Brave,
            parsed: 1,
            dedup_filtered: 0,
        },
    );
    let enqueue2 = effects2
        .iter()
        .filter(|e| matches!(e, Effect::EnqueueUrl { .. }))
        .count();
    assert_eq!(
        enqueue2, 0,
        "duplicate URL should be skipped by ingest_urls"
    );
}
