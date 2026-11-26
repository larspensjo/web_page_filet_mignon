//! Archive export feedback contract (Follow-up, 2026-11-26): a submitted export
//! shows a working notice at once, live read progress while the worker runs, and
//! a done or failed notice afterwards. Lars's report: "When I press the
//! "Archive" button, it takes a few minutes where nothing seems to happen."
use super::*;
use crate::{ArchiveExportNotice, ArchiveExportNoticeKind};

fn submit(request_id: u64) -> Msg {
    Msg::ArchiveDialogSubmitted {
        request_id,
        basename: "archive.md".into(),
        set_checkpoint: false,
        submitted_at: chrono::Utc::now(),
        use_summaries: false,
        use_signal_candidates: false,
    }
}

fn open_request_id(effects: &[Effect]) -> u64 {
    effects
        .iter()
        .find_map(|effect| match effect {
            Effect::OpenArchiveDialog { request_id, .. } => Some(*request_id),
            _ => None,
        })
        .expect("ArchiveClicked opens a dialog")
}

fn notice(state: &AppState) -> ArchiveExportNotice {
    state
        .view()
        .archive_export_notice
        .clone()
        .expect("archive export notice expected")
}

fn submitted_state() -> (AppState, u64) {
    let state = complete_triage_state_for_test(1);
    let (state, effects) = update(state, Msg::ArchiveClicked);
    let request_id = open_request_id(&effects);
    let (state, effects) = update(state, submit(request_id));
    assert!(effects
        .iter()
        .any(|effect| matches!(effect, Effect::ArchiveRequested { .. })));
    (state, request_id)
}

#[test]
fn submit_shows_working_notice_at_once_and_disables_archive() {
    init_logging();
    let state = complete_triage_state_for_test(1);
    assert_eq!(state.view().archive_export_notice, None);
    let (state, _) = submitted_state();
    let shown = notice(&state);
    assert_eq!(shown.kind, ArchiveExportNoticeKind::Working);
    assert_eq!(shown.text, "Exporting archive...");
    assert_eq!(state.view().checkpoint_status_message, None);
    assert!(!state.view().archive_enabled);
}

#[test]
fn progress_updates_the_working_notice_with_grouped_counts() {
    init_logging();
    let (state, request_id) = submitted_state();
    let (state, effects) = update(
        state,
        Msg::ArchiveExportProgress {
            request_id,
            files_checked: 4_000,
            files_total: 17_981,
        },
    );
    assert!(effects.is_empty());
    let shown = notice(&state);
    assert_eq!(shown.kind, ArchiveExportNoticeKind::Working);
    assert_eq!(shown.text, "Reading articles 4,000 of 17,981");
    assert!(!state.view().archive_enabled);
}

#[test]
fn progress_for_a_stale_request_is_ignored() {
    init_logging();
    let (state, request_id) = submitted_state();
    let (state, _) = update(
        state,
        Msg::ArchiveExportProgress {
            request_id: request_id + 7,
            files_checked: 10,
            files_total: 20,
        },
    );
    assert_eq!(notice(&state).text, "Exporting archive...");
}

#[test]
fn completion_shows_done_notice_with_article_count_until_next_open() {
    init_logging();
    let (state, request_id) = submitted_state();
    let (state, _) = update(
        state,
        Msg::ArchiveExportProgress {
            request_id,
            files_checked: 17_981,
            files_total: 17_981,
        },
    );
    let (state, _) = update(
        state,
        Msg::ArchiveExportCompleted {
            request_id,
            path: "archive.md".into(),
            doc_count: 194,
            requested_checkpoint: None,
        },
    );
    let shown = notice(&state);
    assert_eq!(shown.kind, ArchiveExportNoticeKind::Done);
    assert_eq!(shown.text, "archive.md written: 194 articles");
    assert!(state.view().archive_enabled);

    let (state, effects) = update(state, Msg::ArchiveClicked);
    assert_eq!(open_request_id(&effects), request_id + 1);
    assert_eq!(state.view().archive_export_notice, None);
}

#[test]
fn completion_of_a_single_article_uses_the_singular() {
    init_logging();
    let (state, request_id) = submitted_state();
    let (state, _) = update(
        state,
        Msg::ArchiveExportCompleted {
            request_id,
            path: "archive.md".into(),
            doc_count: 1,
            requested_checkpoint: None,
        },
    );
    assert_eq!(notice(&state).text, "archive.md written: 1 article");
}

#[test]
fn done_notice_survives_the_checkpoint_save_that_follows_it() {
    init_logging();
    let (state, request_id) = submitted_state();
    let checkpoint = chrono::Utc::now();
    let (state, effects) = update(
        state,
        Msg::ArchiveExportCompleted {
            request_id,
            path: "archive.md".into(),
            doc_count: 194,
            requested_checkpoint: Some(checkpoint),
        },
    );
    let save_id = effects
        .iter()
        .find_map(|effect| match effect {
            Effect::SaveBriefingCheckpoint { save_id, .. } => Some(*save_id),
            _ => None,
        })
        .expect("checkpoint save requested");
    let (state, _) = update(state, Msg::BriefingCheckpointSaveSucceeded { save_id });
    let shown = notice(&state);
    assert_eq!(shown.kind, ArchiveExportNoticeKind::Done);
    assert_eq!(shown.text, "archive.md written: 194 articles");
}

#[test]
fn failure_shows_failed_notice_and_reenables_archive_until_next_open() {
    init_logging();
    let (state, request_id) = submitted_state();
    let (state, _) = update(
        state,
        Msg::ArchiveExportFailed {
            request_id,
            basename: "archive.md".into(),
            reason: "disk full".into(),
        },
    );
    let shown = notice(&state);
    assert_eq!(shown.kind, ArchiveExportNoticeKind::Failed);
    assert_eq!(shown.text, "Archive export failed: disk full");
    assert!(state.view().archive_enabled);

    let (state, _) = update(state, Msg::ArchiveClicked);
    assert_eq!(state.view().archive_export_notice, None);
}

#[test]
fn second_press_while_exporting_starts_nothing_and_keeps_the_notice() {
    init_logging();
    let (state, request_id) = submitted_state();
    let (state, effects) = update(state, Msg::ArchiveClicked);
    assert!(effects.is_empty());
    assert_eq!(state.archive_request_id(), request_id);
    assert_eq!(notice(&state).kind, ArchiveExportNoticeKind::Working);
}
