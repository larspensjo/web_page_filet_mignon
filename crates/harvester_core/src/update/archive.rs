use crate::{AppState, Effect};
use engine_logging::{engine_info, engine_warn};

pub(super) fn handle_archive_clicked(state: &mut AppState) -> Vec<Effect> {
    let request_id = state.allocate_next_archive_request_id();
    let corpus = state.archive_corpus(); // triage-only; pre-triage excluded
    let article_count = corpus.count();
    let fingerprint = corpus.fingerprint();
    let source = corpus.source();
    engine_info!(
        "[working-corpus] source={:?} count={} fingerprint={:#010x} caller=archive-open request_id={}",
        source,
        article_count,
        fingerprint,
        request_id,
    );
    // Count pre-triage articles ready for review. resolved_included_urls() is phase-gated:
    // returns empty when phase is Idle (after consume) or during Reviewing (phase-gated).
    let pending_pre_triage_count = state.pre_triage().resolved_included_urls().len();
    state.pin_archive_corpus(corpus);
    let since_utc = state.briefing_since_utc();
    vec![Effect::OpenArchiveDialog {
        request_id,
        article_count,
        since_utc,
        default_basename: "archive.md".to_string(),
        pending_pre_triage_count,
    }]
}

#[allow(clippy::too_many_arguments)]
pub(super) fn handle_dialog_ready(
    state: &mut AppState,
    request_id: u64,
    article_count: usize,
    since_utc: Option<chrono::DateTime<chrono::Utc>>,
    default_basename: String,
    default_file_exists: bool,
    export_dir: std::path::PathBuf,
    pending_pre_triage_count: usize,
) -> Vec<Effect> {
    if request_id != state.archive_request_id() {
        return Vec::new();
    }
    vec![Effect::ShowArchiveDialog {
        request_id,
        article_count,
        since_utc,
        default_basename,
        default_file_exists,
        export_dir,
        pending_pre_triage_count,
    }]
}

pub(super) fn handle_dialog_submitted(
    state: &mut AppState,
    request_id: u64,
    basename: String,
    set_checkpoint: bool,
    submitted_at: chrono::DateTime<chrono::Utc>,
) -> Vec<Effect> {
    if request_id != state.archive_request_id() {
        return Vec::new();
    }
    if !is_safe_archive_basename(&basename) {
        engine_warn!(
            "[archive-dialog] rejecting invalid basename request_id={} basename={}",
            request_id,
            basename
        );
        return Vec::new();
    }
    let pinned = state.pinned_archive_corpus();
    let (ordered_urls, fingerprint) = match pinned {
        Some(corpus) => (corpus.ordered_urls().to_vec(), corpus.fingerprint()),
        None => {
            // Unreachable in normal operation: ArchiveClicked always precedes
            // ArchiveDialogSubmitted. Guard defensively rather than emit an empty
            // archive file to disk.
            engine_warn!(
                "[archive-dialog] no pinned corpus at submit time request_id={}; dropping submit",
                request_id
            );
            return Vec::new();
        }
    };
    state.clear_pinned_archive_corpus();
    engine_info!(
        "[working-corpus] source=pinned count={} fingerprint={:#010x} caller=archive-submit request_id={}",
        ordered_urls.len(),
        fingerprint,
        request_id,
    );
    let since_utc = state.briefing_since_utc();
    let requested_checkpoint = set_checkpoint.then_some(submitted_at);
    vec![Effect::ArchiveRequested {
        request_id,
        basename,
        ordered_urls,
        since_utc,
        requested_checkpoint,
    }]
}

pub(super) fn handle_export_completed(
    state: &mut AppState,
    request_id: u64,
    requested_checkpoint: Option<chrono::DateTime<chrono::Utc>>,
) -> Vec<Effect> {
    if request_id != state.archive_request_id() {
        return Vec::new();
    }
    // Clear any residual pin (idempotent, normally already cleared at submit time).
    state.clear_pinned_archive_corpus();
    if let Some(checkpoint) = requested_checkpoint {
        let save_id = state.begin_briefing_checkpoint_save(Some(checkpoint));
        state.mark_dirty();
        vec![Effect::SaveBriefingCheckpoint {
            save_id,
            since_utc: Some(checkpoint),
        }]
    } else {
        Vec::new()
    }
}

pub(super) fn handle_export_failed(
    state: &mut AppState,
    request_id: u64,
    basename: String,
    reason: String,
) -> Vec<Effect> {
    if request_id != state.archive_request_id() {
        return Vec::new();
    }
    engine_warn!(
        "[archive-dialog] export failed request_id={} basename={} reason={}",
        request_id,
        basename,
        reason
    );
    // Clear any residual pin (idempotent, normally already cleared at submit time).
    state.clear_pinned_archive_corpus();
    Vec::new()
}

fn is_safe_archive_basename(name: &str) -> bool {
    if name.is_empty() || name == "." || name == ".." {
        return false;
    }
    if name.contains(['/', '\\', '\0']) {
        return false;
    }
    !std::path::Path::new(name).is_absolute()
}
