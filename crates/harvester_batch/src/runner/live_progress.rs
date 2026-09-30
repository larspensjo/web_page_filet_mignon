use crate::progress::{
    BatchDisplayPhase, BatchProgressProjection, BatchProgressSnapshot, BatchRunBaseline,
    PlainProgressReporter, ProgressClock, ProgressGlyphs, ProjectionContext, SystemProgressClock,
    TerminalProgressSurface,
};
use engine_logging::engine_warn;
use harvester_core::AppState;
use std::io::Write;
use std::time::{Duration, Instant};

const PROGRESS_REFRESH_INTERVAL: Duration = Duration::from_millis(250);
const PLAIN_PROGRESS_HEARTBEAT_INTERVAL: Duration = Duration::from_secs(60);

enum BatchProgressSurface<W: Write> {
    Terminal(TerminalProgressSurface<W>),
    Plain(PlainProgressReporter<W>),
}

impl BatchProgressSurface<std::io::Stdout> {
    fn new(interactive: bool, ascii_progress: bool) -> Self {
        if interactive {
            Self::Terminal(TerminalProgressSurface::new(
                std::io::stdout(),
                progress_glyphs(ascii_progress),
            ))
        } else {
            Self::Plain(PlainProgressReporter::new(std::io::stdout()))
        }
    }
}

fn progress_glyphs(ascii_progress: bool) -> ProgressGlyphs {
    if ascii_progress {
        ProgressGlyphs::Ascii
    } else {
        ProgressGlyphs::Unicode
    }
}

impl<W: Write> BatchProgressSurface<W> {
    fn paint(&mut self, snapshot: &BatchProgressSnapshot) {
        let result = match self {
            Self::Terminal(surface) => surface.repaint(snapshot),
            Self::Plain(reporter) => reporter.report(snapshot),
        };
        if let Err(err) = result {
            engine_warn!(
                "[batch-progress] stdout repaint failed; continuing safely: {}",
                err
            );
        }
    }

    fn suspend_for_output(&mut self) {
        if let Self::Terminal(surface) = self {
            if let Err(err) = surface.suspend_for_output() {
                engine_warn!("[batch-progress] failed to suspend dashboard: {}", err);
            }
        }
    }

    fn resume(&mut self, snapshot: &BatchProgressSnapshot) {
        if let Self::Terminal(surface) = self {
            if let Err(err) = surface.resume(snapshot) {
                engine_warn!("[batch-progress] failed to resume dashboard: {}", err);
            }
        } else {
            self.paint(snapshot);
        }
    }

    fn finish(&mut self) {
        if let Self::Terminal(surface) = self {
            if let Err(err) = surface.finish() {
                engine_warn!("[batch-progress] failed to finish dashboard: {}", err);
            }
        }
    }

    fn is_terminal(&self) -> bool {
        matches!(self, Self::Terminal(_))
    }
}

pub(super) struct LiveBatchProgress<C: ProgressClock, W: Write> {
    clock: C,
    projection: BatchProgressProjection,
    surface: BatchProgressSurface<W>,
    phase_override: Option<BatchDisplayPhase>,
    last_render: Instant,
    last_plain_phase: Option<BatchDisplayPhase>,
}

pub(super) type LiveSystemBatchProgress = LiveBatchProgress<SystemProgressClock, std::io::Stdout>;

impl LiveBatchProgress<SystemProgressClock, std::io::Stdout> {
    pub(super) fn new(baseline: BatchRunBaseline, interactive: bool, ascii_progress: bool) -> Self {
        let clock = SystemProgressClock;
        let surface = BatchProgressSurface::new(interactive, ascii_progress);
        Self::with_parts(baseline, clock, surface)
    }
}

impl<C: ProgressClock, W: Write> LiveBatchProgress<C, W> {
    fn with_parts(baseline: BatchRunBaseline, clock: C, surface: BatchProgressSurface<W>) -> Self {
        let started_at = clock.monotonic_now();
        Self {
            clock,
            projection: BatchProgressProjection::new(baseline, started_at),
            surface,
            phase_override: None,
            last_render: started_at,
            last_plain_phase: None,
        }
    }

    pub(super) fn set_phase(&mut self, phase: BatchDisplayPhase) {
        self.phase_override = Some(phase);
    }

    pub(super) fn clear_phase_override(&mut self) {
        self.phase_override = None;
    }

    fn snapshot(
        &mut self,
        state: &AppState,
        cost_this_run_microdollars: u64,
    ) -> BatchProgressSnapshot {
        self.projection.snapshot(
            &state.batch_observation(),
            ProjectionContext {
                phase_override: self.phase_override,
                cost_this_run_microdollars,
            },
            &self.clock,
        )
    }

    pub(super) fn paint(&mut self, state: &AppState, cost: u64, force: bool) {
        let now = self.clock.monotonic_now();
        let due = now.saturating_duration_since(self.last_render) >= PROGRESS_REFRESH_INTERVAL;
        let plain_due =
            now.saturating_duration_since(self.last_render) >= PLAIN_PROGRESS_HEARTBEAT_INTERVAL;
        if !force && !due {
            return;
        }
        let snapshot = self.snapshot(state, cost);
        let phase_changed = self.last_plain_phase != Some(snapshot.phase);
        if !self.surface.is_terminal() && !force && !phase_changed && !plain_due {
            return;
        }
        self.surface.paint(&snapshot);
        self.last_render = now;
        self.last_plain_phase = Some(snapshot.phase);
    }

    pub(super) fn suspend_for_output(&mut self) {
        self.surface.suspend_for_output();
    }

    pub(super) fn resume(&mut self, state: &AppState, cost: u64) {
        let snapshot = self.snapshot(state, cost);
        self.surface.resume(&snapshot);
        self.last_render = self.clock.monotonic_now();
        self.last_plain_phase = Some(snapshot.phase);
    }

    pub(super) fn finish(&mut self) {
        self.surface.finish();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::progress::TestProgressClock;
    use chrono::TimeZone;
    use std::cell::{Cell, RefCell};
    use std::sync::{Arc, Mutex};

    #[derive(Clone, Default)]
    struct SharedRunnerOutput(Arc<Mutex<Vec<u8>>>);

    impl SharedRunnerOutput {
        fn bytes(&self) -> Vec<u8> {
            self.0.lock().unwrap().clone()
        }

        fn text(&self) -> String {
            String::from_utf8(self.bytes()).unwrap()
        }
    }

    impl Write for SharedRunnerOutput {
        fn write(&mut self, buffer: &[u8]) -> std::io::Result<usize> {
            self.0.lock().unwrap().extend_from_slice(buffer);
            Ok(buffer.len())
        }

        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }

    struct ManualWaitClock {
        start: Instant,
        elapsed: Cell<Duration>,
        wall: RefCell<chrono::DateTime<chrono::FixedOffset>>,
    }

    impl ManualWaitClock {
        fn new(wall: chrono::DateTime<chrono::FixedOffset>) -> Self {
            Self {
                start: Instant::now(),
                elapsed: Cell::new(Duration::ZERO),
                wall: RefCell::new(wall),
            }
        }
    }

    impl ProgressClock for ManualWaitClock {
        fn monotonic_now(&self) -> Instant {
            self.start + self.elapsed.get()
        }
    }

    impl TestProgressClock for ManualWaitClock {
        fn wall_now(&self) -> chrono::DateTime<chrono::FixedOffset> {
            *self.wall.borrow()
        }

        fn sleep(&self, duration: Duration) {
            self.elapsed.set(self.elapsed.get() + duration);
            let updated = *self.wall.borrow() + chrono::Duration::from_std(duration).unwrap();
            *self.wall.borrow_mut() = updated;
        }
    }

    impl ProgressClock for &ManualWaitClock {
        fn monotonic_now(&self) -> Instant {
            (*self).monotonic_now()
        }
    }

    impl TestProgressClock for &ManualWaitClock {
        fn wall_now(&self) -> chrono::DateTime<chrono::FixedOffset> {
            (*self).wall_now()
        }

        fn sleep(&self, duration: Duration) {
            (*self).sleep(duration);
        }
    }

    fn empty_run_baseline() -> BatchRunBaseline {
        BatchRunBaseline {
            jobs_total: 0,
            jobs_done: 0,
            jobs_failed: 0,
        }
    }

    #[test]
    fn painted_cost_reflects_synchronous_session_usage() {
        let (state, _) = harvester_core::update(
            AppState::new(),
            harvester_core::Msg::LlmQuotaUsageUpdated {
                usage: harvester_core::LlmQuotaUsage {
                    calls: 1,
                    cost_microdollars: 1_250_000,
                    ..Default::default()
                },
            },
        );
        for terminal in [false, true] {
            let output = SharedRunnerOutput::default();
            let surface = if terminal {
                BatchProgressSurface::Terminal(TerminalProgressSurface::new(
                    output.clone(),
                    ProgressGlyphs::Ascii,
                ))
            } else {
                BatchProgressSurface::Plain(PlainProgressReporter::new(output.clone()))
            };
            let mut progress =
                LiveBatchProgress::with_parts(empty_run_baseline(), SystemProgressClock, surface);
            progress.paint(&state, state.llm_quota().usage.cost_microdollars, true);
            assert!(output.text().contains("$1.25"), "{}", output.text());
            progress.suspend_for_output();
            progress.resume(&state, state.llm_quota().usage.cost_microdollars);
            assert_eq!(output.text().matches("$1.25").count(), 2);
            progress.finish();
        }
    }

    #[test]
    fn ascii_progress_selects_ascii_glyphs_only_for_interactive_dashboard() {
        assert_eq!(progress_glyphs(true), ProgressGlyphs::Ascii);
        assert_eq!(progress_glyphs(false), ProgressGlyphs::Unicode);
    }

    #[test]
    fn plain_progress_throttles_steady_heartbeats_and_flushes_each_phase_transition() {
        let wall = chrono::FixedOffset::east_opt(2 * 60 * 60)
            .unwrap()
            .with_ymd_and_hms(2026, 7, 23, 9, 43, 30)
            .single()
            .unwrap();
        let clock = ManualWaitClock::new(wall);
        let output = SharedRunnerOutput::default();
        let surface = BatchProgressSurface::Plain(PlainProgressReporter::new(output.clone()));
        let mut progress = LiveBatchProgress::with_parts(empty_run_baseline(), &clock, surface);
        let state = AppState::new();

        progress.set_phase(BatchDisplayPhase::Intake);
        clock.sleep(PROGRESS_REFRESH_INTERVAL);
        progress.paint(&state, 0, false);
        assert_eq!(output.text().lines().count(), 1);

        clock.sleep(PLAIN_PROGRESS_HEARTBEAT_INTERVAL - Duration::from_secs(1));
        progress.paint(&state, 0, false);
        assert_eq!(
            output.text().lines().count(),
            1,
            "steady-state output must remain quiet before the minute boundary"
        );

        clock.sleep(Duration::from_secs(1));
        progress.paint(&state, 0, false);
        assert_eq!(
            output.text().lines().count(),
            2,
            "exactly one steady-state heartbeat is due after one minute"
        );

        clock.sleep(PROGRESS_REFRESH_INTERVAL);
        progress.paint(&state, 0, false);
        assert_eq!(
            output.text().lines().count(),
            2,
            "a second steady-state line must not follow within the minute"
        );

        progress.set_phase(BatchDisplayPhase::Triage);
        clock.sleep(PROGRESS_REFRESH_INTERVAL);
        progress.paint(&state, 0, false);
        assert_eq!(
            output.text().lines().count(),
            3,
            "a phase transition must flush one line within the heartbeat window"
        );

        clock.sleep(PROGRESS_REFRESH_INTERVAL);
        progress.paint(&state, 0, false);
        assert_eq!(
            output.text().lines().count(),
            3,
            "the phase transition must emit exactly one line"
        );

        progress.set_phase(BatchDisplayPhase::Summaries);
        clock.sleep(PROGRESS_REFRESH_INTERVAL);
        progress.paint(&state, 0, false);
        assert_eq!(
            output.text().lines().count(),
            4,
            "every distinct phase transition must flush exactly one line"
        );
    }

    #[test]
    fn terminal_progress_stays_live_across_stages_and_finishes_once() {
        let wall = chrono::FixedOffset::east_opt(0)
            .unwrap()
            .with_ymd_and_hms(2026, 7, 23, 9, 43, 30)
            .single()
            .unwrap();
        let clock = ManualWaitClock::new(wall);
        let output = SharedRunnerOutput::default();
        let surface = BatchProgressSurface::Terminal(TerminalProgressSurface::new(
            output.clone(),
            ProgressGlyphs::Unicode,
        ));
        let mut progress = LiveBatchProgress::with_parts(empty_run_baseline(), &clock, surface);
        let state = AppState::new();

        progress.set_phase(BatchDisplayPhase::Intake);
        progress.paint(&state, 0, true);
        for _ in 0..3 {
            progress.set_phase(BatchDisplayPhase::Triage);
            progress.paint(&state, 0, true);
        }

        let before_finish = output.text();
        assert_eq!(
            before_finish.matches("\u{1b}[?25l").count(),
            1,
            "one persistent surface hides the cursor only once"
        );
        assert!(
            !before_finish.contains("\u{1b}[?25h"),
            "collection passes must not finish and append historical dashboards"
        );

        progress.set_phase(BatchDisplayPhase::Complete);
        progress.paint(&state, 0, true);
        progress.finish();

        let finished = output.text();
        assert_eq!(
            finished.matches("\u{1b}[?25h").count(),
            1,
            "the terminal surface must be finished exactly once"
        );
        assert!(
            finished.ends_with("\u{1b}[?25h\n"),
            "only the final dashboard may be terminated as historical output"
        );
    }
}
