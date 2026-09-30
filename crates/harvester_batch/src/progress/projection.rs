#[cfg(test)]
use chrono::{DateTime, FixedOffset, Local};
use harvester_core::BatchObservation;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ProgressStage {
    Triage,
    Summary,
    SignalCandidate,
}
use std::time::{Duration, Instant};

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

/// One stage's local completion and remaining-work counts.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct StageProgress {
    pub total: usize,
    pub successful: usize,
    pub failed: usize,
    pub pending_or_in_flight: usize,
    pub local_remaining: usize,
}

impl StageProgress {
    pub fn settled(self) -> usize {
        self.successful.saturating_add(self.failed)
    }
}

/// The projection derives the active pipeline phase unless the host supplies
/// a persistence or terminal phase.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BatchDisplayPhase {
    Intake,
    Triage,
    Summaries,
    Signals,
    Persisting,
    Complete,
    Interrupted,
}

/// Runner clock seam for elapsed-time presentation.
pub trait ProgressClock {
    fn monotonic_now(&self) -> Instant;
}

#[cfg(test)]
pub(crate) trait TestProgressClock: ProgressClock {
    fn wall_now(&self) -> DateTime<FixedOffset>;
    fn sleep(&self, duration: Duration);
}

pub struct SystemProgressClock;

impl ProgressClock for SystemProgressClock {
    fn monotonic_now(&self) -> Instant {
        Instant::now()
    }
}

#[cfg(test)]
impl TestProgressClock for SystemProgressClock {
    fn wall_now(&self) -> DateTime<FixedOffset> {
        Local::now().fixed_offset()
    }

    fn sleep(&self, duration: Duration) {
        std::thread::sleep(duration);
    }
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
    pub phase: BatchDisplayPhase,
    pub remaining_work: usize,
}

/// Explicit facts supplied by the runner without reaching into reducer state.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct ProjectionContext {
    pub phase_override: Option<BatchDisplayPhase>,
    pub cost_this_run_microdollars: u64,
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
        let triage = self.stage_progress(
            ProgressStage::Triage,
            LocalStageCounts {
                total: observation.triage_total,
                successful: observation.triage_completed,
                failed: observation.triage_failed,
                pending_or_in_flight: observation
                    .triage_pending
                    .saturating_add(observation.triage_in_flight),
            },
        );
        let summaries = self.stage_progress(
            ProgressStage::Summary,
            LocalStageCounts {
                total: observation.summary_total,
                successful: observation.summary_completed,
                failed: observation.summary_failed,
                pending_or_in_flight: observation
                    .summary_pending
                    .saturating_add(observation.summary_in_flight),
            },
        );
        let signals = self.stage_progress(
            ProgressStage::SignalCandidate,
            LocalStageCounts {
                total: observation.signal_total,
                successful: observation.signal_completed,
                failed: observation.signal_failed,
                pending_or_in_flight: observation.signal_pending_or_in_flight,
            },
        );
        let phase = classify_display_phase(observation, context.phase_override);
        BatchProgressSnapshot {
            elapsed: now.saturating_duration_since(self.started_at),
            cost_this_run_microdollars: context.cost_this_run_microdollars,
            intake,
            triage,
            summaries,
            signals,
            phase,
            remaining_work: triage
                .local_remaining
                .saturating_add(summaries.local_remaining)
                .saturating_add(signals.local_remaining),
        }
    }

    fn stage_progress(&mut self, stage: ProgressStage, local: LocalStageCounts) -> StageProgress {
        let latched_total = match stage {
            ProgressStage::Triage => {
                self.triage_total = self.triage_total.max(local.total);
                self.triage_total
            }
            ProgressStage::Summary => {
                self.summary_total = self.summary_total.max(local.total);
                self.summary_total
            }
            ProgressStage::SignalCandidate => {
                self.signal_total = self.signal_total.max(local.total);
                self.signal_total
            }
        };
        let total = latched_total;
        StageProgress {
            total,
            successful: local.successful,
            failed: local.failed,
            pending_or_in_flight: local.pending_or_in_flight,
            local_remaining: local.pending_or_in_flight,
        }
    }
}

/// Classifies one active phase from reducer/provider state. Signal work is
/// intentionally considered before the terminal fallback.
pub fn classify_display_phase(
    observation: &BatchObservation,
    phase_override: Option<BatchDisplayPhase>,
) -> BatchDisplayPhase {
    if let Some(phase) = phase_override {
        return phase;
    }
    // Intake intentionally wins first-match classification while later stages overlap it.
    if observation.poll_in_progress
        || (observation.jobs_total > 0
            && observation
                .jobs_done
                .saturating_add(observation.jobs_failed)
                < observation.jobs_total)
    {
        return BatchDisplayPhase::Intake;
    }
    for (stage, pending) in [
        (
            ProgressStage::Triage,
            observation
                .triage_pending
                .saturating_add(observation.triage_in_flight),
        ),
        (
            ProgressStage::Summary,
            observation
                .summary_pending
                .saturating_add(observation.summary_in_flight),
        ),
        (
            ProgressStage::SignalCandidate,
            observation.signal_pending_or_in_flight,
        ),
    ] {
        if pending > 0 {
            return stage_display_phase(stage);
        }
    }
    BatchDisplayPhase::Complete
}

fn stage_display_phase(stage: ProgressStage) -> BatchDisplayPhase {
    match stage {
        ProgressStage::Triage => BatchDisplayPhase::Triage,
        ProgressStage::Summary => BatchDisplayPhase::Summaries,
        ProgressStage::SignalCandidate => BatchDisplayPhase::Signals,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::{DateTime, FixedOffset, TimeZone};
    use harvester_core::{
        BatchObservation, ImportPhase, PreTriagePhase, SessionState, TriagePhase,
    };
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
            signal_total: 0,
            signal_pending_or_in_flight: 0,
            signal_completed: 0,
            signal_failed: 0,
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
    }

    impl TestProgressClock for ManualProgressClock {
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

    fn snapshot(
        projection: &mut BatchProgressProjection,
        observation: &BatchObservation,
        clock: &ManualProgressClock,
    ) -> BatchProgressSnapshot {
        projection.snapshot(observation, ProjectionContext::default(), clock)
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
        let first = snapshot(&mut progress, &observed, &clock);
        assert_eq!(first.intake.discovered, 76);
        assert_eq!(first.intake.fetched, 69);
        assert_eq!(first.intake.failed, 7);
        assert_eq!(first.intake.total, 76);

        observed.jobs_total += 2;
        let after_settlement = snapshot(&mut progress, &observed, &clock);
        assert_eq!(after_settlement.intake.discovered, 78);
        assert_eq!(after_settlement.intake.total, 76);
    }

    #[test]
    fn signal_work_selects_signals_before_terminal_fallback() {
        let mut observation = import_obs_idle();
        observation.signal_total = 7;
        observation.signal_pending_or_in_flight = 2;
        let clock = ManualProgressClock::new(
            FixedOffset::east_opt(0)
                .unwrap()
                .timestamp_opt(0, 0)
                .unwrap(),
        );
        let mut progress = projection(&clock, &observation);
        assert_eq!(
            snapshot(&mut progress, &observation, &clock).phase,
            BatchDisplayPhase::Signals
        );
    }

    #[test]
    fn intake_remains_the_first_display_phase_during_stage_overlap() {
        let mut observation = import_obs_idle();
        observation.poll_in_progress = true;
        observation.triage_pending = 1;
        observation.summary_pending = 1;
        observation.signal_pending_or_in_flight = 1;
        assert_eq!(
            classify_display_phase(&observation, None),
            BatchDisplayPhase::Intake
        );

        observation.poll_in_progress = false;
        observation.jobs_total = 2;
        observation.jobs_done = 1;
        assert_eq!(
            classify_display_phase(&observation, None),
            BatchDisplayPhase::Intake
        );
    }

    #[test]
    fn settlement_cannot_shrink_latched_stage_total() {
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
            snapshot(&mut progress, &observation, &clock).signals.total,
            50
        );
        observation.signal_total = 0;
        observation.signal_pending_or_in_flight = 0;
        assert_eq!(
            snapshot(&mut progress, &observation, &clock).signals.total,
            50
        );
    }

    #[test]
    fn pending_window_can_shrink_without_shrinking_latched_total() {
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
        let _ = snapshot(&mut progress, &observation, &clock);
        observation.summary_total = 5;
        observation.summary_pending = 5;
        let later = snapshot(&mut progress, &observation, &clock);
        assert_eq!(later.summaries.total, 12);
        assert_eq!(later.summaries.pending_or_in_flight, 5);
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
        let snapshot = snapshot(&mut progress, &observation, &clock);
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
        let snapshot = snapshot(&mut progress, &observation, &clock);
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
            ProjectionContext {
                cost_this_run_microdollars: 250_000,
                ..ProjectionContext::default()
            },
            &clock,
        );
        assert_eq!(snapshot.cost_this_run_microdollars, 250_000);
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
            BatchDisplayPhase::Intake,
            BatchDisplayPhase::Triage,
            BatchDisplayPhase::Summaries,
            BatchDisplayPhase::Signals,
            BatchDisplayPhase::Persisting,
            BatchDisplayPhase::Complete,
            BatchDisplayPhase::Interrupted,
        ];
        assert_eq!(phases.len(), 7);
    }
}
