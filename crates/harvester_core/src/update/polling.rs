use super::*;

pub(super) fn handle_poll_sources_clicked(state: &mut AppState) -> Vec<Effect> {
    if matches!(
        state.session(),
        SessionState::Finishing | SessionState::Finished
    ) || state.is_poll_in_progress()
    {
        Vec::new()
    } else if state.start_poll() {
        engine_info!("[source-poll] polling requested");
        state.pre_triage_coordinator.note_poll_started();
        state.begin_indirect_link_generation();
        vec![Effect::PollAllSources]
    } else {
        Vec::new()
    }
}

pub(super) fn handle_poll_indirect_links(state: &mut AppState) -> Vec<Effect> {
    if !state.has_indirect_links() || state.indirect_poll_in_progress() {
        Vec::new()
    } else {
        state.set_indirect_poll_in_progress(true);
        let links = state.drain_indirect_links();
        let result = state.ingest_indirect_links(links);
        state.set_indirect_poll_in_progress(false);
        result.effects
    }
}

pub(super) fn handle_poll_started(state: &mut AppState, total: usize) -> Vec<Effect> {
    state.set_poll_total(total);
    Vec::new()
}

pub(super) fn handle_source_poll_completed(
    state: &mut AppState,
    source_id: harvester_engine::SourceId,
    urls: Vec<String>,
    kind: harvester_engine::SourceKind,
    parsed: usize,
    dedup_filtered: usize,
) -> Vec<Effect> {
    engine_info!("[source-poll] {} returned {} urls", source_id, urls.len());
    state.record_source_poll(&source_id, urls.len());
    let ingest = state.ingest_urls(urls);
    state.record_poll_stat(crate::SourcePollStat {
        source_id: source_id.clone(),
        kind,
        parsed,
        dedup_filtered,
        emitted: ingest.enqueued,
    });
    ingest.effects
}

pub(super) fn handle_source_poll_failed(
    state: &mut AppState,
    source_id: harvester_engine::SourceId,
    error: String,
) -> Vec<Effect> {
    engine_warn!("[source-poll] {} failed: {}", source_id, error);
    state.record_source_error(&source_id, error);
    Vec::new()
}

pub(super) fn handle_all_sources_poll_ended(state: &mut AppState) -> Vec<Effect> {
    state.end_poll();
    state.pre_triage_coordinator.note_poll_sources_ended();
    state.select_tab(AppTab::PollStats);
    Vec::new()
}
