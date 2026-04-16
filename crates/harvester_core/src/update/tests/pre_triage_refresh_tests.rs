use super::*;

fn count_triage_loads(effects: &[Effect]) -> usize {
    effects
        .iter()
        .filter(|e| matches!(e, Effect::LoadArticlesForTriage { .. }))
        .count()
}

fn extract_triage_load_request_id(effects: &[Effect]) -> Option<u64> {
    effects.iter().find_map(|e| match e {
        Effect::LoadArticlesForTriage { request_id, .. } => Some(*request_id),
        _ => None,
    })
}

/// Advance `n` ticks and collect all emitted effects.
fn advance_ticks(mut state: AppState, n: usize) -> (AppState, Vec<Effect>) {
    let mut all_effects = Vec::new();
    for _ in 0..n {
        let (next, effects) = update(state, Msg::Tick);
        state = next;
        all_effects.extend(effects);
    }
    (state, all_effects)
}

/// Submit a URL and return the job_id WITHOUT completing the job.
fn submit_job_for_test(state: AppState, url: &str) -> (AppState, crate::JobId) {
    let (state, effects) = update(state, Msg::InputChanged(format!("{url}\n")));
    let (state, effects2) = update(state, Msg::UrlsSubmitted);
    let job_id = effects
        .into_iter()
        .chain(effects2)
        .find_map(|e| match e {
            Effect::EnqueueUrl { job_id, .. } => Some(job_id),
            _ => None,
        })
        .expect("EnqueueUrl effect expected");
    (state, job_id)
}

/// Mark a job as successfully completed.
fn complete_job_for_test(state: AppState, job_id: crate::JobId) -> AppState {
    let (state, _) = update(
        state,
        Msg::JobDone {
            job_id,
            result: crate::JobResultKind::Success,
            content_preview: None,
            extracted_links: Vec::new(),
            fetched_utc: None,
        },
    );
    apply_pending_pre_triage_refresh_evaluation(state)
}

#[test]
fn triage_loaded_matching_request_id_applies_articles() {
    init_logging();
    let state = add_completed_job_for_test(AppState::new(), "https://example.com/1");
    // Coordinator now batches demand; advance ticks until dispatch.
    let (state, request_id) = tick_until_dispatch(state);
    let (state, _) = update(
        state,
        Msg::TriageArticlesLoaded {
            request_id,
            articles: loaded_triage_articles(1),
        },
    );
    assert!(
        matches!(
            state.pre_triage().phase(),
            crate::pre_triage_filter::PreTriagePhase::ReadyToTriage
        ),
        "matching result should apply articles"
    );
    assert!(
        state.triage_in_flight_request_id().is_none(),
        "in-flight id must be cleared after apply"
    );
}

#[test]
fn triage_loaded_stale_request_id_is_ignored() {
    init_logging();
    let state = add_completed_job_for_test(AppState::new(), "https://example.com/1");
    // Advance ticks until dispatch so there IS an in-flight request.
    let (state, _real_request_id) = tick_until_dispatch(state);
    let stale_id = 999u64;
    let pre_triage_phase_before = state.pre_triage().phase().clone();
    let (state, _) = update(
        state,
        Msg::TriageArticlesLoaded {
            request_id: stale_id,
            articles: loaded_triage_articles(1),
        },
    );
    assert_eq!(
        state.pre_triage().phase(),
        pre_triage_phase_before,
        "stale result must not mutate pre-triage state"
    );
    assert!(
        state.triage_in_flight_request_id().is_some(),
        "in-flight id must remain set after stale result"
    );
}

#[test]
fn triage_load_failed_matching_clears_manual_overrides_not_triage_session() {
    init_logging();
    let state = add_completed_job_for_test(AppState::new(), "https://example.com/1");
    let (state, request_id) = tick_until_dispatch(state);
    let (state, _) = update(
        state,
        Msg::TriageArticlesLoadFailed {
            request_id,
            reason: "disk error".to_string(),
        },
    );
    assert!(
        state.triage_in_flight_request_id().is_none(),
        "in-flight id must be cleared after failure"
    );
    // Triage session should NOT be in Failed state — background refresh errors
    // must not destroy the user's active triage session.
    assert!(
        !matches!(
            state.triage().phase(),
            crate::triage::TriagePhase::Failed { .. }
        ),
        "background refresh failure must not fail the triage session"
    );
}

#[test]
fn triage_load_failed_stale_request_id_is_ignored() {
    init_logging();
    let state = add_completed_job_for_test(AppState::new(), "https://example.com/1");
    let (state, _real_request_id) = tick_until_dispatch(state);
    let stale_id = 888u64;
    let in_flight_before = state.triage_in_flight_request_id();
    let (state, _) = update(
        state,
        Msg::TriageArticlesLoadFailed {
            request_id: stale_id,
            reason: "boom".to_string(),
        },
    );
    assert_eq!(
        state.triage_in_flight_request_id(),
        in_flight_before,
        "stale failure must not clear in-flight request id"
    );
}

#[test]
fn triage_articles_load_progress_updates_matching_request() {
    init_logging();
    let state = add_completed_job_for_test(AppState::new(), "https://example.com/1");
    let (state, request_id) = tick_until_dispatch(state);

    let (state, _) = update(
        state,
        Msg::TriageArticlesLoadProgress {
            request_id,
            files_scanned: 25,
            files_total: 80,
            matched_urls: 1,
        },
    );

    assert_eq!(
        state.view().operation_progress,
        Some(crate::view_model::OperationProgress {
            label: "Refreshing triage set (1 saved)".to_string(),
            completed: 25,
            total: 80,
        })
    );
}

#[test]
fn triage_articles_load_progress_ignores_stale_request() {
    init_logging();
    let state = add_completed_job_for_test(AppState::new(), "https://example.com/1");
    let (state, _request_id) = tick_until_dispatch(state);

    let (state, _) = update(
        state,
        Msg::TriageArticlesLoadProgress {
            request_id: 999,
            files_scanned: 25,
            files_total: 80,
            matched_urls: 1,
        },
    );

    assert_eq!(
        state.view().operation_progress,
        Some(crate::view_model::OperationProgress {
            label: "Refreshing triage set (1 saved)".to_string(),
            completed: 0,
            total: 1,
        })
    );
}

#[test]
fn triage_articles_load_progress_cleared_on_success() {
    init_logging();
    let state = add_completed_job_for_test(AppState::new(), "https://example.com/1");
    let (state, request_id) = tick_until_dispatch(state);
    let (state, _) = update(
        state,
        Msg::TriageArticlesLoadProgress {
            request_id,
            files_scanned: 25,
            files_total: 80,
            matched_urls: 1,
        },
    );

    let (state, _) = update(
        state,
        Msg::TriageArticlesLoaded {
            request_id,
            articles: loaded_triage_articles(1),
        },
    );

    assert!(state.view().operation_progress.is_none());
}

#[test]
fn triage_articles_load_progress_cleared_on_failure() {
    init_logging();
    let state = add_completed_job_for_test(AppState::new(), "https://example.com/1");
    let (state, request_id) = tick_until_dispatch(state);
    let (state, _) = update(
        state,
        Msg::TriageArticlesLoadProgress {
            request_id,
            files_scanned: 25,
            files_total: 80,
            matched_urls: 1,
        },
    );

    let (state, _) = update(
        state,
        Msg::TriageArticlesLoadFailed {
            request_id,
            reason: "boom".to_string(),
        },
    );

    assert!(state.view().operation_progress.is_none());
}

#[test]
fn multiple_job_dones_within_quiet_window_emit_exactly_one_triage_load() {
    init_logging();
    let mut state = AppState::new();
    // Complete 3 jobs in quick succession (all within quiet window of tick 0).
    state = add_completed_job_for_test(state, "https://example.com/1");
    state = add_completed_job_for_test(state, "https://example.com/2");
    state = add_completed_job_for_test(state, "https://example.com/3");

    // Advance past the quiet window.
    let (_state, effects) = advance_ticks(
        state,
        crate::pre_triage_coordinator::QUIET_TICKS_NORMAL as usize + 1,
    );
    assert_eq!(
        count_triage_loads(&effects),
        1,
        "burst of 3 JobDones must yield exactly one triage load"
    );
    // Verify a valid request_id was emitted; the exact value depends on prior
    // allocations and is an implementation detail of the coordinator counter.
    assert!(
        extract_triage_load_request_id(&effects).is_some(),
        "first dispatch must emit a triage load with a request id"
    );
}

#[test]
fn restore_completed_jobs_schedules_and_dispatches_after_quiet_window() {
    init_logging();
    let state = add_completed_job_for_test(AppState::new(), "https://example.com/1");
    // Drain first dispatch so we start fresh.
    let (state, request_id) = tick_until_dispatch(state);
    let (state, _) = update(
        state,
        Msg::TriageArticlesLoaded {
            request_id,
            articles: loaded_triage_articles(1),
        },
    );

    // Now simulate a RestoreCompletedJobs (e.g., after restart).
    let snapshot = state.completed_jobs_snapshot();
    let (state, effects) = update(state, Msg::RestoreCompletedJobs(snapshot));
    let state = apply_pending_pre_triage_refresh_evaluation(state);
    assert_eq!(
        count_triage_loads(&effects),
        0,
        "RestoreCompletedJobs must not dispatch immediately"
    );
    let (state, effects) = advance_ticks(
        state,
        crate::pre_triage_coordinator::QUIET_TICKS_NORMAL as usize + 1,
    );
    assert_eq!(
        count_triage_loads(&effects),
        1,
        "must dispatch exactly one triage load after quiet window"
    );
    let _ = state;
}

#[test]
fn restore_completed_jobs_loading_text_explains_startup_preparation() {
    init_logging();
    let snapshot = vec![crate::CompletedJobSnapshot {
        url: "https://example.com/restored".to_string(),
        tokens: None,
        bytes: None,
        links: Vec::new(),
        fetched_utc: None,
    }];

    let (state, _) = update(AppState::new(), Msg::RestoreCompletedJobs(snapshot));
    let state = apply_pending_pre_triage_refresh_evaluation(state);

    let view = state.view();
    assert_eq!(
        view.triage_progress,
        Some("Preparing triage set from 1 saved article...".to_string())
    );
    assert_eq!(
        view.triage_blocked_reason,
        Some("Triage is unavailable while startup prepares the article set".to_string())
    );
}

#[test]
fn job_done_loading_text_explains_refresh_preparation() {
    init_logging();
    let state = add_completed_job_for_test(AppState::new(), "https://example.com/1");

    let view = state.view();
    assert_eq!(
        view.triage_progress,
        Some("Refreshing triage set from 1 saved article...".to_string())
    );
    assert_eq!(
        view.operation_progress,
        Some(crate::view_model::OperationProgress {
            label: "Refreshing triage set (1 saved)".to_string(),
            completed: 0,
            total: 1,
        })
    );
}

#[test]
fn new_demand_while_in_flight_queues_and_dispatches_after_response() {
    init_logging();
    let state = add_completed_job_for_test(AppState::new(), "https://example.com/1");
    let (state, first_request_id) = tick_until_dispatch(state);

    // New demand arrives while in-flight.
    let state = add_completed_job_for_test(state, "https://example.com/2");

    // No second dispatch while in-flight.
    let (state, effects) = advance_ticks(
        state,
        crate::pre_triage_coordinator::QUIET_TICKS_NORMAL as usize + 5,
    );
    assert_eq!(
        count_triage_loads(&effects),
        0,
        "must not dispatch while prior request is in flight"
    );

    // Complete the in-flight request.
    let (state, _) = update(
        state,
        Msg::TriageArticlesLoaded {
            request_id: first_request_id,
            articles: loaded_triage_articles(1),
        },
    );

    // Now the queued demand should dispatch.
    let (state, request_id) = tick_until_dispatch(state);
    assert_ne!(
        request_id, first_request_id,
        "second dispatch must use a distinct request id"
    );
    let _ = state;
}

#[test]
fn empty_corpus_after_job_done_triggers_immediate_reset_no_loader_effect() {
    init_logging();
    // No jobs → corpus is empty → ImmediateReset path.
    let state = AppState::new();
    // Simulate a JobDone for a URL that was never submitted (not tracked in state),
    // which means ordered_completed_job_urls() returns empty.
    // Since AppState::new() has no jobs, RestoreCompletedJobs with empty snapshot
    // is the cleanest way to test this path.
    let (state, effects) = update(state, Msg::RestoreCompletedJobs(vec![]));
    let state = apply_pending_pre_triage_refresh_evaluation(state);
    assert_eq!(
        count_triage_loads(&effects),
        0,
        "empty corpus must not dispatch a loader effect"
    );
    // Pre-triage should be reset to default (Idle).
    assert!(
        matches!(
            state.pre_triage().phase(),
            crate::pre_triage_filter::PreTriagePhase::Idle
        ),
        "empty corpus must reset pre-triage to Idle"
    );
}

/// Core poll sequence: multiple JobDones during a burst yield exactly one
/// post-burst triage load after AllSourcesPollEnded.
#[test]
fn poll_burst_multiple_job_dones_yields_exactly_one_triage_load() {
    use crate::pre_triage_coordinator::{QUIET_TICKS_AFTER_POLL, QUIET_TICKS_NORMAL};
    init_logging();

    let state = AppState::new();
    let (state, _) = update(state, Msg::PollSourcesClicked);

    // Complete 3 jobs during the burst (immediately done, so jobs_in_flight=0 between calls).
    let state = add_completed_job_for_test(state, "https://example.com/1");
    let state = add_completed_job_for_test(state, "https://example.com/2");
    let state = add_completed_job_for_test(state, "https://example.com/3");

    // Tick through the after-poll quiet window — poll still active, no dispatch.
    let (state, effects_during_burst) = advance_ticks(state, QUIET_TICKS_AFTER_POLL as usize);
    assert_eq!(
        count_triage_loads(&effects_during_burst),
        0,
        "must not dispatch while poll burst is active (poll_sources_ended=false)"
    );

    // End the poll burst.
    let (state, _) = update(state, Msg::AllSourcesPollEnded);

    // Tick until the single post-burst dispatch.
    let (state, _request_id) = tick_until_dispatch(state);

    // No second dispatch in the next few ticks.
    let (state, extra) = advance_ticks(state, QUIET_TICKS_NORMAL as usize + 5);
    assert_eq!(
        count_triage_loads(&extra),
        0,
        "no second dispatch immediately after first"
    );
    let _ = state;
}

/// Dispatch must wait until engine jobs are no longer in flight, even after
/// AllSourcesPollEnded.
#[test]
fn poll_burst_waits_for_engine_jobs_to_drain_before_dispatching() {
    use crate::pre_triage_coordinator::QUIET_TICKS_AFTER_POLL;
    init_logging();

    // Submit two URLs but do not complete them yet — both jobs in flight.
    let state = AppState::new();
    let (state, job_id1) = submit_job_for_test(state, "https://example.com/1");
    let (state, job_id2) = submit_job_for_test(state, "https://example.com/2");
    assert_eq!(
        state.batch_observation().jobs_in_flight,
        2,
        "both jobs should be in flight"
    );

    // Start the poll burst.
    let (state, _) = update(state, Msg::PollSourcesClicked);

    // Complete job1 — demand is scheduled, job2 still in flight.
    let state = complete_job_for_test(state, job_id1);
    assert_eq!(
        state.batch_observation().jobs_in_flight,
        1,
        "job2 should still be in flight"
    );

    // End poll sources — engine job2 still running.
    let (state, _) = update(state, Msg::AllSourcesPollEnded);

    // Tick past quiet window — must NOT dispatch because job2 is still in flight.
    let (state, effects) = advance_ticks(state, QUIET_TICKS_AFTER_POLL as usize + 5);
    assert_eq!(
        count_triage_loads(&effects),
        0,
        "must not dispatch while an engine job is still in flight"
    );

    // Complete job2 — demand re-recorded, jobs_in_flight now 0.
    let state = complete_job_for_test(state, job_id2);
    assert_eq!(
        state.batch_observation().jobs_in_flight,
        0,
        "all engine jobs should be done"
    );

    // Tick until dispatch — should fire exactly one triage load now.
    // A dispatch must occur; the exact request_id value is an implementation detail.
    let (_state, _request_id) = tick_until_dispatch(state);
}

/// Poll burst with no completed jobs must not dispatch any triage load.
#[test]
fn poll_burst_zero_urls_no_triage_load_dispatched() {
    use crate::pre_triage_coordinator::QUIET_TICKS_AFTER_POLL;
    init_logging();

    let state = AppState::new();
    let (state, _) = update(state, Msg::PollSourcesClicked);
    let (state, _) = update(state, Msg::AllSourcesPollEnded);

    // No jobs → no demand → no dispatch.
    let (state, effects) = advance_ticks(state, QUIET_TICKS_AFTER_POLL as usize + 20);
    assert_eq!(
        count_triage_loads(&effects),
        0,
        "poll with no completed jobs must not dispatch a triage load"
    );
    let _ = state;
}

/// Non-poll single JobDone must use the normal (shorter) quiet window and
/// not be gated by poll-burst logic.
#[test]
fn non_poll_single_job_done_dispatches_with_normal_quiet_window() {
    use crate::pre_triage_coordinator::QUIET_TICKS_NORMAL;
    init_logging();

    let state = AppState::new();
    let state = add_completed_job_for_test(state, "https://example.com/1");

    // Must not dispatch before the normal quiet window elapses.
    let (state, effects_before) = advance_ticks(state, QUIET_TICKS_NORMAL as usize - 1);
    assert_eq!(
        count_triage_loads(&effects_before),
        0,
        "must not dispatch before normal quiet window"
    );

    // Should dispatch at or after QUIET_TICKS_NORMAL.
    // A dispatch must occur after the quiet window; exact request_id is an
    // implementation detail of the coordinator counter.
    let (_state, _request_id) = tick_until_dispatch(state);
}
