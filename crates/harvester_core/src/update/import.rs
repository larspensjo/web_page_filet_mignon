use super::*;

pub(super) fn handle_import_requested(
    state: &mut AppState,
    dir: std::path::PathBuf,
) -> Vec<Effect> {
    let request_id = state.allocate_next_llm_request_id();
    engine_info!(
        "[import-saved-web] request id={request_id} dir={}",
        dir.display()
    );
    state.import_session.start_import(request_id, dir.clone());
    vec![Effect::ImportSavedWebpages { dir, request_id }]
}

pub(super) fn handle_import_completed(
    state: &mut AppState,
    request_id: u64,
    report: harvester_engine::ImportReport,
) -> Vec<Effect> {
    if !state.import_session.is_authoritative(request_id) {
        engine_warn!(
            "[import-saved-web] stale completion request_id={request_id}, current={} — ignored",
            state.import_session.request_id
        );
        return Vec::new();
    }

    let imports_completed = report.imported_entries.len();
    let imports_failed = report.failures.len();
    engine_info!(
        "[import-saved-web] completed id={request_id} imported={imports_completed} failed={imports_failed}"
    );

    let imported_entries = report.imported_entries.clone();

    state.import_session.phase = crate::import_session::ImportPhase::Complete;
    state.import_session.imported_entries = imported_entries.clone();
    state.import_session.imports_completed = imports_completed;
    state.import_session.imports_failed = imports_failed;
    state.import_session.duplicate_url_count = report.duplicate_url_count;
    state.import_session.duplicate_content_count = report.duplicate_content_count;
    state.import_session.warnings = report.warnings;
    state.apply_imported_archive_entries(&imported_entries);
    state.request_pre_triage_refresh_evaluation(false);
    Vec::new()
}

pub(super) fn handle_import_failed(
    state: &mut AppState,
    request_id: u64,
    reason: String,
) -> Vec<Effect> {
    if !state.import_session.is_authoritative(request_id) {
        engine_warn!("[import-saved-web] stale failure request_id={request_id} — ignored");
        return Vec::new();
    }
    engine_warn!("[import-saved-web] failed id={request_id} reason={reason}");
    state.import_session.phase = crate::import_session::ImportPhase::Failed;
    state.import_session.failure_reason = Some(reason);
    Vec::new()
}

pub(super) fn handle_corpus_cleared(state: &mut AppState) -> Vec<Effect> {
    engine_info!("[import-saved-web] corpus cleared");
    state.import_session.clear();
    Vec::new()
}
