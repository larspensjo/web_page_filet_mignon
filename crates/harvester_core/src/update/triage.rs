use crate::pre_triage_filter::{PreTriagePolicy, PreTriageSession};
use crate::state::InitialArticleWindowOutcome as StartupWindow;

use crate::{AppState, Effect};
use engine_logging::{engine_info, engine_warn};

pub(super) fn handle_evaluate_pre_triage_refresh(
    state: &mut AppState,
    ordered_urls: Vec<String>,
    triggered_by_job_done: bool,
) -> Vec<Effect> {
    state.take_pre_triage_refresh_evaluation_request();
    let stopping = state.pipeline_run_phase() == crate::PipelineRunPhase::Stopping
        || state.session() == crate::SessionState::Finishing;
    let stopped_download_drain = triggered_by_job_done && stopping;
    if stopping && !stopped_download_drain {
        return Vec::new();
    }
    // INTENTIONAL EXCEPTION: pre-triage refresh is the mechanism that BUILDS
    // the candidate corpus — it runs before the shared working-corpus selector
    // has anything to select from. It reads from completed jobs (upstream of
    // the selector) and loads article content so that the pre-triage session
    // can be populated. Using the shared selector here would be circular: the
    // selector cannot produce a ReadyToTriage corpus until this refresh finishes.
    let reason = if triggered_by_job_done {
        crate::pre_triage_coordinator::PreTriageRefreshReason::JobDone
    } else {
        crate::pre_triage_coordinator::PreTriageRefreshReason::RestoreCompletedJobs
    };
    schedule_pre_triage_refresh(state, reason, ordered_urls)
}

pub(super) fn handle_articles_loaded(
    state: &mut AppState,
    request_id: u64,
    delta: harvester_engine::TriageArticleDelta,
) -> Vec<Effect> {
    if Some(request_id) != state.triage_in_flight_request_id() {
        engine_info!(
            "[pre-triage-refresh-coord] stale result ignored request_id={} in_flight={:?}",
            request_id,
            state.triage_in_flight_request_id()
        );
        return Vec::new();
    }
    engine_info!("[pre-triage-refresh-coord] apply request_id={}", request_id);
    state.clear_triage_in_flight();
    state.pre_triage_coordinator.complete_request(request_id);
    // Backfill fetched_utc from frontmatter for jobs restored without it (pre-feature state).
    let url_to_fetched: std::collections::HashMap<String, chrono::DateTime<chrono::Utc>> = delta
        .members
        .iter()
        .filter_map(|a| {
            let fu = a.fetched_utc.as_deref()?;
            let dt = chrono::DateTime::parse_from_rfc3339(fu).ok()?;
            Some((a.url.clone(), dt.with_timezone(&chrono::Utc)))
        })
        .collect();
    state.backfill_jobs_fetched_utc(&url_to_fetched);
    let members = delta
        .members
        .iter()
        .map(|m| (m.url.clone(), m.content_hash.clone()))
        .collect();
    super::waves::prune(state, members);
    let policy = PreTriagePolicy::default();
    let job_url_pairs = state.job_url_pairs();
    state.pre_triage_mut().merge_delta(delta, &policy);
    state.pre_triage_mut().bind_job_ids(&job_url_pairs);
    state.startup_inputs.initial_article_window = StartupWindow::LoadedAndResolved;

    state.mark_dirty();
    let effects = super::processing::resume(state);
    if state.pipeline_ready() && state.processing_start.is_none() {
        let included = state.pre_triage().resolved_included_articles();
        super::waves::admit_triage(state, included);
    }
    effects
}

pub(super) fn handle_articles_load_failed(
    state: &mut AppState,
    request_id: u64,
    reason: String,
) -> Vec<Effect> {
    if Some(request_id) != state.triage_in_flight_request_id() {
        engine_info!(
            "[pre-triage-refresh-coord] stale failure ignored request_id={} in_flight={:?}",
            request_id,
            state.triage_in_flight_request_id()
        );
        return Vec::new();
    }
    engine_warn!(
        "[pre-triage-refresh-coord] background refresh failed request_id={} reason={}",
        request_id,
        reason
    );
    state.clear_triage_in_flight();
    state.pre_triage_coordinator.complete_request(request_id);
    super::processing::fail(state, reason.clone());
    // Do NOT fail the TriageSession — a background refresh error should not
    // destroy the user's active triage session.
    state.set_pre_triage(PreTriageSession::default());
    if matches!(
        state.startup_inputs.initial_article_window,
        StartupWindow::Pending | StartupWindow::Failed
    ) {
        state.startup_inputs.initial_article_window = StartupWindow::Failed;
    }
    state.mark_dirty();
    Vec::new()
}

/// Record refresh demand with the coordinator. If the URL list is empty,
/// resets pre-triage immediately (same as before). Otherwise, marks demand
/// as pending — actual dispatch happens on the next eligible `Msg::Tick`.
fn schedule_pre_triage_refresh(
    state: &mut AppState,
    reason: crate::pre_triage_coordinator::PreTriageRefreshReason,
    ordered_urls: Vec<String>,
) -> Vec<Effect> {
    if (state.pipeline_run_phase() == crate::PipelineRunPhase::Stopping
        || state.session() == crate::SessionState::Finishing)
        && reason != crate::pre_triage_coordinator::PreTriageRefreshReason::JobDone
    {
        return Vec::new();
    }
    let tick = state.current_tick();
    state
        .pre_triage_coordinator
        .set_run_active(state.pipeline_run_armed());
    let result = state
        .pre_triage_coordinator
        .schedule_refresh(ordered_urls, reason, tick);

    match result {
        crate::pre_triage_coordinator::PreTriageRefreshScheduleResult::ImmediateReset => {
            engine_info!("[pre-triage-refresh-coord] immediate reset (empty corpus)");
            state.set_pre_triage(PreTriageSession::default());
            state.clear_triage_in_flight();
            Vec::new()
        }
        crate::pre_triage_coordinator::PreTriageRefreshScheduleResult::Scheduled => {
            engine_info!(
                "[pre-triage-refresh-coord] request scheduled reason={:?}",
                reason
            );
            state.set_pre_triage_load_context(reason);
            state.mark_dirty();
            Vec::new()
        }
    }
}

/// Check whether the coordinator wants to dispatch a pre-triage load on this tick.
pub(super) fn dispatch_pre_triage_if_due(
    state: &mut AppState,
    tick: u64,
    has_in_flight_engine_jobs: bool,
) -> Vec<Effect> {
    if state.pipeline_run_armed() && !state.pipeline_ready() {
        return Vec::new();
    }
    let armed = state.pipeline_run_armed();
    state.pre_triage_coordinator.set_run_active(armed);
    let Some(dispatch) = state
        .pre_triage_coordinator
        .maybe_dispatch(tick, has_in_flight_engine_jobs)
    else {
        return Vec::new();
    };

    let request_id = dispatch.request_id;
    let ordered_urls = dispatch.ordered_urls;
    let since_utc = state.briefing_since_utc();

    // Keep Slice 1 in-flight tracker in sync with the coordinator.
    state.set_triage_in_flight(request_id);
    state.mark_dirty();
    engine_info!(
        "[pre-triage-refresh-coord] dispatch request_id={} urls={}",
        request_id,
        ordered_urls.len()
    );
    vec![Effect::LoadArticlesForTriage {
        held: state.pre_triage().held_articles(),
        request_id,
        ordered_urls,
        since_utc,
    }]
}

pub(super) fn start_triage_from_pretriage(state: &mut AppState) -> Vec<Effect> {
    state.reset_provider_rate_limit_failures();
    state.reset_provider_model_dispatch_halt();
    // Consumes the pre-triage articles via a phase-guarded helper that atomically
    // resets pre-triage to Idle, ensuring it cannot remain action-ready after
    // its articles have been handed off to triage.
    let included = match state.consume_interactive_pre_triage_articles_for_triage() {
        Some(articles) => articles,
        None => {
            state
                .triage_mut()
                .fail("no completed articles found".to_string());
            state.mark_dirty();
            return Vec::new();
        }
    };
    engine_info!(
        "[triage] consumed pre-triage for triage start count={}",
        included.len(),
    );
    state.start_triage_cache_run();
    state.mark_triage_metadata_ready();
    super::waves::admit_triage(state, included);
    Vec::new()
}

pub(super) fn settle_triage(state: &mut AppState, effects: &mut Vec<Effect>) {
    // Check if all articles are settled (no pending, no in-progress).
    if state.triage().pending_count() == 0 && state.triage().in_progress_count() == 0 {
        if state.triage().completed_count() == 0 {
            state
                .triage_mut()
                .fail("all triage attempts failed".to_string());
        } else {
            state.triage_mut().complete();
        }
        log_triage_cache_run_summary(state);
        effects.push(Effect::FlushResults);
        state.mark_dirty();
    }
}

pub(super) fn log_triage_cache_run_start_if_needed(state: &mut AppState) {
    if state.triage_cache_run_start_logged() {
        return;
    }
    let metadata = state.triage_cache_metadata();
    if let Some((version, model_id, _)) = metadata {
        engine_info!(
            "[triage-cache] run-start prompt_version={} model_id={}",
            version,
            model_id
        );
        state.mark_triage_cache_run_started();
    }
}

fn log_triage_cache_run_summary(state: &mut AppState) {
    let metrics = state.triage_cache_metrics();
    engine_info!(
        "[triage-cache] run summary hits={} misses={} key_unavailable={} total={}",
        metrics.hits(),
        metrics.misses(),
        metrics.key_unavailable(),
        metrics.total()
    );
    state.finalize_triage_cache_run();
}
