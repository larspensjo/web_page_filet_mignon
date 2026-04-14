use super::summary_cache_support::short_hash;
use crate::pre_triage_filter::{
    ArticleFilterKey, ManualDecision, PreTriagePolicy, PreTriageSession,
};
use crate::state::TriageCacheLookupResult;
use crate::tabs::LeftTab;
use crate::triage::TriageSession;
use crate::{AppState, Effect};
use engine_logging::{engine_info, engine_warn};
use harvester_engine::llm::prompt::PromptId;

pub(super) fn handle_evaluate_pre_triage_refresh(
    state: &mut AppState,
    ordered_urls: Vec<String>,
    triggered_by_job_done: bool,
) -> Vec<Effect> {
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

pub(super) fn handle_triage_clicked(state: &mut AppState) -> Vec<Effect> {
    if state.briefing_orchestration_requested() {
        engine_info!("[briefing-triage] interleave blocked: briefing owns triage");
        return Vec::new();
    }
    if !state.triage_ai_available() {
        state.set_left_tab(LeftTab::TriageResults);
        state.mark_dirty();
        return Vec::new();
    }
    if !state.triage().can_start() {
        return Vec::new();
    }
    if !state.can_start_triage_from_pre_triage() {
        return Vec::new();
    }
    state.set_left_tab(LeftTab::TriageResults);
    if !state.triage_metadata_ready() {
        state.mark_triage_metadata_pending();
        engine_warn!("[triage-cache] metadata not ready; loading metadata before dispatch");
        return vec![Effect::LoadPromptContexts, Effect::LoadLlmMetadata];
    }
    engine_info!("[triage] triage requested");
    start_triage_from_pretriage(state)
}

pub(super) fn handle_articles_loaded(
    state: &mut AppState,
    request_id: u64,
    articles: Vec<crate::briefing::LoadedArticle>,
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
    let url_to_fetched: std::collections::HashMap<String, chrono::DateTime<chrono::Utc>> = articles
        .iter()
        .filter_map(|a| {
            let fu = a.fetched_utc.as_deref()?;
            let dt = chrono::DateTime::parse_from_rfc3339(fu).ok()?;
            Some((a.url.clone(), dt.with_timezone(&chrono::Utc)))
        })
        .collect();
    state.backfill_jobs_fetched_utc(&url_to_fetched);
    let policy = PreTriagePolicy::default();
    let mut pre_triage = PreTriageSession::load_articles(articles, &policy);
    let job_url_pairs = state
        .view()
        .jobs
        .iter()
        .map(|job| (job.job_id, job.url.clone()))
        .collect::<Vec<_>>();
    pre_triage.bind_job_ids(&job_url_pairs);
    pre_triage.apply_manual_overrides(state.pre_triage_manual_overrides());
    state.set_pre_triage(pre_triage);
    state.refresh_selected_preview();
    state.mark_dirty();
    Vec::new()
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
    // Clear manual overrides to avoid stale decisions on the blank pre-triage.
    // Do NOT fail the TriageSession — a background refresh error should not
    // destroy the user's active triage session.
    state.clear_pre_triage_manual_overrides();
    state.set_pre_triage(PreTriageSession::default());
    state.mark_dirty();
    Vec::new()
}

pub(super) fn handle_pre_triage_decision_set(
    state: &mut AppState,
    key: ArticleFilterKey,
    decision: ManualDecision,
) -> Vec<Effect> {
    if state.set_pre_triage_manual_decision(key, decision) {
        state.mark_dirty();
    }
    Vec::new()
}

pub(super) fn handle_pre_triage_apply_clicked(_state: &mut AppState) -> Vec<Effect> {
    Vec::new()
}

pub(super) fn handle_pre_triage_reset_clicked(state: &mut AppState) -> Vec<Effect> {
    state.clear_pre_triage_manual_overrides();
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
    let tick = state.current_tick();
    let result = state
        .pre_triage_coordinator
        .schedule_refresh(ordered_urls, reason, tick);

    match result {
        crate::pre_triage_coordinator::PreTriageRefreshScheduleResult::ImmediateReset => {
            engine_info!("[pre-triage-refresh-coord] immediate reset (empty corpus)");
            // Clear overrides BEFORE resetting the session so that
            // `clear_manual_decisions` (which re-derives phase) runs on the
            // old session, not the freshly-reset one. The final
            // `set_pre_triage(default)` then establishes the correct Idle phase.
            state.clear_pre_triage_manual_overrides();
            state.set_pre_triage(PreTriageSession::default());
            state.clear_triage_in_flight();
            Vec::new()
        }
        crate::pre_triage_coordinator::PreTriageRefreshScheduleResult::Scheduled => {
            engine_info!(
                "[pre-triage-refresh-coord] request scheduled reason={:?}",
                reason
            );
            // Set pre-triage to loading so the UI shows a spinner immediately.
            state.set_pre_triage(PreTriageSession::new_loading());
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
        request_id,
        ordered_urls,
        since_utc,
    }]
}

fn start_triage_from_pretriage(state: &mut AppState) -> Vec<Effect> {
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
    state.set_triage(TriageSession::new_loading(None));
    state.triage_mut().set_articles(included);
    state.triage_mut().transition_to_triaging();
    state.mark_dirty();
    state.start_triage_cache_run();
    state.mark_triage_metadata_ready();
    let mut effects = Vec::new();
    dispatch_next_triage_step(state, &mut effects);
    effects
}

pub(super) fn dispatch_next_triage_step(state: &mut AppState, effects: &mut Vec<Effect>) {
    log_triage_cache_run_start_if_needed(state);
    let limit = state.triage_max_in_flight();

    // Fill available in-flight slots.
    while state.triage().can_dispatch_more(limit) {
        let next_idx = state
            .triage()
            .next_pending_index()
            .expect("can_dispatch_more guarantees pending exists");

        let content_hash = state.triage().articles()[next_idx].content_hash.clone();
        let content_hash_short = short_hash(&content_hash);

        match state.try_reuse_triage(&content_hash) {
            TriageCacheLookupResult::Hit(cached) => {
                let themes = cached.tags.clone();
                let url = state.triage().articles()[next_idx].url.clone();
                let fetched_utc = state.triage().articles()[next_idx].fetched_utc.clone();
                let result = cached.clone();
                state.record_triage_cache_hit();
                engine_info!("[triage-cache] hit content_hash={}", content_hash_short);
                state.triage_mut().complete_article(next_idx, result);
                state.refresh_selected_preview();
                state.mark_dirty();
                effects.push(Effect::UpsertEntityIndexEntry {
                    url,
                    fetched_utc,
                    content_hash: Some(content_hash.clone()),
                    summary_entities: None,
                    themes: Some(themes),
                });
                continue;
            }
            TriageCacheLookupResult::Miss => {
                state.record_triage_cache_miss();
                engine_info!("[triage-cache] miss content_hash={}", content_hash_short);
            }
            TriageCacheLookupResult::KeyUnavailable => {
                state.record_triage_cache_key_unavailable();
                if state.triage_metadata_ready() {
                    engine_warn!(
                        "[triage-cache] key-unavailable despite metadata-ready content_hash={}",
                        content_hash_short
                    );
                } else {
                    engine_info!(
                        "[triage-cache] key-unavailable metadata-pending content_hash={}",
                        content_hash_short
                    );
                }
            }
        }

        let prepared_text = state.triage().articles()[next_idx].prepared_text.clone();
        let request_id = state.allocate_next_llm_request_id();
        state.record_pending_llm_request(request_id, PromptId::ArticleTriage);
        state.triage_mut().start_article(next_idx, request_id);

        let context = state.context_for(PromptId::ArticleTriage).to_vec();

        engine_info!(
            "[llm-concurrency] triage dispatch request_id={} article={} in_flight={} limit={}",
            request_id,
            next_idx,
            state.triage().in_progress_count(),
            limit
        );

        effects.push(Effect::RequestLlmCompletion {
            request_id,
            prompt_id: PromptId::ArticleTriage,
            prompt_version: None,
            model_override: None,
            input_content: prepared_text,
            context,
            template_override: None,
            extra_template_vars: vec![],
        });
        state.mark_dirty();
    }

    // Check if all articles are settled (no pending, no in-progress).
    if state.triage().pending_count() == 0 && state.triage().in_progress_count() == 0 {
        if state.triage().completed_count() == 0 {
            state
                .triage_mut()
                .fail("all triage attempts failed".to_string());
        } else {
            state.triage_mut().complete();
            if state.briefing_orchestration_requested() {
                super::briefing::on_triage_settled_for_briefing(state, effects);
            }
        }
        log_triage_cache_run_summary(state);
        effects.push(Effect::PersistTriageCache {
            cache: state.triage_cache().clone(),
        });
        state.mark_dirty();
    }
}

fn log_triage_cache_run_start_if_needed(state: &mut AppState) {
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
