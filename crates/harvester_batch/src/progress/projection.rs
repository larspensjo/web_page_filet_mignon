use chrono::{DateTime, FixedOffset, Local};
use harvester_core::{BatchObservation, StageKind};
use openai_provider_kit::BatchLifecycle;
use std::time::{Duration, Instant};

use crate::batch_coordinator::BatchPeek;

/// Cumulative reducer counts captured immediately before this process starts
/// its single source-intake pass.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BatchRunBaseline {
    pub jobs_total: usize,
    pub jobs_done: usize,
    pub jobs_failed: usize,
}

impl BatchRunBaseline {
    pub fn from_observation(observation: &BatchObservation) -> Self {
        Self {
            jobs_total: observation.jobs_total,
            jobs_done: observation.jobs_done,
            jobs_failed: observation.jobs_failed,
        }
    }
}

/// One stage's local state plus the matching provider scope, if any.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct StageProgress {
    pub total: usize,
    pub successful: usize,
    pub failed: usize,
    pub pending_or_in_flight: usize,
    pub deferred: usize,
    pub provider_total: usize,
    pub provider_completed: usize,
    pub provisional_settled: usize,
    pub local_remaining: usize,
    pub unsubmitted: usize,
}

impl StageProgress {
    pub fn settled(self) -> usize {
        self.successful.saturating_add(self.failed)
    }
}

/// The renderer vocabulary. Runner integration will explicitly select the
/// lifecycle-specific variants in Phase 4; the projection derives pipeline
/// phases whenever no explicit lifecycle phase is supplied.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BatchDisplayPhase {
    Reconciling,
    Intake,
    Triage,
    Summaries,
    Signals,
    PreparingBatch,
    CheckingProvider,
    WaitingForProvider,
    Collecting,
    Replaying,
    Persisting,
    Complete,
    Interrupted,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProviderLifecycle {
    NoRemoteBatch,
    InProgress,
    ReadyToCollect,
    Indeterminate,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct ProviderStageProgress {
    pub submitted: usize,
    pub completed: usize,
    pub failed: usize,
    pub attached_batches: usize,
    pub terminal_batches: usize,
    pub indeterminate_batches: usize,
}

impl ProviderStageProgress {
    fn lifecycle(self) -> ProviderLifecycle {
        if self.indeterminate_batches > 0 {
            ProviderLifecycle::Indeterminate
        } else if self.attached_batches == 0 {
            ProviderLifecycle::NoRemoteBatch
        } else if self.terminal_batches > 0
            || self.completed.saturating_add(self.failed) >= self.submitted
        {
            ProviderLifecycle::ReadyToCollect
        } else {
            ProviderLifecycle::InProgress
        }
    }
}

/// Provider request counts grouped by the typed pipeline stage.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct ProviderProgress {
    pub triage: ProviderStageProgress,
    pub summaries: ProviderStageProgress,
    pub signals: ProviderStageProgress,
}

impl ProviderProgress {
    pub fn from_peeks(peeks: &[BatchPeek]) -> Self {
        let mut progress = Self::default();
        for peek in peeks {
            let stage = progress.stage_mut(peek.stage);
            stage.attached_batches = stage.attached_batches.saturating_add(1);
            if peek
                .status
                .as_ref()
                .is_some_and(is_terminal_provider_lifecycle)
            {
                stage.terminal_batches = stage.terminal_batches.saturating_add(1);
            }
            match &peek.request_counts {
                Some(counts) => {
                    stage.submitted = stage.submitted.saturating_add(counts.total as usize);
                    stage.completed = stage.completed.saturating_add(counts.completed as usize);
                    stage.failed = stage.failed.saturating_add(counts.failed as usize);
                }
                None => stage.indeterminate_batches = stage.indeterminate_batches.saturating_add(1),
            }
        }
        progress
    }

    pub fn stage(self, stage: StageKind) -> ProviderStageProgress {
        match stage {
            StageKind::Triage => self.triage,
            StageKind::Summary => self.summaries,
            StageKind::SignalCandidate => self.signals,
        }
    }

    pub fn lifecycle(self, stage: StageKind) -> ProviderLifecycle {
        self.stage(stage).lifecycle()
    }

    fn stage_mut(&mut self, stage: StageKind) -> &mut ProviderStageProgress {
        match stage {
            StageKind::Triage => &mut self.triage,
            StageKind::Summary => &mut self.summaries,
            StageKind::SignalCandidate => &mut self.signals,
        }
    }
}

fn is_terminal_provider_lifecycle(lifecycle: &BatchLifecycle) -> bool {
    matches!(
        lifecycle,
        BatchLifecycle::Completed
            | BatchLifecycle::Failed
            | BatchLifecycle::Expired
            | BatchLifecycle::Cancelled
    )
}

/// Wait timing supplied by the runner. Relative values are monotonic; local
/// values are presentation-only and retain their numeric UTC offset.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WaitProgress {
    pub last_provider_check: Option<Instant>,
    pub next_provider_check: Option<Instant>,
    pub checked_age: Option<Duration>,
    pub countdown: Option<Duration>,
    pub last_provider_check_local: Option<DateTime<FixedOffset>>,
    pub next_provider_check_local: Option<DateTime<FixedOffset>>,
    pub last_provider_check_display: Option<String>,
    pub next_provider_check_display: Option<String>,
}

/// Runner clock seam for elapsed time, wait scheduling, and local presentation.
pub trait ProgressClock {
    fn monotonic_now(&self) -> Instant;
    fn wall_now(&self) -> DateTime<FixedOffset>;
    fn sleep(&self, duration: Duration);
}

pub struct SystemProgressClock;

impl ProgressClock for SystemProgressClock {
    fn monotonic_now(&self) -> Instant {
        Instant::now()
    }

    fn wall_now(&self) -> DateTime<FixedOffset> {
        Local::now().fixed_offset()
    }

    fn sleep(&self, duration: Duration) {
        std::thread::sleep(duration);
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct PassCounts {
    pub intake_passes: usize,
    pub collection_passes: usize,
    pub replay_passes: usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct IntakeProgress {
    pub discovered: usize,
    pub fetched: usize,
    pub failed: usize,
    pub total: usize,
}

#[derive(Debug, Clone, Copy)]
struct LocalStageCounts {
    total: usize,
    successful: usize,
    failed: usize,
    pending_or_in_flight: usize,
    deferred: usize,
}

/// Complete pure input for a future terminal or append-only renderer.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BatchProgressSnapshot {
    pub elapsed: Duration,
    pub cost_this_run_microdollars: u64,
    pub intake: IntakeProgress,
    pub triage: StageProgress,
    pub summaries: StageProgress,
    pub signals: StageProgress,
    pub provider: ProviderProgress,
    pub phase: BatchDisplayPhase,
    pub remaining_work: usize,
    pub pass_counts: PassCounts,
    pub wait: Option<WaitProgress>,
}

/// Explicit facts supplied by the runner without reaching into reducer state.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct ProjectionContext {
    pub phase_override: Option<BatchDisplayPhase>,
    pub pass_counts: PassCounts,
    pub cost_this_run_microdollars: u64,
    pub last_provider_check: Option<Instant>,
    pub next_provider_check: Option<Instant>,
    pub last_provider_check_local: Option<DateTime<FixedOffset>>,
    pub next_provider_check_local: Option<DateTime<FixedOffset>>,
}

/// Pure, runner-owned projection state. Its only retained data is the
/// invocation-scoped denominator/intake latches; it performs no I/O and never
/// mutates reducer state.
pub struct BatchProgressProjection {
    baseline: BatchRunBaseline,
    started_at: Instant,
    intake_total: Option<usize>,
    triage_total: usize,
    summary_total: usize,
    signal_total: usize,
}

impl BatchProgressProjection {
    pub fn new(baseline: BatchRunBaseline, started_at: Instant) -> Self {
        Self {
            baseline,
            started_at,
            intake_total: None,
            triage_total: 0,
            summary_total: 0,
            signal_total: 0,
        }
    }

    pub fn snapshot<C: ProgressClock>(
        &mut self,
        observation: &BatchObservation,
        peeks: &[BatchPeek],
        context: ProjectionContext,
        clock: &C,
    ) -> BatchProgressSnapshot {
        let now = clock.monotonic_now();
        let discovered = observation
            .jobs_total
            .saturating_sub(self.baseline.jobs_total);
        let fetched = observation
            .jobs_done
            .saturating_sub(self.baseline.jobs_done);
        let failed = observation
            .jobs_failed
            .saturating_sub(self.baseline.jobs_failed);
        if !observation.poll_in_progress {
            self.intake_total.get_or_insert(discovered);
        }
        let intake = IntakeProgress {
            discovered,
            fetched,
            failed,
            total: self.intake_total.unwrap_or(discovered),
        };
        let provider = ProviderProgress::from_peeks(peeks);
        let triage = self.stage_progress(
            StageKind::Triage,
            LocalStageCounts {
                total: observation.triage_total,
                successful: observation.triage_completed,
                failed: observation.triage_failed,
                pending_or_in_flight: observation
                    .triage_pending
                    .saturating_add(observation.triage_in_flight),
                deferred: observation.triage_deferred,
            },
            provider.stage(StageKind::Triage),
        );
        let summaries = self.stage_progress(
            StageKind::Summary,
            LocalStageCounts {
                total: observation.summary_total,
                successful: observation.summary_completed,
                failed: observation.summary_failed,
                pending_or_in_flight: observation
                    .summary_pending
                    .saturating_add(observation.summary_in_flight),
                deferred: observation.summary_deferred,
            },
            provider.stage(StageKind::Summary),
        );
        let signals = self.stage_progress(
            StageKind::SignalCandidate,
            LocalStageCounts {
                total: observation.signal_total,
                successful: observation.signal_completed,
                failed: observation.signal_failed,
                pending_or_in_flight: observation.signal_pending_or_in_flight,
                deferred: observation.signal_deferred,
            },
            provider.stage(StageKind::SignalCandidate),
        );
        let phase = classify_display_phase(observation, &provider, context.phase_override);
        let wait = wait_progress(&context, now);
        BatchProgressSnapshot {
            elapsed: now.saturating_duration_since(self.started_at),
            cost_this_run_microdollars: context.cost_this_run_microdollars,
            intake,
            triage,
            summaries,
            signals,
            provider,
            phase,
            remaining_work: triage
                .local_remaining
                .saturating_add(summaries.local_remaining)
                .saturating_add(signals.local_remaining),
            pass_counts: context.pass_counts,
            wait,
        }
    }

    fn stage_progress(
        &mut self,
        stage: StageKind,
        local: LocalStageCounts,
        provider: ProviderStageProgress,
    ) -> StageProgress {
        let latched_total = match stage {
            StageKind::Triage => {
                self.triage_total = self.triage_total.max(local.total);
                self.triage_total
            }
            StageKind::Summary => {
                self.summary_total = self.summary_total.max(local.total);
                self.summary_total
            }
            StageKind::SignalCandidate => {
                self.signal_total = self.signal_total.max(local.total);
                self.signal_total
            }
        };
        let total = latched_total.max(provider.submitted);
        let local_settled = local.successful.saturating_add(local.failed);
        StageProgress {
            total,
            successful: local.successful,
            failed: local.failed,
            pending_or_in_flight: local.pending_or_in_flight,
            deferred: local.deferred,
            provider_total: provider.submitted,
            provider_completed: provider.completed,
            provisional_settled: local_settled.saturating_add(provider.completed).min(total),
            local_remaining: local.pending_or_in_flight.saturating_add(local.deferred),
            unsubmitted: local.deferred.saturating_sub(provider.submitted),
        }
    }
}

/// Formats an operator-facing absolute timestamp without changing durable UTC
/// timestamps elsewhere in the runner.
pub fn format_local_timestamp(timestamp: DateTime<FixedOffset>) -> String {
    timestamp.format("%Y-%m-%d %H:%M:%S %:z").to_string()
}

fn wait_progress(context: &ProjectionContext, now: Instant) -> Option<WaitProgress> {
    let has_wait = context.last_provider_check.is_some()
        || context.next_provider_check.is_some()
        || context.last_provider_check_local.is_some()
        || context.next_provider_check_local.is_some();
    has_wait.then(|| WaitProgress {
        last_provider_check: context.last_provider_check,
        next_provider_check: context.next_provider_check,
        checked_age: context
            .last_provider_check
            .map(|last| now.saturating_duration_since(last)),
        countdown: context
            .next_provider_check
            .map(|next| next.saturating_duration_since(now)),
        last_provider_check_local: context.last_provider_check_local,
        next_provider_check_local: context.next_provider_check_local,
        last_provider_check_display: context
            .last_provider_check_local
            .map(format_local_timestamp),
        next_provider_check_display: context
            .next_provider_check_local
            .map(format_local_timestamp),
    })
}

/// Classifies one active phase from reducer/provider state. Signal work is
/// intentionally considered before the terminal fallback.
pub fn classify_display_phase(
    observation: &BatchObservation,
    provider: &ProviderProgress,
    phase_override: Option<BatchDisplayPhase>,
) -> BatchDisplayPhase {
    if let Some(phase) = phase_override {
        return phase;
    }
    if observation.poll_in_progress
        || (observation.jobs_total > 0
            && observation
                .jobs_done
                .saturating_add(observation.jobs_failed)
                < observation.jobs_total)
    {
        return BatchDisplayPhase::Intake;
    }
    for (stage, pending, deferred) in [
        (
            StageKind::Triage,
            observation
                .triage_pending
                .saturating_add(observation.triage_in_flight),
            observation.triage_deferred,
        ),
        (
            StageKind::Summary,
            observation
                .summary_pending
                .saturating_add(observation.summary_in_flight),
            observation.summary_deferred,
        ),
        (
            StageKind::SignalCandidate,
            observation.signal_pending_or_in_flight,
            observation.signal_deferred,
        ),
    ] {
        if pending > 0 {
            return stage_display_phase(stage);
        }
        if deferred > 0 {
            return match provider.lifecycle(stage) {
                ProviderLifecycle::NoRemoteBatch => BatchDisplayPhase::PreparingBatch,
                ProviderLifecycle::ReadyToCollect => BatchDisplayPhase::Collecting,
                ProviderLifecycle::InProgress | ProviderLifecycle::Indeterminate => {
                    BatchDisplayPhase::WaitingForProvider
                }
            };
        }
    }
    BatchDisplayPhase::Complete
}

fn stage_display_phase(stage: StageKind) -> BatchDisplayPhase {
    match stage {
        StageKind::Triage => BatchDisplayPhase::Triage,
        StageKind::Summary => BatchDisplayPhase::Summaries,
        StageKind::SignalCandidate => BatchDisplayPhase::Signals,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::{DateTime, FixedOffset, TimeZone};
    use harvester_core::{
        BatchObservation, ImportPhase, PreTriagePhase, SessionState, TriagePhase,
    };
    use openai_provider_kit::{BatchLifecycle, BatchRequestCounts};
    use std::cell::{Cell, RefCell};
    use std::time::Duration;

    fn import_obs_idle() -> BatchObservation {
        BatchObservation {
            poll_in_progress: false,
            session_state: SessionState::Idle,
            jobs_total: 0,
            jobs_done: 0,
            jobs_failed: 0,
            jobs_in_flight: 0,
            pre_triage_phase: PreTriagePhase::Idle,
            pre_triage_total: 0,
            pre_triage_included: 0,
            pre_triage_review: 0,
            pre_triage_filtered: 0,
            triage_phase: TriagePhase::Idle,
            triage_total: 0,
            triage_pending: 0,
            triage_in_flight: 0,
            triage_completed: 0,
            triage_failed: 0,
            summary_total: 0,
            summary_pending: 0,
            summary_in_flight: 0,
            summary_completed: 0,
            summary_failed: 0,
            triage_deferred: 0,
            summary_deferred: 0,
            signal_total: 0,
            signal_pending_or_in_flight: 0,
            signal_completed: 0,
            signal_failed: 0,
            signal_deferred: 0,
            triage_cache_hits: 0,
            triage_cache_misses: 0,
            triage_cache_key_unavailable: 0,
            summary_cache_hits: 0,
            summary_cache_misses: 0,
            summary_cache_key_unavailable: 0,
            import_phase: ImportPhase::Idle,
            imports_completed: 0,
            imports_failed: 0,
            import_in_flight: false,
            source_poll_stats: vec![],
        }
    }

    struct ManualProgressClock {
        start: Instant,
        elapsed: Cell<Duration>,
        wall: RefCell<DateTime<FixedOffset>>,
    }

    impl ManualProgressClock {
        fn new(wall: DateTime<FixedOffset>) -> Self {
            Self {
                start: Instant::now(),
                elapsed: Cell::new(Duration::ZERO),
                wall: RefCell::new(wall),
            }
        }

        fn advance(&self, duration: Duration) {
            self.elapsed
                .set(self.elapsed.get().saturating_add(duration));
            let wall = *self.wall.borrow();
            *self.wall.borrow_mut() = wall + chrono::Duration::from_std(duration).unwrap();
        }
    }

    impl ProgressClock for ManualProgressClock {
        fn monotonic_now(&self) -> Instant {
            self.start + self.elapsed.get()
        }

        fn wall_now(&self) -> DateTime<FixedOffset> {
            *self.wall.borrow()
        }

        fn sleep(&self, duration: Duration) {
            self.advance(duration);
        }
    }

    fn projection(
        clock: &ManualProgressClock,
        observation: &BatchObservation,
    ) -> BatchProgressProjection {
        BatchProgressProjection::new(
            BatchRunBaseline::from_observation(observation),
            clock.monotonic_now(),
        )
    }

    fn peek(
        stage: StageKind,
        batch_id: &str,
        status: Option<BatchLifecycle>,
        completed: u32,
        total: u32,
    ) -> BatchPeek {
        BatchPeek {
            batch_id: batch_id.into(),
            stage,
            status: status.clone(),
            request_counts: status.map(|_| BatchRequestCounts {
                total,
                completed,
                failed: 0,
            }),
        }
    }

    fn snapshot(
        projection: &mut BatchProgressProjection,
        observation: &BatchObservation,
        peeks: &[BatchPeek],
        clock: &ManualProgressClock,
    ) -> BatchProgressSnapshot {
        projection.snapshot(observation, peeks, ProjectionContext::default(), clock)
    }

    #[test]
    fn projection_uses_current_run_intake_deltas_and_freezes_total() {
        let mut initial = import_obs_idle();
        initial.jobs_total = 7_225;
        initial.jobs_done = 7_218;
        let clock = ManualProgressClock::new(
            FixedOffset::east_opt(2 * 3600)
                .unwrap()
                .with_ymd_and_hms(2026, 7, 23, 9, 48, 30)
                .unwrap(),
        );
        let mut progress = projection(&clock, &initial);
        let mut observed = initial.clone();
        observed.jobs_total += 76;
        observed.jobs_done += 69;
        observed.jobs_failed += 7;
        let first = snapshot(&mut progress, &observed, &[], &clock);
        assert_eq!(first.intake.discovered, 76);
        assert_eq!(first.intake.fetched, 69);
        assert_eq!(first.intake.failed, 7);
        assert_eq!(first.intake.total, 76);

        observed.jobs_total += 2;
        let after_settlement = snapshot(&mut progress, &observed, &[], &clock);
        assert_eq!(after_settlement.intake.discovered, 78);
        assert_eq!(after_settlement.intake.total, 76);
    }

    #[test]
    fn signal_work_selects_signals_before_terminal_fallback() {
        let mut observation = import_obs_idle();
        observation.signal_total = 7;
        observation.signal_pending_or_in_flight = 2;
        observation.signal_deferred = 5;
        let clock = ManualProgressClock::new(
            FixedOffset::east_opt(0)
                .unwrap()
                .timestamp_opt(0, 0)
                .unwrap(),
        );
        let mut progress = projection(&clock, &observation);
        assert_eq!(
            snapshot(&mut progress, &observation, &[], &clock).phase,
            BatchDisplayPhase::Signals
        );
    }

    #[test]
    fn provider_progress_is_provisional_without_replacing_local_denominator() {
        let mut observation = import_obs_idle();
        observation.signal_total = 32;
        observation.signal_deferred = 32;
        let clock = ManualProgressClock::new(
            FixedOffset::east_opt(0)
                .unwrap()
                .timestamp_opt(0, 0)
                .unwrap(),
        );
        let mut progress = projection(&clock, &observation);
        let snapshot = snapshot(
            &mut progress,
            &observation,
            &[peek(
                StageKind::SignalCandidate,
                "batch-signal",
                Some(BatchLifecycle::InProgress),
                25,
                32,
            )],
            &clock,
        );
        assert_eq!(snapshot.signals.total, 32);
        assert_eq!(snapshot.signals.settled(), 0);
        assert_eq!(snapshot.signals.provisional_settled, 25);
    }

    #[test]
    fn budget_capped_provider_scope_keeps_unsubmitted_work_visible() {
        let mut observation = import_obs_idle();
        observation.signal_total = 50;
        observation.signal_deferred = 50;
        let clock = ManualProgressClock::new(
            FixedOffset::east_opt(0)
                .unwrap()
                .timestamp_opt(0, 0)
                .unwrap(),
        );
        let mut progress = projection(&clock, &observation);
        let snapshot = snapshot(
            &mut progress,
            &observation,
            &[peek(
                StageKind::SignalCandidate,
                "batch-signal",
                Some(BatchLifecycle::InProgress),
                25,
                32,
            )],
            &clock,
        );
        assert_eq!(snapshot.signals.total, 50);
        assert_eq!(snapshot.signals.provider_total, 32);
        assert_eq!(snapshot.signals.unsubmitted, 18);
    }

    #[test]
    fn same_stage_peeks_aggregate_by_typed_stage_after_chunking() {
        let peeks = [
            peek(
                StageKind::Triage,
                "batch-one",
                Some(BatchLifecycle::InProgress),
                10,
                16,
            ),
            peek(
                StageKind::Triage,
                "batch-two",
                Some(BatchLifecycle::Finalizing),
                7,
                9,
            ),
            peek(
                StageKind::Summary,
                "batch-summary",
                Some(BatchLifecycle::InProgress),
                3,
                5,
            ),
        ];
        let provider = ProviderProgress::from_peeks(&peeks);
        assert_eq!(provider.triage.submitted, 25);
        assert_eq!(provider.triage.completed, 17);
        assert_eq!(provider.summaries.submitted, 5);

        let terminal = ProviderProgress::from_peeks(&[peek(
            StageKind::Triage,
            "batch-terminal",
            Some(BatchLifecycle::Completed),
            8,
            10,
        )]);
        assert_eq!(
            terminal.lifecycle(StageKind::Triage),
            ProviderLifecycle::ReadyToCollect
        );
    }

    #[test]
    fn custom_id_text_cannot_change_typed_provider_grouping() {
        let provider = ProviderProgress::from_peeks(&[peek(
            StageKind::SignalCandidate,
            "signal-triage-looking-custom-id",
            Some(BatchLifecycle::InProgress),
            1,
            2,
        )]);
        assert_eq!(provider.signals.submitted, 2);
        assert_eq!(provider.triage.submitted, 0);
    }

    #[test]
    fn provider_lookup_failure_is_indeterminate() {
        let mut observation = import_obs_idle();
        observation.triage_total = 8;
        observation.triage_deferred = 8;
        let clock = ManualProgressClock::new(
            FixedOffset::east_opt(0)
                .unwrap()
                .timestamp_opt(0, 0)
                .unwrap(),
        );
        let mut progress = projection(&clock, &observation);
        let snapshot = snapshot(
            &mut progress,
            &observation,
            &[peek(StageKind::Triage, "batch-failed-lookup", None, 0, 0)],
            &clock,
        );
        assert_eq!(
            snapshot.provider.lifecycle(StageKind::Triage),
            ProviderLifecycle::Indeterminate
        );
        assert_eq!(snapshot.phase, BatchDisplayPhase::WaitingForProvider);
    }

    #[test]
    fn deferred_work_without_peeks_is_preparing_not_zero_request_wait() {
        let mut observation = import_obs_idle();
        observation.summary_total = 4;
        observation.summary_deferred = 4;
        let clock = ManualProgressClock::new(
            FixedOffset::east_opt(0)
                .unwrap()
                .timestamp_opt(0, 0)
                .unwrap(),
        );
        let mut progress = projection(&clock, &observation);
        let snapshot = snapshot(&mut progress, &observation, &[], &clock);
        assert_eq!(snapshot.phase, BatchDisplayPhase::PreparingBatch);
        assert_eq!(snapshot.summaries.provider_total, 0);
        assert_eq!(snapshot.summaries.total, 4);
    }

    #[test]
    fn rearm_transition_cannot_shrink_latched_stage_total() {
        let mut observation = import_obs_idle();
        observation.signal_total = 50;
        observation.signal_pending_or_in_flight = 50;
        let clock = ManualProgressClock::new(
            FixedOffset::east_opt(0)
                .unwrap()
                .timestamp_opt(0, 0)
                .unwrap(),
        );
        let mut progress = projection(&clock, &observation);
        assert_eq!(
            snapshot(&mut progress, &observation, &[], &clock)
                .signals
                .total,
            50
        );
        observation.signal_total = 0;
        observation.signal_pending_or_in_flight = 0;
        observation.signal_deferred = 50;
        assert_eq!(
            snapshot(&mut progress, &observation, &[], &clock)
                .signals
                .total,
            50
        );
    }

    #[test]
    fn rearm_can_replace_pending_urls_without_shrinking_latched_total() {
        let mut observation = import_obs_idle();
        observation.summary_total = 12;
        observation.summary_pending = 12;
        let clock = ManualProgressClock::new(
            FixedOffset::east_opt(0)
                .unwrap()
                .timestamp_opt(0, 0)
                .unwrap(),
        );
        let mut progress = projection(&clock, &observation);
        let _ = snapshot(&mut progress, &observation, &[], &clock);
        observation.summary_total = 5;
        observation.summary_pending = 5;
        let rearmed = snapshot(&mut progress, &observation, &[], &clock);
        assert_eq!(rearmed.summaries.total, 12);
        assert_eq!(rearmed.summaries.pending_or_in_flight, 5);
    }

    #[test]
    fn failures_settle_stage_progress_without_hiding_failure_count() {
        let mut observation = import_obs_idle();
        observation.triage_total = 10;
        observation.triage_completed = 7;
        observation.triage_failed = 3;
        let clock = ManualProgressClock::new(
            FixedOffset::east_opt(0)
                .unwrap()
                .timestamp_opt(0, 0)
                .unwrap(),
        );
        let mut progress = projection(&clock, &observation);
        let snapshot = snapshot(&mut progress, &observation, &[], &clock);
        assert_eq!(snapshot.triage.settled(), 10);
        assert_eq!(snapshot.triage.failed, 3);
    }

    #[test]
    fn dynamic_stage_totals_remain_per_stage_without_an_overall_percentage() {
        let mut observation = import_obs_idle();
        observation.triage_total = 10;
        observation.summary_total = 4;
        let clock = ManualProgressClock::new(
            FixedOffset::east_opt(0)
                .unwrap()
                .timestamp_opt(0, 0)
                .unwrap(),
        );
        let mut progress = projection(&clock, &observation);
        let snapshot = snapshot(&mut progress, &observation, &[], &clock);
        assert_eq!(snapshot.triage.total, 10);
        assert_eq!(snapshot.summaries.total, 4);
    }

    #[test]
    fn replay_cost_is_explicitly_scoped_to_this_run() {
        let observation = import_obs_idle();
        let clock = ManualProgressClock::new(
            FixedOffset::east_opt(0)
                .unwrap()
                .timestamp_opt(0, 0)
                .unwrap(),
        );
        let mut progress = projection(&clock, &observation);
        let snapshot = progress.snapshot(
            &observation,
            &[],
            ProjectionContext {
                cost_this_run_microdollars: 250_000,
                ..ProjectionContext::default()
            },
            &clock,
        );
        assert_eq!(snapshot.cost_this_run_microdollars, 250_000);
    }

    #[test]
    fn local_wait_timestamps_keep_fixed_offsets() {
        let observation = import_obs_idle();
        let summer = FixedOffset::east_opt(2 * 3600)
            .unwrap()
            .with_ymd_and_hms(2026, 7, 23, 9, 48, 30)
            .unwrap();
        let winter = FixedOffset::east_opt(3600)
            .unwrap()
            .with_ymd_and_hms(2026, 12, 23, 9, 48, 30)
            .unwrap();
        let clock = ManualProgressClock::new(summer);
        let mut progress = projection(&clock, &observation);
        let snapshot = progress.snapshot(
            &observation,
            &[],
            ProjectionContext {
                last_provider_check_local: Some(summer),
                next_provider_check_local: Some(winter),
                ..ProjectionContext::default()
            },
            &clock,
        );
        let wait = snapshot.wait.unwrap();
        assert_eq!(
            wait.last_provider_check_display.as_deref(),
            Some("2026-07-23 09:48:30 +02:00")
        );
        assert_eq!(
            wait.next_provider_check_display.as_deref(),
            Some("2026-12-23 09:48:30 +01:00")
        );
        assert_eq!(summer.to_utc().to_rfc3339(), "2026-07-23T07:48:30+00:00");
    }

    #[test]
    fn clock_contract_supports_manual_advancement_and_all_display_phases() {
        let wall = FixedOffset::east_opt(2 * 3600)
            .unwrap()
            .with_ymd_and_hms(2026, 7, 23, 9, 48, 30)
            .unwrap();
        let clock = ManualProgressClock::new(wall);
        clock.sleep(Duration::from_secs(5));
        assert_eq!(clock.wall_now(), wall + chrono::Duration::seconds(5));
        let _system_clock = SystemProgressClock;
        let phases = [
            BatchDisplayPhase::Reconciling,
            BatchDisplayPhase::Intake,
            BatchDisplayPhase::Triage,
            BatchDisplayPhase::Summaries,
            BatchDisplayPhase::Signals,
            BatchDisplayPhase::PreparingBatch,
            BatchDisplayPhase::CheckingProvider,
            BatchDisplayPhase::WaitingForProvider,
            BatchDisplayPhase::Collecting,
            BatchDisplayPhase::Replaying,
            BatchDisplayPhase::Persisting,
            BatchDisplayPhase::Complete,
            BatchDisplayPhase::Interrupted,
        ];
        assert_eq!(phases.len(), 13);
    }
}
