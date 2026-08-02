use super::drain_control::{decide_batch_wait, BatchWaitDecision};
use crate::batch_coordinator::BatchPeek;
use crate::progress::{
    BatchDisplayPhase, BatchProgressProjection, BatchProgressSnapshot, BatchRunBaseline,
    PassCounts, PlainProgressReporter, ProgressClock, ProgressGlyphs, ProjectionContext,
    SystemProgressClock, TerminalProgressSurface,
};
use engine_logging::engine_warn;
use harvester_core::AppState;
use std::io::Write;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

pub(super) const BATCH_WAIT_INTERVAL: Duration = Duration::from_secs(5 * 60);
const PROGRESS_REFRESH_INTERVAL: Duration = Duration::from_millis(250);
const PLAIN_PROGRESS_HEARTBEAT_INTERVAL: Duration = Duration::from_secs(60);
const LOCAL_WAIT_REFRESH_INTERVAL: Duration = Duration::from_secs(1);
const SHUTDOWN_POLL_INTERVAL: Duration = Duration::from_millis(500);

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
    pub(super) peeks: Vec<BatchPeek>,
    last_provider_check: Option<Instant>,
    next_provider_check: Option<Instant>,
    pub(super) last_provider_check_local: Option<chrono::DateTime<chrono::FixedOffset>>,
    next_provider_check_local: Option<chrono::DateTime<chrono::FixedOffset>>,
    pass_counts: PassCounts,
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
            peeks: Vec::new(),
            last_provider_check: None,
            next_provider_check: None,
            last_provider_check_local: None,
            next_provider_check_local: None,
            pass_counts: PassCounts::default(),
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

    pub(super) fn set_provider_check(&mut self, peeks: Vec<BatchPeek>) {
        self.peeks = retain_last_successful_provider_counts(&self.peeks, peeks);
        let now = self.clock.monotonic_now();
        let local_now = self.clock.wall_now();
        let wait_interval = chrono::Duration::from_std(BATCH_WAIT_INTERVAL)
            .expect("batch wait interval fits chrono duration");
        self.last_provider_check = Some(now);
        self.next_provider_check = Some(now + BATCH_WAIT_INTERVAL);
        self.last_provider_check_local = Some(local_now);
        self.next_provider_check_local = Some(local_now + wait_interval);
    }

    pub(super) fn set_provider_wait_render(&mut self, render: &ProviderWaitRender) {
        self.set_phase(render.phase);
        self.peeks = retain_last_successful_provider_counts(&self.peeks, render.peeks.clone());
        self.last_provider_check = render.last_provider_check;
        self.next_provider_check = render.next_provider_check;
        self.last_provider_check_local = render.last_provider_check_local;
        self.next_provider_check_local = render.next_provider_check_local;
    }

    fn snapshot(
        &mut self,
        state: &AppState,
        cost_this_run_microdollars: u64,
    ) -> BatchProgressSnapshot {
        self.projection.snapshot(
            &state.batch_observation(),
            &self.peeks,
            ProjectionContext {
                phase_override: self.phase_override,
                pass_counts: self.pass_counts,
                cost_this_run_microdollars,
                last_provider_check: self.last_provider_check,
                next_provider_check: self.next_provider_check,
                last_provider_check_local: self.last_provider_check_local,
                next_provider_check_local: self.next_provider_check_local,
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

    pub(super) fn record_pass(&mut self, collect_only: bool) {
        self.pass_counts.intake_passes = 1;
        if collect_only {
            self.pass_counts.collection_passes =
                self.pass_counts.collection_passes.saturating_add(1);
        }
    }
}

/// Retains the last successful provider counts when a status lookup fails.
/// The current failed lookup remains in the result, so the pure formatter keeps
/// showing its retry/indeterminate state rather than pretending the old result
/// was fresh.
fn retain_last_successful_provider_counts(
    previous: &[BatchPeek],
    current: Vec<BatchPeek>,
) -> Vec<BatchPeek> {
    let mut retained: Vec<_> = previous
        .iter()
        .filter(|peek| peek.status.is_some() && peek.request_counts.is_some())
        .cloned()
        .collect();
    retained.retain(|old| {
        !current
            .iter()
            .any(|new| new.batch_id == old.batch_id && new.status.is_some())
    });
    retained.extend(current);
    retained
}

#[derive(Debug, Clone)]
pub(super) struct ProviderWaitRender {
    pub(super) phase: BatchDisplayPhase,
    pub(super) peeks: Vec<BatchPeek>,
    pub(super) last_provider_check: Option<Instant>,
    pub(super) next_provider_check: Option<Instant>,
    pub(super) last_provider_check_local: Option<chrono::DateTime<chrono::FixedOffset>>,
    pub(super) next_provider_check_local: Option<chrono::DateTime<chrono::FixedOffset>>,
}

pub(super) enum ProviderWaitOutcome {
    Collect(Vec<BatchPeek>),
    Shutdown,
}

/// Waits locally between existing provider peeks. The injected clock makes the
/// heartbeat cadence and shutdown polling deterministic without changing
/// coordinator transport or provider-check frequency.
pub(super) fn run_provider_wait_loop<C, P, R>(
    clock: &C,
    shutdown_flag: &AtomicBool,
    mut latest_peeks: Vec<BatchPeek>,
    mut peek: P,
    mut render: R,
) -> ProviderWaitOutcome
where
    C: ProgressClock,
    P: FnMut() -> Vec<BatchPeek>,
    R: FnMut(ProviderWaitRender),
{
    let mut last_provider_check = None;
    let mut next_provider_check = None;
    let mut last_provider_check_local = None;
    let mut next_provider_check_local = None;
    loop {
        render(ProviderWaitRender {
            phase: BatchDisplayPhase::CheckingProvider,
            peeks: latest_peeks.clone(),
            last_provider_check,
            next_provider_check,
            last_provider_check_local,
            next_provider_check_local,
        });
        latest_peeks = peek();
        // A signal cannot be observed during the synchronous provider call,
        // but it must win as soon as that atomic call returns.
        if shutdown_flag.load(Ordering::Relaxed) {
            return ProviderWaitOutcome::Shutdown;
        }
        if decide_batch_wait(&latest_peeks) == BatchWaitDecision::RunCollectCycle {
            return ProviderWaitOutcome::Collect(latest_peeks);
        }

        let checked_at = clock.monotonic_now();
        let next_check = checked_at + BATCH_WAIT_INTERVAL;
        let checked_at_local = clock.wall_now();
        let next_check_local = checked_at_local
            + chrono::Duration::from_std(BATCH_WAIT_INTERVAL)
                .expect("batch wait interval fits chrono duration");
        last_provider_check = Some(checked_at);
        next_provider_check = Some(next_check);
        last_provider_check_local = Some(checked_at_local);
        next_provider_check_local = Some(next_check_local);
        let mut next_refresh = checked_at;
        loop {
            if shutdown_flag.load(Ordering::Relaxed) {
                return ProviderWaitOutcome::Shutdown;
            }
            let now = clock.monotonic_now();
            if now >= next_check {
                break;
            }
            if now >= next_refresh {
                render(ProviderWaitRender {
                    phase: BatchDisplayPhase::WaitingForProvider,
                    peeks: latest_peeks.clone(),
                    last_provider_check,
                    next_provider_check,
                    last_provider_check_local,
                    next_provider_check_local,
                });
                next_refresh = now + LOCAL_WAIT_REFRESH_INTERVAL;
            }
            let until_refresh = next_refresh.saturating_duration_since(now);
            let until_check = next_check.saturating_duration_since(now);
            let sleep_for = SHUTDOWN_POLL_INTERVAL.min(until_refresh).min(until_check);
            if !sleep_for.is_zero() {
                clock.sleep(sleep_for);
            }
        }
    }
}

/// Local-only retry delay used by the no-progress safeguard. It deliberately
/// shares the same 500 ms shutdown polling and one-second presentation cadence
/// as the provider wait, while making no provider calls.
pub(super) fn wait_with_local_heartbeat<C, R>(
    clock: &C,
    shutdown_flag: &AtomicBool,
    duration: Duration,
    mut render: R,
) -> bool
where
    C: ProgressClock,
    R: FnMut(),
{
    let deadline = clock.monotonic_now() + duration;
    let mut next_refresh = clock.monotonic_now();
    loop {
        if shutdown_flag.load(Ordering::Relaxed) {
            return true;
        }
        let now = clock.monotonic_now();
        if now >= deadline {
            return false;
        }
        if now >= next_refresh {
            render();
            next_refresh = now + LOCAL_WAIT_REFRESH_INTERVAL;
        }
        let sleep_for = SHUTDOWN_POLL_INTERVAL
            .min(next_refresh.saturating_duration_since(now))
            .min(deadline.saturating_duration_since(now));
        if !sleep_for.is_zero() {
            clock.sleep(sleep_for);
        }
    }
}

#[cfg(test)]
pub(super) fn batch_peek(
    status: Option<openai_provider_kit::BatchLifecycle>,
    completed: u32,
    total: u32,
) -> BatchPeek {
    let request_counts = status
        .as_ref()
        .map(|_| openai_provider_kit::BatchRequestCounts {
            total,
            completed,
            failed: 0,
        });
    BatchPeek {
        batch_id: "batch-test".to_string(),
        stage: harvester_core::StageKind::Triage,
        status,
        request_counts,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
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
    fn ascii_progress_selects_ascii_glyphs_only_for_interactive_dashboard() {
        assert_eq!(progress_glyphs(true), ProgressGlyphs::Ascii);
        assert_eq!(progress_glyphs(false), ProgressGlyphs::Unicode);
    }

    #[test]
    fn provider_lookup_failure_retains_last_successful_counts_until_a_successful_retry() {
        let previous = vec![batch_peek(
            Some(openai_provider_kit::BatchLifecycle::InProgress),
            4,
            10,
        )];
        let failed_lookup = vec![batch_peek(None, 0, 0)];
        let retained = retain_last_successful_provider_counts(&previous, failed_lookup);
        assert_eq!(retained.len(), 2);
        assert!(retained.iter().any(|peek| {
            peek.status == Some(openai_provider_kit::BatchLifecycle::InProgress)
                && peek
                    .request_counts
                    .as_ref()
                    .is_some_and(|counts| counts.completed == 4)
        }));
        assert!(retained.iter().any(|peek| peek.status.is_none()));

        let recovered = retain_last_successful_provider_counts(
            &retained,
            vec![batch_peek(
                Some(openai_provider_kit::BatchLifecycle::InProgress),
                6,
                10,
            )],
        );
        assert_eq!(recovered.len(), 1);
        assert_eq!(recovered[0].request_counts.as_ref().unwrap().completed, 6);
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
    fn terminal_progress_stays_live_across_collection_passes_and_finishes_once() {
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

        progress.record_pass(false);
        progress.set_phase(BatchDisplayPhase::Intake);
        progress.paint(&state, 0, true);
        for _ in 0..3 {
            progress.record_pass(true);
            progress.set_phase(BatchDisplayPhase::Collecting);
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

    #[test]
    fn provider_wait_uses_second_heartbeats_without_extra_peeks_between_deadlines() {
        let wall = chrono::FixedOffset::east_opt(2 * 60 * 60)
            .unwrap()
            .with_ymd_and_hms(2026, 7, 23, 9, 43, 30)
            .single()
            .unwrap();
        let clock = ManualWaitClock::new(wall);
        let shutdown = AtomicBool::new(false);
        let mut peek_calls = 0usize;
        let mut heartbeats = 0usize;
        let mut checking = 0usize;
        let mut checked_at_local = None;
        let mut next_check_local = None;
        let result = run_provider_wait_loop(
            &clock,
            &shutdown,
            Vec::new(),
            || {
                peek_calls += 1;
                if peek_calls == 1 {
                    vec![batch_peek(
                        Some(openai_provider_kit::BatchLifecycle::InProgress),
                        1,
                        2,
                    )]
                } else {
                    vec![batch_peek(
                        Some(openai_provider_kit::BatchLifecycle::Completed),
                        2,
                        2,
                    )]
                }
            },
            |event| match event.phase {
                BatchDisplayPhase::WaitingForProvider => {
                    heartbeats += 1;
                    checked_at_local = event.last_provider_check_local;
                    next_check_local = event.next_provider_check_local;
                }
                BatchDisplayPhase::CheckingProvider => checking += 1,
                _ => unreachable!(),
            },
        );

        assert!(matches!(result, ProviderWaitOutcome::Collect(_)));
        assert_eq!(peek_calls, 2, "one initial and one deadline check only");
        assert_eq!(checking, 2);
        assert_eq!(
            heartbeats, 300,
            "one local refresh per second for five minutes"
        );
        assert_eq!(
            checked_at_local
                .unwrap()
                .format("%Y-%m-%d %H:%M:%S %:z")
                .to_string(),
            "2026-07-23 09:43:30 +02:00"
        );
        assert_eq!(
            next_check_local
                .unwrap()
                .format("%Y-%m-%d %H:%M:%S %:z")
                .to_string(),
            "2026-07-23 09:48:30 +02:00"
        );
    }

    #[test]
    fn provider_wait_shutdown_is_observed_at_the_half_second_local_poll_boundary() {
        let wall = chrono::FixedOffset::east_opt(0)
            .unwrap()
            .with_ymd_and_hms(2026, 7, 23, 9, 43, 30)
            .single()
            .unwrap();
        let clock = ManualWaitClock::new(wall);
        let shutdown = AtomicBool::new(false);
        let mut rendered_wait = false;
        let outcome = run_provider_wait_loop(
            &clock,
            &shutdown,
            Vec::new(),
            || {
                vec![batch_peek(
                    Some(openai_provider_kit::BatchLifecycle::InProgress),
                    0,
                    1,
                )]
            },
            |event| {
                if event.phase == BatchDisplayPhase::WaitingForProvider && !rendered_wait {
                    rendered_wait = true;
                    shutdown.store(true, std::sync::atomic::Ordering::Relaxed);
                }
            },
        );
        assert!(matches!(outcome, ProviderWaitOutcome::Shutdown));
        assert!(clock.elapsed.get() <= Duration::from_millis(500));
    }

    #[test]
    fn provider_wait_marks_checking_before_a_blocking_peek_and_observes_shutdown_after_it_returns()
    {
        let wall = chrono::FixedOffset::east_opt(0)
            .unwrap()
            .with_ymd_and_hms(2026, 7, 23, 9, 43, 30)
            .single()
            .unwrap();
        let clock = ManualWaitClock::new(wall);
        let shutdown = AtomicBool::new(false);
        let checking_rendered = Cell::new(false);
        let outcome = run_provider_wait_loop(
            &clock,
            &shutdown,
            Vec::new(),
            || {
                assert!(
                    checking_rendered.get(),
                    "CheckingProvider must paint before the peek"
                );
                clock.sleep(Duration::from_secs(5));
                shutdown.store(true, std::sync::atomic::Ordering::Relaxed);
                vec![batch_peek(
                    Some(openai_provider_kit::BatchLifecycle::InProgress),
                    0,
                    1,
                )]
            },
            |event| {
                if event.phase == BatchDisplayPhase::CheckingProvider {
                    checking_rendered.set(true);
                }
            },
        );

        assert!(matches!(outcome, ProviderWaitOutcome::Shutdown));
        assert!(checking_rendered.get());
        assert_eq!(clock.elapsed.get(), Duration::from_secs(5));
    }

    #[test]
    fn local_heartbeat_wait_observes_shutdown_within_half_a_second() {
        let wall = chrono::FixedOffset::east_opt(0)
            .unwrap()
            .with_ymd_and_hms(2026, 7, 23, 9, 43, 30)
            .single()
            .unwrap();
        let clock = ManualWaitClock::new(wall);
        let shutdown = AtomicBool::new(false);
        let renders = Cell::new(0usize);

        let interrupted =
            wait_with_local_heartbeat(&clock, &shutdown, Duration::from_secs(30), || {
                renders.set(renders.get() + 1);
                shutdown.store(true, std::sync::atomic::Ordering::Relaxed);
            });

        assert!(interrupted);
        assert_eq!(renders.get(), 1);
        assert!(
            clock.elapsed.get() <= SHUTDOWN_POLL_INTERVAL,
            "shutdown must be observed within the local poll boundary"
        );
    }
}
