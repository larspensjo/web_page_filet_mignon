use super::*;
use crate::import_session::ImportPhase;
use harvester_engine::{ImportReport, ImportedArchiveRef};
use std::path::PathBuf;

fn init() {
    static INIT: std::sync::Once = std::sync::Once::new();
    INIT.call_once(engine_logging::initialize_for_tests);
}

fn sample_report(n_imported: usize) -> ImportReport {
    ImportReport {
        scanned_count: n_imported,
        imported_entries: (0..n_imported)
            .map(|i| ImportedArchiveRef {
                persisted_path: PathBuf::from(format!("/archive/{i}.md")),
                canonical_url: format!("https://example.com/{i}"),
                content_hash: format!("hash{i}"),
                fetched_utc: "2026-03-08T00:00:00Z".to_string(),
            })
            .collect(),
        warnings: Vec::new(),
        failures: Vec::new(),
        duplicate_url_count: 0,
        duplicate_content_count: 0,
    }
}

fn import_dir() -> PathBuf {
    PathBuf::from("/saved-pages")
}

fn start_import(state: AppState) -> (AppState, Vec<Effect>) {
    update(
        state,
        Msg::ImportSavedWebpagesRequested { dir: import_dir() },
    )
}

#[test]
fn import_requested_sets_importing_phase_and_emits_effect() {
    init();
    let state = AppState::new();
    let (state, effects) = start_import(state);
    assert_eq!(state.import_session.phase, ImportPhase::Importing);
    assert_eq!(state.import_session.source_dir, Some(import_dir()));
    assert!(effects
        .iter()
        .any(|e| matches!(e, Effect::ImportSavedWebpages { .. })));
}

#[test]
fn import_completion_emits_no_downstream_work() {
    init();
    let state = AppState::new();
    let (state, effects) = start_import(state);
    let Effect::ImportSavedWebpages { request_id, .. } = effects
        .into_iter()
        .find(|e| matches!(e, Effect::ImportSavedWebpages { .. }))
        .unwrap()
    else {
        panic!("expected ImportSavedWebpages effect");
    };

    let (state, effects) = update(
        state,
        Msg::ImportSavedWebpagesCompleted {
            request_id,
            report: sample_report(2),
        },
    );
    assert_eq!(state.import_session.phase, ImportPhase::Complete);
    assert_eq!(state.import_session.imports_completed, 2);
    assert!(
        effects.is_empty(),
        "import completion must emit no downstream work"
    );
}

#[test]
fn import_completion_projects_imported_entries_into_completed_jobs_snapshot() {
    init();
    let state = AppState::new();
    let (state, effects) = start_import(state);
    let Effect::ImportSavedWebpages { request_id, .. } = effects
        .into_iter()
        .find(|e| matches!(e, Effect::ImportSavedWebpages { .. }))
        .unwrap()
    else {
        panic!("expected ImportSavedWebpages effect");
    };

    let imported_fetched_utc = "2026-03-08T06:01:56Z".to_string();
    let (state, _effects) = update(
        state,
        Msg::ImportSavedWebpagesCompleted {
            request_id,
            report: harvester_engine::ImportReport {
                scanned_count: 1,
                imported_entries: vec![harvester_engine::ImportedArchiveRef {
                    persisted_path: PathBuf::from("/archive/imported.md"),
                    canonical_url: "https://example.com/imported".to_string(),
                    content_hash: "hash-imported".to_string(),
                    fetched_utc: imported_fetched_utc.clone(),
                }],
                warnings: Vec::new(),
                failures: Vec::new(),
                duplicate_url_count: 0,
                duplicate_content_count: 0,
            },
        },
    );

    let snapshot = state.completed_jobs_snapshot();
    assert_eq!(snapshot.len(), 1);
    assert_eq!(snapshot[0].url, "https://example.com/imported");
    let actual = snapshot[0]
        .fetched_utc
        .as_deref()
        .and_then(|value| chrono::DateTime::parse_from_rfc3339(value).ok())
        .map(|dt| dt.with_timezone(&chrono::Utc));
    let expected = chrono::DateTime::parse_from_rfc3339(&imported_fetched_utc)
        .ok()
        .map(|dt| dt.with_timezone(&chrono::Utc));
    assert_eq!(actual, expected);
}

#[test]
fn stale_completion_is_ignored_and_emits_no_effects() {
    init();
    let state = AppState::new();
    // Start request A.
    let (state, effects_a) = start_import(state);
    let Effect::ImportSavedWebpages {
        request_id: rid_a, ..
    } = effects_a
        .into_iter()
        .find(|e| matches!(e, Effect::ImportSavedWebpages { .. }))
        .unwrap()
    else {
        panic!();
    };

    // Start request B (supersedes A).
    let (state, _) = start_import(state);

    // Complete A — should be ignored.
    let (state, effects) = update(
        state,
        Msg::ImportSavedWebpagesCompleted {
            request_id: rid_a,
            report: sample_report(1),
        },
    );
    assert!(effects.is_empty(), "stale completion must emit no effects");
    // Phase should still reflect the active (B) session.
    assert_eq!(state.import_session.phase, ImportPhase::Importing);
}

#[test]
fn import_failed_sets_failed_phase() {
    init();
    let state = AppState::new();
    let (state, effects) = start_import(state);
    let Effect::ImportSavedWebpages { request_id, .. } = effects
        .into_iter()
        .find(|e| matches!(e, Effect::ImportSavedWebpages { .. }))
        .unwrap()
    else {
        panic!();
    };

    let (state, effects) = update(
        state,
        Msg::ImportSavedWebpagesFailed {
            request_id,
            reason: "scan failed".to_string(),
        },
    );
    assert_eq!(state.import_session.phase, ImportPhase::Failed);
    assert_eq!(
        state.import_session.failure_reason.as_deref(),
        Some("scan failed")
    );
    assert!(effects.is_empty());
}

#[test]
fn imported_corpus_cleared_resets_state() {
    init();
    let state = AppState::new();
    let (state, _) = start_import(state);
    let (state, _) = update(state, Msg::ImportedCorpusCleared);
    assert_eq!(state.import_session.phase, ImportPhase::Idle);
    assert!(state.import_session.source_dir.is_none());
}

#[test]
fn window_resize_completed_emits_persist_effect() {
    let state = AppState::default();
    let (_, effects) = update(
        state,
        Msg::WindowResizeCompleted {
            outer_width: 1200,
            outer_height: 900,
        },
    );
    assert_eq!(
        effects,
        vec![Effect::PersistWindowSize {
            width: 1200,
            height: 900,
        }]
    );
}

#[test]
fn source_poll_completed_emitted_reflects_ingest_dedup() {
    // If the reducer receives two SourcePollCompleted messages with the same URL,
    // the second one should record emitted=0 because ingest_urls drops the duplicate.
    let state = AppState::new();
    let source_id = harvester_engine::SourceId::new("rss").unwrap();

    let (state, _) = update(
        state,
        Msg::SourcePollCompleted {
            source_id: source_id.clone(),
            urls: vec!["https://example.com/1".to_string()],
            kind: harvester_engine::SourceKind::Rss,
            parsed: 1,
            dedup_filtered: 0,
        },
    );
    let stat = state.source_states().poll_stats().last().unwrap();
    assert_eq!(stat.emitted, 1, "first poll: URL should be enqueued");

    let (state, _) = update(
        state,
        Msg::SourcePollCompleted {
            source_id: source_id.clone(),
            urls: vec!["https://example.com/1".to_string()],
            kind: harvester_engine::SourceKind::Rss,
            parsed: 1,
            dedup_filtered: 0,
        },
    );
    let stat = state.source_states().poll_stats().last().unwrap();
    assert_eq!(
        stat.emitted, 0,
        "second poll: duplicate URL must not be counted as emitted"
    );
}

#[test]
fn poll_stats_cleared_when_new_poll_starts() {
    let state = AppState::new();
    let source_id = harvester_engine::SourceId::new("rss").unwrap();

    // First poll cycle: accumulate a stat.
    let (state, _) = update(state, Msg::PollSourcesClicked);
    let (state, _) = update(
        state,
        Msg::SourcePollCompleted {
            source_id: source_id.clone(),
            urls: vec!["https://example.com/1".to_string()],
            kind: harvester_engine::SourceKind::Rss,
            parsed: 1,
            dedup_filtered: 0,
        },
    );
    assert_eq!(state.source_states().poll_stats().len(), 1);

    // Simulate poll ended so a second PollSourcesClicked is accepted.
    let (state, _) = update(state, Msg::AllSourcesPollEnded);
    assert!(!state.is_poll_in_progress());

    // Second poll cycle: stats should be cleared.
    let (state, _) = update(state, Msg::PollSourcesClicked);
    assert!(
        state.source_states().poll_stats().is_empty(),
        "poll_stats must be cleared when a new poll starts"
    );
}

#[test]
fn poll_started_sets_total() {
    let state = AppState::new();
    let (state, _) = update(state, Msg::PollSourcesClicked);
    let (state, _) = update(state, Msg::PollStarted { total: 5 });
    assert_eq!(state.source_states().poll_progress(), Some((0, 5)));
}

#[test]
fn poll_complete_increments_progress() {
    let state = AppState::new();
    let source_id = harvester_engine::SourceId::new("rss").unwrap();
    let (state, _) = update(state, Msg::PollSourcesClicked);
    let (state, _) = update(state, Msg::PollStarted { total: 2 });
    let (state, _) = update(
        state,
        Msg::SourcePollCompleted {
            source_id,
            urls: vec!["https://example.com/1".to_string()],
            kind: harvester_engine::SourceKind::Rss,
            parsed: 1,
            dedup_filtered: 0,
        },
    );
    assert_eq!(state.source_states().poll_progress(), Some((1, 2)));
}

#[test]
fn poll_failed_increments_progress() {
    let state = AppState::new();
    let source_id = harvester_engine::SourceId::new("rss").unwrap();
    let (state, _) = update(state, Msg::PollSourcesClicked);
    let (state, _) = update(state, Msg::PollStarted { total: 2 });
    let (state, _) = update(
        state,
        Msg::SourcePollFailed {
            source_id,
            error: "boom".to_string(),
        },
    );
    assert_eq!(state.source_states().poll_progress(), Some((1, 2)));
}

#[test]
fn poll_ended_auto_switches_to_poll_stats_tab() {
    let state = AppState::new();
    let (state, _) = update(state, Msg::PollSourcesClicked);
    let (state, _) = update(state, Msg::AllSourcesPollEnded);
    assert_eq!(state.active_tab(), AppTab::PollStats);
}
