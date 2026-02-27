//! Pre-triage refresh coordinator: reducer-owned quiet-period batching.
//!
//! # Why this module exists
//!
//! `Poll Sources` can complete many jobs in quick succession. Without batching,
//! each `Msg::JobDone` would trigger a full pre-triage rebuild and content-prep
//! pass over the entire corpus, making the feature feel slow even though the
//! actual RSS polling is fast.
//!
//! The coordinator implements a quiet-period scheduling policy:
//! - Refresh demand is recorded (dirty flag + latest URL snapshot).
//! - A single in-flight load request is tracked via a `request_id`.
//! - `Msg::Tick` drives dispatch after a configurable quiet window.
//! - A max-wait guard prevents indefinite postponement in continuous streams.
//! - During a poll burst, dispatch is additionally gated on `jobs_in_flight`.
//!
//! # Scheduling architecture
//!
//! Scheduling policy lives **entirely here** (reducer-owned); `harvester_io`
//! only executes IO and returns results. This makes scheduling deterministic,
//! tick-driven, and unit-testable without real timers or threads.
//!
//! Tick cadence: 75 ms per tick (as emitted by the platform layer).

use std::num::NonZeroU64;

// ── Quiet-period constants ────────────────────────────────────────────────────

/// Ticks to wait after demand is recorded before dispatching (normal case).
/// ~300 ms at 75 ms/tick.
pub(crate) const QUIET_TICKS_NORMAL: u64 = 4;

/// Ticks to wait after demand is recorded during/after a poll burst.
/// ~1200 ms at 75 ms/tick.
pub(crate) const QUIET_TICKS_AFTER_POLL: u64 = 16;

/// Hard ceiling: dispatch even if demand keeps arriving (prevents starvation).
/// ~6 s at 75 ms/tick.
pub(crate) const MAX_WAIT_TICKS: u64 = 80;

// ── Public types ──────────────────────────────────────────────────────────────

/// Why a pre-triage refresh was requested.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum PreTriageRefreshReason {
    JobDone,
    RestoreCompletedJobs,
}

/// Dispatch decision returned by `maybe_dispatch`.
pub(crate) struct PreTriageRefreshDispatch {
    pub request_id: u64,
    pub ordered_urls: Vec<String>,
}

/// Outcome of `schedule_refresh`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum PreTriageRefreshScheduleResult {
    /// Refresh has been queued; caller should set pre-triage to loading state.
    Scheduled,
    /// URL list was empty; caller should reset pre-triage immediately (no effect dispatch).
    ImmediateReset,
}

// ── Coordinator ───────────────────────────────────────────────────────────────

/// State machine that batches pre-triage refresh demand and dispatches at most
/// one `LoadArticlesForTriage` effect per quiet window.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct PreTriageRefreshCoordinator {
    /// True when new demand has arrived since the last dispatch (or initial state).
    dirty: bool,
    /// The URL snapshot to use on next dispatch.
    pending_ordered_urls: Vec<String>,
    /// The request ID currently being processed by the loader, if any.
    in_flight_request_id: Option<NonZeroU64>,
    /// Next value to allocate as a request ID. Starts at 1.
    next_request_id: u64,
    /// Earliest tick at which dispatch is allowed (updated on every `schedule_refresh`).
    earliest_dispatch_tick: u64,
    /// Tick at which the oldest still-pending demand first arrived (max-wait guard).
    demand_started_tick: Option<u64>,

    // Poll-burst tracking.
    poll_burst_active: bool,
    poll_sources_ended: bool,
    last_job_done_tick: Option<u64>,
}

impl PreTriageRefreshCoordinator {
    pub(crate) fn new() -> Self {
        Self {
            dirty: false,
            pending_ordered_urls: Vec::new(),
            in_flight_request_id: None,
            next_request_id: 1,
            earliest_dispatch_tick: 0,
            demand_started_tick: None,
            poll_burst_active: false,
            poll_sources_ended: false,
            last_job_done_tick: None,
        }
    }

    // ── Demand recording ─────────────────────────────────────────────────────

    /// Record refresh demand. Returns `ImmediateReset` if `ordered_urls` is empty.
    pub(crate) fn schedule_refresh(
        &mut self,
        ordered_urls: Vec<String>,
        reason: PreTriageRefreshReason,
        current_tick: u64,
    ) -> PreTriageRefreshScheduleResult {
        if ordered_urls.is_empty() {
            // Empty corpus: caller must reset pre-triage immediately.
            self.dirty = false;
            self.pending_ordered_urls.clear();
            self.demand_started_tick = None;
            return PreTriageRefreshScheduleResult::ImmediateReset;
        }

        let quiet_ticks = if self.poll_burst_active {
            QUIET_TICKS_AFTER_POLL
        } else {
            QUIET_TICKS_NORMAL
        };

        if !self.dirty {
            // First demand in this cycle: record when it started.
            self.demand_started_tick = Some(current_tick);
        }
        self.dirty = true;
        self.pending_ordered_urls = ordered_urls;
        self.earliest_dispatch_tick = current_tick.saturating_add(quiet_ticks);

        if reason == PreTriageRefreshReason::JobDone {
            self.last_job_done_tick = Some(current_tick);
        }

        PreTriageRefreshScheduleResult::Scheduled
    }

    // ── Poll lifecycle ────────────────────────────────────────────────────────

    /// Called when `Msg::PollSourcesClicked` is processed.
    pub(crate) fn note_poll_started(&mut self) {
        self.poll_burst_active = true;
        self.poll_sources_ended = false;
    }

    /// Called when `Msg::AllSourcesPollEnded` is processed.
    pub(crate) fn note_poll_sources_ended(&mut self) {
        self.poll_sources_ended = true;
    }

    // ── Tick-driven dispatch ──────────────────────────────────────────────────

    /// Called on every `Msg::Tick`. Returns `Some(dispatch)` when a load
    /// should be dispatched.
    ///
    /// `has_in_flight_engine_jobs` is `batch_observation().jobs_in_flight > 0`.
    pub(crate) fn maybe_dispatch(
        &mut self,
        current_tick: u64,
        has_in_flight_engine_jobs: bool,
    ) -> Option<PreTriageRefreshDispatch> {
        if !self.dirty {
            return None;
        }
        if self.in_flight_request_id.is_some() {
            // Wait for the in-flight request to complete before dispatching again.
            return None;
        }

        let max_wait_exceeded = self
            .demand_started_tick
            .map(|start| current_tick.saturating_sub(start) >= MAX_WAIT_TICKS)
            .unwrap_or(false);

        // During an active poll burst, block dispatch until:
        //   - poll has ended AND no engine jobs are in flight
        //   - OR max-wait is exceeded (starvation guard)
        if self.poll_burst_active && !max_wait_exceeded {
            if !self.poll_sources_ended {
                return None;
            }
            if has_in_flight_engine_jobs {
                return None;
            }
        }

        if current_tick < self.earliest_dispatch_tick && !max_wait_exceeded {
            return None;
        }

        // Dispatch.
        let request_id_raw = self.next_request_id;
        self.next_request_id += 1;
        let request_id =
            NonZeroU64::new(request_id_raw).expect("next_request_id is always >= 1");

        self.in_flight_request_id = Some(request_id);
        self.dirty = false;
        self.demand_started_tick = None;
        // Reset poll-burst state on dispatch when no more engine jobs remain.
        if !has_in_flight_engine_jobs {
            self.poll_burst_active = false;
            self.poll_sources_ended = false;
        }

        Some(PreTriageRefreshDispatch {
            request_id: request_id_raw,
            ordered_urls: self.pending_ordered_urls.clone(),
        })
    }

    // ── Result handling ───────────────────────────────────────────────────────

    /// Called when a matching `TriageArticlesLoaded` or `TriageArticlesLoadFailed`
    /// is applied. Clears in-flight state and returns `true` if new demand is
    /// already queued (caller may advance to next dispatch cycle immediately).
    pub(crate) fn complete_request(&mut self, request_id: u64) -> bool {
        let matches = self
            .in_flight_request_id
            .map(|id| id.get() == request_id)
            .unwrap_or(false);
        if matches {
            self.in_flight_request_id = None;
        }
        self.dirty
    }

    // ── Test-only accessors ───────────────────────────────────────────────────

    #[cfg(test)]
    pub(crate) fn is_dirty(&self) -> bool {
        self.dirty
    }
}

// ── Unit tests ────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    fn urls(n: usize) -> Vec<String> {
        (0..n)
            .map(|i| format!("https://example.com/{i}"))
            .collect()
    }

    #[test]
    fn empty_urls_returns_immediate_reset() {
        let mut coord = PreTriageRefreshCoordinator::new();
        let result =
            coord.schedule_refresh(vec![], PreTriageRefreshReason::JobDone, 0);
        assert_eq!(result, PreTriageRefreshScheduleResult::ImmediateReset);
        assert!(!coord.is_dirty());
        assert!(coord.maybe_dispatch(10, false).is_none());
    }

    #[test]
    fn single_request_dispatches_after_quiet_window() {
        let mut coord = PreTriageRefreshCoordinator::new();
        coord.schedule_refresh(urls(1), PreTriageRefreshReason::JobDone, 0);

        // Not yet — quiet window not elapsed.
        assert!(coord.maybe_dispatch(0, false).is_none());
        assert!(coord.maybe_dispatch(QUIET_TICKS_NORMAL - 1, false).is_none());

        // Now eligible.
        let dispatch = coord
            .maybe_dispatch(QUIET_TICKS_NORMAL, false)
            .expect("should dispatch after quiet window");
        assert_eq!(dispatch.ordered_urls.len(), 1);
        assert_eq!(dispatch.request_id, 1);
        assert!(!coord.is_dirty());
    }

    #[test]
    fn multiple_requests_coalesced_into_one_dispatch() {
        let mut coord = PreTriageRefreshCoordinator::new();
        coord.schedule_refresh(urls(1), PreTriageRefreshReason::JobDone, 0);
        coord.schedule_refresh(urls(2), PreTriageRefreshReason::JobDone, 1);
        coord.schedule_refresh(urls(3), PreTriageRefreshReason::JobDone, 2);

        // After quiet window, only one dispatch with latest URLs.
        let dispatch = coord
            .maybe_dispatch(QUIET_TICKS_NORMAL + 2, false)
            .expect("should dispatch once");
        assert_eq!(dispatch.ordered_urls.len(), 3);
        assert_eq!(dispatch.request_id, 1);

        // No second dispatch immediately.
        assert!(coord.maybe_dispatch(QUIET_TICKS_NORMAL + 3, false).is_none());
    }

    #[test]
    fn new_demand_while_in_flight_sets_dirty_no_double_dispatch() {
        let mut coord = PreTriageRefreshCoordinator::new();
        coord.schedule_refresh(urls(1), PreTriageRefreshReason::JobDone, 0);
        let dispatch = coord
            .maybe_dispatch(QUIET_TICKS_NORMAL, false)
            .expect("first dispatch");

        // Arrive while in flight.
        coord.schedule_refresh(urls(2), PreTriageRefreshReason::JobDone, QUIET_TICKS_NORMAL + 1);

        // In-flight — no second dispatch yet.
        assert!(
            coord
                .maybe_dispatch(QUIET_TICKS_NORMAL + 1, false)
                .is_none()
        );

        // Complete the in-flight request.
        let has_queued = coord.complete_request(dispatch.request_id);
        assert!(has_queued, "new demand should be queued");

        // Now dispatchable after quiet window.
        let second = coord
            .maybe_dispatch(QUIET_TICKS_NORMAL * 2 + 2, false)
            .expect("second dispatch after response");
        assert_eq!(second.ordered_urls.len(), 2);
        assert_eq!(second.request_id, 2);
    }

    #[test]
    fn max_wait_guard_forces_dispatch_despite_continuous_arrivals() {
        let mut coord = PreTriageRefreshCoordinator::new();
        coord.schedule_refresh(urls(1), PreTriageRefreshReason::JobDone, 0);

        // Keep pushing earliest_dispatch_tick forward by re-scheduling every tick.
        for tick in 1..MAX_WAIT_TICKS {
            coord.schedule_refresh(
                urls(1),
                PreTriageRefreshReason::JobDone,
                tick,
            );
            assert!(
                coord.maybe_dispatch(tick, false).is_none(),
                "should not dispatch before MAX_WAIT_TICKS"
            );
        }

        // At MAX_WAIT_TICKS, dispatch should be forced.
        coord.schedule_refresh(urls(1), PreTriageRefreshReason::JobDone, MAX_WAIT_TICKS);
        let dispatch = coord
            .maybe_dispatch(MAX_WAIT_TICKS, false)
            .expect("max-wait must force dispatch");
        assert_eq!(dispatch.request_id, 1);
    }

    #[test]
    fn poll_burst_blocks_dispatch_until_ended_and_no_engine_jobs() {
        let mut coord = PreTriageRefreshCoordinator::new();
        coord.note_poll_started();
        coord.schedule_refresh(urls(2), PreTriageRefreshReason::JobDone, 0);

        // Poll still active — block even after quiet window.
        assert!(
            coord
                .maybe_dispatch(QUIET_TICKS_AFTER_POLL, true)
                .is_none(),
            "should not dispatch while poll is active"
        );

        // Poll ended but engine jobs still in flight — still blocked.
        coord.note_poll_sources_ended();
        assert!(
            coord
                .maybe_dispatch(QUIET_TICKS_AFTER_POLL, true)
                .is_none(),
            "should not dispatch with engine jobs in flight"
        );

        // Poll ended and no engine jobs — dispatch.
        let dispatch = coord
            .maybe_dispatch(QUIET_TICKS_AFTER_POLL, false)
            .expect("should dispatch when poll ended and no engine jobs");
        assert_eq!(dispatch.ordered_urls.len(), 2);
    }

    #[test]
    fn restore_completed_jobs_schedules_refresh() {
        let mut coord = PreTriageRefreshCoordinator::new();
        let result = coord.schedule_refresh(
            urls(3),
            PreTriageRefreshReason::RestoreCompletedJobs,
            0,
        );
        assert_eq!(result, PreTriageRefreshScheduleResult::Scheduled);
        let dispatch = coord
            .maybe_dispatch(QUIET_TICKS_NORMAL, false)
            .expect("should dispatch after quiet window");
        assert_eq!(dispatch.ordered_urls.len(), 3);
    }
}
