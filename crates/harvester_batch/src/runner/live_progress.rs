use crate::progress::{
    format_progress_block, progress_status_signature, PlainProgressReporter,
    TerminalProgressSurface,
};
use engine_logging::engine_warn;
use harvester_core::{AppState, PipelineRunPhase};
use std::io::Write;
use std::time::{Duration, Instant};

const PROGRESS_REFRESH_INTERVAL: Duration = Duration::from_millis(250);
const PLAIN_PROGRESS_HEARTBEAT_INTERVAL: Duration = Duration::from_secs(60);

pub(super) trait ProgressClock {
    fn now(&self) -> Instant;
}

pub(super) struct SystemProgressClock;
impl ProgressClock for SystemProgressClock {
    fn now(&self) -> Instant {
        Instant::now()
    }
}

enum BatchProgressSurface<W: Write> {
    Terminal(TerminalProgressSurface<W>),
    Plain(PlainProgressReporter<W>),
}

impl<W: Write> BatchProgressSurface<W> {
    fn paint(&mut self, lines: &[String]) {
        let result = match self {
            Self::Terminal(surface) => surface.repaint(lines),
            Self::Plain(reporter) => reporter.report(lines),
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
                engine_warn!("[batch-progress] failed to suspend block: {}", err);
            }
        }
    }
    fn finish(&mut self) {
        if let Self::Terminal(surface) = self {
            if let Err(err) = surface.finish() {
                engine_warn!("[batch-progress] failed to finish block: {}", err);
            }
        }
    }
    fn is_terminal(&self) -> bool {
        matches!(self, Self::Terminal(_))
    }
}

pub(super) struct LiveBatchProgress<C: ProgressClock, W: Write> {
    clock: C,
    started_at: Instant,
    surface: BatchProgressSurface<W>,
    stopping: bool,
    last_check: Instant,
    last_render: Instant,
    last_lines: Vec<String>,
    last_statuses: Vec<&'static str>,
    last_stopping: bool,
}

pub(super) type LiveSystemBatchProgress = LiveBatchProgress<SystemProgressClock, std::io::Stdout>;

impl LiveSystemBatchProgress {
    pub(super) fn new(interactive: bool) -> Self {
        let surface = if interactive {
            BatchProgressSurface::Terminal(TerminalProgressSurface::new(std::io::stdout()))
        } else {
            BatchProgressSurface::Plain(PlainProgressReporter::new(std::io::stdout()))
        };
        Self::with_parts(SystemProgressClock, surface)
    }
}

impl<C: ProgressClock, W: Write> LiveBatchProgress<C, W> {
    fn with_parts(clock: C, surface: BatchProgressSurface<W>) -> Self {
        let started_at = clock.now();
        Self {
            clock,
            started_at,
            surface,
            stopping: false,
            last_check: started_at,
            last_render: started_at,
            last_lines: Vec::new(),
            last_statuses: Vec::new(),
            last_stopping: false,
        }
    }
    // Keep stopping presentation through final persistence, even after core settles.
    pub(super) fn set_stopping(&mut self, stopping: bool) {
        self.stopping |= stopping;
    }
    pub(super) fn paint(&mut self, state: &AppState, cost: u64, force: bool) {
        let now = self.clock.now();
        let terminal = self.surface.is_terminal();
        if terminal
            && !force
            && now.saturating_duration_since(self.last_check) < PROGRESS_REFRESH_INTERVAL
        {
            return;
        }
        self.last_check = now;
        self.stopping |= state.pipeline_run_phase() == PipelineRunPhase::Stopping;
        let heartbeat =
            now.saturating_duration_since(self.last_render) >= PLAIN_PROGRESS_HEARTBEAT_INTERVAL;
        let progress = state.run_progress_view();
        let statuses = progress_status_signature(&progress.stages, self.stopping);
        // Plain output reports status transitions immediately, but count-only
        // changes wait for a heartbeat or a forced paint with changed content.
        if !terminal
            && !force
            && !heartbeat
            && self.stopping == self.last_stopping
            && statuses == self.last_statuses
        {
            return;
        }
        let lines = format_progress_block(
            &progress.stages,
            self.stopping,
            now.saturating_duration_since(self.started_at),
            cost,
        );
        if !terminal && self.last_lines == lines {
            return;
        }
        self.surface.paint(&lines);
        self.last_lines = lines;
        self.last_statuses = statuses;
        self.last_stopping = self.stopping;
        self.last_render = now;
    }
    pub(super) fn suspend_for_output(&mut self) {
        self.surface.suspend_for_output();
    }
    pub(super) fn resume(&mut self, state: &AppState, cost: u64) {
        self.paint(state, cost, true);
    }
    pub(super) fn finish(&mut self) {
        self.surface.finish();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use harvester_core::{Msg, PipelineRunScope};
    use std::cell::Cell;
    use std::sync::{Arc, Mutex};

    #[derive(Clone, Default)]
    struct SharedRunnerOutput(Arc<Mutex<Vec<u8>>>);
    impl SharedRunnerOutput {
        fn text(&self) -> String {
            String::from_utf8(self.0.lock().unwrap().clone()).unwrap()
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
    struct ManualClock {
        start: Instant,
        elapsed: Cell<Duration>,
    }
    impl ManualClock {
        fn new() -> Self {
            Self {
                start: Instant::now(),
                elapsed: Cell::new(Duration::ZERO),
            }
        }
        fn advance(&self, duration: Duration) {
            self.elapsed.set(self.elapsed.get() + duration);
        }
    }
    impl ProgressClock for &ManualClock {
        fn now(&self) -> Instant {
            self.start + self.elapsed.get()
        }
    }
    #[test]
    fn painted_cost_reflects_synchronous_session_usage() {
        let (state, _) = harvester_core::update(
            AppState::new(),
            Msg::LlmQuotaUsageUpdated {
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
                BatchProgressSurface::Terminal(TerminalProgressSurface::new(output.clone()))
            } else {
                BatchProgressSurface::Plain(PlainProgressReporter::new(output.clone()))
            };
            let mut progress = LiveBatchProgress::with_parts(SystemProgressClock, surface);
            progress.paint(&state, state.llm_quota().usage.cost_microdollars, true);
            assert!(output.text().contains("$1.25"), "{}", output.text());
            progress.suspend_for_output();
            progress.resume(&state, state.llm_quota().usage.cost_microdollars);
            assert_eq!(
                output.text().matches("$1.25").count(),
                if terminal { 2 } else { 1 }
            );
            progress.finish();
        }
    }
    #[test]
    fn plain_progress_throttles_steady_heartbeats_and_flushes_each_phase_transition() {
        let clock = ManualClock::new();
        let output = SharedRunnerOutput::default();
        let surface = BatchProgressSurface::Plain(PlainProgressReporter::new(output.clone()));
        let mut progress = LiveBatchProgress::with_parts(&clock, surface);
        let state = AppState::new();
        progress.paint(&state, 0, true);
        clock.advance(PLAIN_PROGRESS_HEARTBEAT_INTERVAL - Duration::from_secs(1));
        progress.paint(&state, 0, false);
        assert_eq!(output.text().lines().count(), 1);
        clock.advance(Duration::from_secs(1));
        progress.paint(&state, 0, false);
        assert_eq!(output.text().lines().count(), 2);
        clock.advance(PROGRESS_REFRESH_INTERVAL);
        progress.paint(&state, 0, false);
        assert_eq!(output.text().lines().count(), 2);
        // Real reducer transitions replace the old host-selected phase overrides.
        let (state, _) = harvester_core::update(
            state,
            Msg::PipelineRunRequested {
                scope: PipelineRunScope::Full,
            },
        );
        clock.advance(PROGRESS_REFRESH_INTERVAL);
        progress.paint(&state, 0, false);
        assert_eq!(output.text().lines().count(), 3);
        clock.advance(PROGRESS_REFRESH_INTERVAL);
        progress.paint(&state, 0, false);
        assert_eq!(output.text().lines().count(), 3);
        let (state, _) = harvester_core::update(state, Msg::PollStarted { total: 2 });
        // The following changes arrive at the same instant, inside the terminal
        // repaint interval. Starting the poll changes a stage status and emits.
        progress.paint(&state, 0, false);
        assert_eq!(output.text().lines().count(), 4);
        let (state, _) = harvester_core::update(
            state,
            Msg::SourcePollCompleted {
                source_id: harvester_engine::SourceId::new("first").unwrap(),
                urls: Vec::new(),
                kind: harvester_engine::SourceKind::Rss,
                parsed: 0,
                dedup_filtered: 0,
            },
        );
        progress.paint(&state, 0, false);
        assert_eq!(
            output.text().lines().count(),
            4,
            "count changes within a stage do not emit a line"
        );
        assert!(!output.text().contains("Scanning sources: 1 of 2 to do"));
        clock.advance(PLAIN_PROGRESS_HEARTBEAT_INTERVAL);
        progress.paint(&state, 0, false);
        assert_eq!(output.text().lines().count(), 5);
        assert!(output.text().contains("Scanning sources: 1 of 2 to do"));
        let (state, _) = harvester_core::update(
            state,
            Msg::SourcePollCompleted {
                source_id: harvester_engine::SourceId::new("second").unwrap(),
                urls: Vec::new(),
                kind: harvester_engine::SourceKind::Rss,
                parsed: 0,
                dedup_filtered: 0,
            },
        );
        let (state, _) = harvester_core::update(state, Msg::AllSourcesPollEnded);
        progress.paint(&state, 0, false);
        assert_eq!(
            output.text().lines().count(),
            6,
            "stage completion emits a line"
        );
        assert!(output
            .text()
            .contains("Scanning sources: 0 of 2 to do | Done"));
    }
    #[test]
    fn terminal_progress_stays_live_across_stages_and_finishes_once() {
        let output = SharedRunnerOutput::default();
        let surface = BatchProgressSurface::Terminal(TerminalProgressSurface::new(output.clone()));
        let mut progress = LiveBatchProgress::with_parts(SystemProgressClock, surface);
        let mut state = AppState::new();
        progress.paint(&state, 0, true);
        for msg in [
            Msg::PipelineRunRequested {
                scope: PipelineRunScope::Full,
            },
            Msg::PipelineRunAdvance,
        ] {
            (state, _) = harvester_core::update(state, msg);
            progress.paint(&state, 0, true);
        }
        let before_finish = output.text();
        assert_eq!(before_finish.matches("\u{1b}[?25l").count(), 1);
        assert!(!before_finish.contains("\u{1b}[?25h"));
        progress.finish();
        progress.finish();
        let finished = output.text();
        assert_eq!(finished.matches("\u{1b}[?25h").count(), 1);
        assert!(finished.ends_with("\u{1b}[?25h\n"));
    }
    #[test]
    fn stopping_counts_persist_through_final_paints() {
        let clock = ManualClock::new();
        let output = SharedRunnerOutput::default();
        let surface = BatchProgressSurface::Plain(PlainProgressReporter::new(output.clone()));
        let mut progress = LiveBatchProgress::with_parts(&clock, surface);
        progress.set_stopping(true);
        let state = AppState::new();
        progress.paint(&state, 0, true);
        progress.set_stopping(false);
        clock.advance(Duration::from_secs(1));
        progress.paint(&state, 0, true);
        assert_eq!(output.text().matches("Triage: 0 done").count(), 2);
        assert!(!output.text().contains("to do"));
    }

    #[test]
    fn plain_progress_emits_stopping_once_and_suppresses_identical_forced_paints() {
        let clock = ManualClock::new();
        let output = SharedRunnerOutput::default();
        let surface = BatchProgressSurface::Plain(PlainProgressReporter::new(output.clone()));
        let mut progress = LiveBatchProgress::with_parts(&clock, surface);
        let state = AppState::new();
        progress.paint(&state, 0, true);
        progress.resume(&state, 0);
        progress.paint(&state, 0, true);
        assert_eq!(output.text().lines().count(), 1);
        progress.paint(&state, 10_000, false);
        assert_eq!(output.text().lines().count(), 1, "cost-only change waits");
        progress.paint(&state, 10_000, true);
        progress.resume(&state, 10_000);
        assert_eq!(
            output.text().lines().count(),
            2,
            "changed forced paint emits once"
        );
        progress.set_stopping(true);
        progress.paint(&state, 10_000, false);
        assert_eq!(
            output.text().lines().count(),
            3,
            "stopping emits immediately"
        );
        progress.paint(&state, 10_000, true);
        progress.paint(&state, 10_000, true);
        assert_eq!(
            output.text().lines().count(),
            3,
            "final identical paints are silent"
        );
        assert!(output
            .text()
            .contains("Stopping safely; Ctrl+C again exits immediately"));
    }
}
