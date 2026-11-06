use std::panic::{catch_unwind, AssertUnwindSafe};
use std::sync::{mpsc, Arc, RwLock};
use std::time::{Duration, Instant};

use chrono::{DateTime, Utc};
use engine_logging::engine_error;
use harvester_core::{
    AppState, AppViewModel, ArchiveTokenEstimates, Effect, Msg, SignalCandidateDialogDefault,
};
use harvester_io::host_bootstrap::pump_pre_triage_refresh;
use serde::{Deserialize, Serialize};

#[cfg(test)]
use crate::snapshot::ProjectedSnapshot;
use crate::snapshot::{project, BodyTable, SnapshotEnvelope};

pub const SNAPSHOT_MIN_INTERVAL_MS: u64 = 50;

/// The Last 24h job-list window advances through tick-driven view rebuilds
/// (DecisionLog 2026-09-16); refreshing once per second preserves that coarse
/// movement while bounding how stale the view can become.
pub const CLOCK_ONLY_VIEW_REFRESH_MS: u64 = 1_000;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum UiCommand {
    ShowArchiveDialog(ArchiveDialogRequest),
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ArchiveDialogRequest {
    pub request_id: u64,
    pub article_count: usize,
    pub since_utc: Option<DateTime<Utc>>,
    pub default_basename: String,
    pub default_file_exists: bool,
    pub export_dir: std::path::PathBuf,
    pub pending_pre_triage_count: usize,
    pub token_estimates: ArchiveTokenEstimates,
    pub signal_candidate_default: SignalCandidateDialogDefault,
    pub signal_candidate_count: usize,
    pub signal_candidate_scoring_done: u32,
    pub signal_candidate_scoring_total: u32,
    pub signal_candidate_token_estimates: ArchiveTokenEstimates,
}

#[derive(Debug, Clone, PartialEq)]
pub enum SnapshotSignal {
    Snapshot(SnapshotEnvelope),
    Fatal { message: String },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DriverTermination {
    Clean,
    Fatal,
}

/// Wall time spent in one pass through the desktop driver loop. Effect hand-off
/// includes any work performed synchronously by the supplied effect sink.
#[derive(Debug, Default, Clone, Copy)]
pub struct DriverIterationTiming {
    pub wall: Duration,
    pub reduce: Duration,
    pub pre_triage_pump: Duration,
    pub effect_hand_off: Duration,
    pub view_build: Duration,
    pub view_compare: Duration,
    pub snapshot: Duration,
    pub idle_recv: Duration,
}

pub fn partition_effects(effects: Vec<Effect>) -> (Vec<Effect>, Vec<UiCommand>) {
    let mut runner = Vec::new();
    let mut commands = Vec::new();
    for effect in effects {
        match effect {
            Effect::ShowArchiveDialog {
                request_id,
                article_count,
                since_utc,
                default_basename,
                default_file_exists,
                export_dir,
                pending_pre_triage_count,
                token_estimates,
                signal_candidate_default,
                signal_candidate_count,
                signal_candidate_scoring_done,
                signal_candidate_scoring_total,
                signal_candidate_token_estimates,
            } => commands.push(UiCommand::ShowArchiveDialog(ArchiveDialogRequest {
                request_id,
                article_count,
                since_utc,
                default_basename,
                default_file_exists,
                export_dir,
                pending_pre_triage_count,
                token_estimates,
                signal_candidate_default,
                signal_candidate_count,
                signal_candidate_scoring_done,
                signal_candidate_scoring_total,
                signal_candidate_token_estimates,
            })),
            other => runner.push(other),
        }
    }
    (runner, commands)
}

type PendingSnapshot = Arc<AppViewModel>;

pub struct SnapshotCoalescer {
    last_emitted_at: Option<Duration>,
    next_generation: u64,
    pending: Option<PendingSnapshot>,
    #[cfg(test)]
    projector: fn(&AppViewModel) -> (ProjectedSnapshot, BodyTable),
}

impl Default for SnapshotCoalescer {
    fn default() -> Self {
        Self {
            last_emitted_at: None,
            next_generation: 1,
            pending: None,
            #[cfg(test)]
            projector: project,
        }
    }
}

impl SnapshotCoalescer {
    pub fn push(
        &mut self,
        view: Arc<AppViewModel>,
        now: Duration,
    ) -> Option<(SnapshotEnvelope, BodyTable)> {
        if self.is_due(now) {
            Some(self.assign(view, now))
        } else {
            self.pending = Some(view);
            None
        }
    }

    pub fn flush_due(&mut self, now: Duration) -> Option<(SnapshotEnvelope, BodyTable)> {
        if !self.is_due(now) {
            return None;
        }
        let pending = self.pending.take()?;
        Some(self.assign(pending, now))
    }

    fn is_due(&self, now: Duration) -> bool {
        self.last_emitted_at.is_none_or(|last| {
            now.saturating_sub(last) >= Duration::from_millis(SNAPSHOT_MIN_INTERVAL_MS)
        })
    }

    fn has_pending(&self) -> bool {
        self.pending.is_some()
    }

    fn remaining_until_due(&self, now: Duration) -> Duration {
        let interval = Duration::from_millis(SNAPSHOT_MIN_INTERVAL_MS);
        self.last_emitted_at.map_or(Duration::ZERO, |last| {
            interval.saturating_sub(now.saturating_sub(last))
        })
    }

    fn assign(&mut self, view: Arc<AppViewModel>, now: Duration) -> (SnapshotEnvelope, BodyTable) {
        self.pending = None;
        let generation = self.next_generation;
        self.next_generation = self.next_generation.saturating_add(1);
        self.last_emitted_at = Some(now);
        #[cfg(not(test))]
        let (snapshot, bodies) = project(view.as_ref());
        #[cfg(test)]
        let (snapshot, bodies) = (self.projector)(view.as_ref());
        (snapshot.with_generation(generation), bodies)
    }

    #[cfg(test)]
    fn with_projector(projector: fn(&AppViewModel) -> (ProjectedSnapshot, BodyTable)) -> Self {
        Self {
            projector,
            ..Self::default()
        }
    }
}

#[allow(clippy::too_many_arguments)]
pub fn run_driver<R, ES, CS, SS, N>(
    state: AppState,
    receiver: mpsc::Receiver<Msg>,
    reducer: R,
    effect_sink: ES,
    command_sink: CS,
    snapshot_sink: SS,
    bodies: Arc<RwLock<BodyTable>>,
    now: N,
) -> DriverTermination
where
    R: Fn(AppState, Msg) -> (AppState, Vec<Effect>),
    ES: FnMut(Vec<Effect>),
    CS: FnMut(UiCommand),
    SS: FnMut(SnapshotSignal),
    N: FnMut() -> Duration,
{
    run_driver_with_observers(
        state,
        receiver,
        reducer,
        effect_sink,
        command_sink,
        snapshot_sink,
        bodies,
        now,
        |_, _, _| {},
        |_| {},
        None::<fn(DriverIterationTiming)>,
        |_| false,
    )
}

#[allow(clippy::too_many_arguments)]
pub fn run_driver_with_observers<R, ES, CS, SS, N, RO, VO, IO, STOP>(
    mut state: AppState,
    receiver: mpsc::Receiver<Msg>,
    reducer: R,
    mut effect_sink: ES,
    mut command_sink: CS,
    mut snapshot_sink: SS,
    bodies: Arc<RwLock<BodyTable>>,
    mut now: N,
    mut reducer_observer: RO,
    mut view_observer: VO,
    mut iteration_observer: Option<IO>,
    mut should_terminate: STOP,
) -> DriverTermination
where
    R: Fn(AppState, Msg) -> (AppState, Vec<Effect>),
    ES: FnMut(Vec<Effect>),
    CS: FnMut(UiCommand),
    SS: FnMut(SnapshotSignal),
    N: FnMut() -> Duration,
    RO: FnMut(&str, Duration, &AppState),
    VO: FnMut(Duration),
    IO: FnMut(DriverIterationTiming),
    STOP: FnMut(&AppState) -> bool,
{
    // Timings live at the host boundary, keeping the reducer deterministic.
    macro_rules! measure_phase {
        ($timing:ident, $field:ident, $work:expr) => {{
            let started = Instant::now();
            let result = $work;
            let elapsed = started.elapsed();
            $timing.$field += elapsed;
            if elapsed > Duration::from_millis(250) && stringify!($field) != "idle_recv" {
                engine_logging::engine_info!(
                    "[driver] host=desktop operation={} elapsed_ms={}",
                    stringify!($field),
                    elapsed.as_millis()
                );
            }
            result
        }};
    }
    let mut last_message_kind = "startup".to_string();
    let body = AssertUnwindSafe(|| {
        let mut coalescer = SnapshotCoalescer::default();
        let mut last_view: Option<Arc<AppViewModel>> = None;
        let mut last_view_built_at: Option<Duration> = None;
        loop {
            let iteration_started = iteration_observer.as_ref().map(|_| Instant::now());
            let mut timing = DriverIterationTiming::default();
            let received = measure_phase!(
                timing,
                idle_recv,
                if coalescer.has_pending() {
                    receiver.recv_timeout(coalescer.remaining_until_due(now()))
                } else {
                    receiver
                        .recv()
                        .map_err(|_| mpsc::RecvTimeoutError::Disconnected)
                }
            );
            let first = match received {
                Ok(message) => message,
                Err(mpsc::RecvTimeoutError::Timeout) => {
                    measure_phase!(timing, snapshot, {
                        if let Some((snapshot, table)) = coalescer.flush_due(now()) {
                            *bodies.write().expect("body table lock") = table;
                            snapshot_sink(SnapshotSignal::Snapshot(snapshot));
                        }
                    });
                    if let Some(started) = iteration_started {
                        timing.wall = started.elapsed();
                        iteration_observer.as_mut().expect("iteration observer")(timing);
                    }
                    continue;
                }
                Err(mpsc::RecvTimeoutError::Disconnected) => return Err(()),
            };
            let mut messages = vec![first];
            messages.extend(receiver.try_iter());
            let clock_only_batch = messages
                .iter()
                .all(|message| matches!(message, Msg::Tick { .. }));
            let mut reducer_effects_emitted = false;
            for message in messages {
                last_message_kind = message.kind().to_owned();
                let reducer_started = Instant::now();
                let (next, effects) = measure_phase!(timing, reduce, reducer(state, message));
                let elapsed = reducer_started.elapsed();
                if elapsed > Duration::from_millis(250) {
                    engine_logging::engine_info!(
                        "[driver] host=desktop message={} elapsed_ms={}",
                        last_message_kind,
                        elapsed.as_millis()
                    );
                }
                reducer_observer(&last_message_kind, elapsed, &next);
                state = next;
                reducer_effects_emitted |= !effects.is_empty();
                measure_phase!(
                    timing,
                    effect_hand_off,
                    dispatch_effects(effects, &mut effect_sink, &mut command_sink)
                );
            }
            let refresh_started = Instant::now();
            let (next, effects, refresh_triggered) =
                measure_phase!(timing, pre_triage_pump, pump_pre_triage_refresh(state));
            if refresh_triggered {
                reducer_observer("EvaluatePreTriageRefresh", refresh_started.elapsed(), &next);
            }
            state = next;
            let pre_triage_effects_emitted = !effects.is_empty();
            measure_phase!(
                timing,
                effect_hand_off,
                dispatch_effects(effects, &mut effect_sink, &mut command_sink)
            );
            let pipeline_advance_ran = !matches!(
                state.pipeline_run_phase(),
                harvester_core::PipelineRunPhase::Idle
            );
            if pipeline_advance_ran {
                last_message_kind = "PipelineRunAdvance".to_string();
                let reducer_started = Instant::now();
                let (next, effects) =
                    measure_phase!(timing, reduce, reducer(state, Msg::PipelineRunAdvance));
                reducer_observer(&last_message_kind, reducer_started.elapsed(), &next);
                state = next;
                reducer_effects_emitted |= !effects.is_empty();
                measure_phase!(
                    timing,
                    effect_hand_off,
                    dispatch_effects(effects, &mut effect_sink, &mut command_sink)
                );
            }
            let clock_only = clock_only_batch
                && !reducer_effects_emitted
                && !refresh_triggered
                && !pre_triage_effects_emitted
                && !pipeline_advance_ran
                && matches!(
                    state.pipeline_run_phase(),
                    harvester_core::PipelineRunPhase::Idle
                );
            let should_build_view = !clock_only
                || last_view.is_none()
                || last_view_built_at.is_none_or(|last| {
                    now().saturating_sub(last) >= Duration::from_millis(CLOCK_ONLY_VIEW_REFRESH_MS)
                });
            let changed_view = if should_build_view {
                let view_started = Instant::now();
                let view = measure_phase!(timing, view_build, Arc::new(state.view()));
                view_observer(view_started.elapsed());
                let view_built_at = now();
                last_view_built_at = Some(view_built_at);
                let changed = measure_phase!(
                    timing,
                    view_compare,
                    last_view.as_deref() != Some(view.as_ref())
                );
                if changed {
                    last_view = Some(Arc::clone(&view));
                    Some((view, view_built_at))
                } else {
                    None
                }
            } else {
                None
            };
            measure_phase!(timing, snapshot, {
                if let Some((view, view_built_at)) = changed_view {
                    if let Some((snapshot, table)) = coalescer.push(view, view_built_at) {
                        *bodies.write().expect("body table lock") = table;
                        snapshot_sink(SnapshotSignal::Snapshot(snapshot));
                    }
                }
                if let Some((snapshot, table)) = coalescer.flush_due(now()) {
                    *bodies.write().expect("body table lock") = table;
                    snapshot_sink(SnapshotSignal::Snapshot(snapshot));
                }
            });
            if let Some(started) = iteration_started {
                timing.wall = started.elapsed();
                iteration_observer.as_mut().expect("iteration observer")(timing);
            }
            if should_terminate(&state) {
                return Ok(());
            }
        }
    });
    match catch_unwind(body) {
        Ok(Err(())) => fatal(
            &mut snapshot_sink,
            &last_message_kind,
            "core message channel disconnected",
        ),
        Ok(Ok(())) => DriverTermination::Clean,
        Err(_) => fatal(
            &mut snapshot_sink,
            &last_message_kind,
            "core reducer panicked",
        ),
    }
}

fn dispatch_effects<ES, CS>(effects: Vec<Effect>, effect_sink: &mut ES, command_sink: &mut CS)
where
    ES: FnMut(Vec<Effect>),
    CS: FnMut(UiCommand),
{
    let (runner, commands) = partition_effects(effects);
    if !runner.is_empty() {
        effect_sink(runner);
    }
    for command in commands {
        command_sink(command);
    }
}

fn fatal<S>(sink: &mut S, last_message_kind: &str, reason: &str) -> DriverTermination
where
    S: FnMut(SnapshotSignal),
{
    engine_error!("[ui-bridge] fatal core-thread termination last_message={last_message_kind} reason={reason}");
    sink(SnapshotSignal::Fatal { message: format!("Harvester stopped responding — see engine.log ({reason}; last message: {last_message_kind})") });
    DriverTermination::Fatal
}

#[cfg(test)]
mod tests {
    use std::cell::Cell;
    use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
    use std::sync::mpsc;
    use std::thread;

    use super::*;

    fn view(value: u64) -> Arc<AppViewModel> {
        Arc::new(AppViewModel {
            job_count: value as usize,
            ..Default::default()
        })
    }

    // Assumes one iteration per message: the test reducer leaves the view unchanged, so no pending snapshot causes a recv-timeout iteration.
    fn run_driver_steps<R>(steps: Vec<(u64, Msg)>, reducer: R) -> Vec<usize>
    where
        R: Fn(AppState, Msg) -> (AppState, Vec<Effect>) + Send + 'static,
    {
        let expected_steps = steps.len();
        let processed = Arc::new(AtomicUsize::new(0));
        let reducer_processed = Arc::clone(&processed);
        let counted_reducer = move |state, message| {
            let result = reducer(state, message);
            reducer_processed.fetch_add(1, Ordering::SeqCst);
            result
        };
        let clock_ms = Arc::new(AtomicU64::new(0));
        let driver_clock_ms = Arc::clone(&clock_ms);
        let view_builds = Arc::new(AtomicUsize::new(0));
        let driver_view_builds = Arc::clone(&view_builds);
        let (sender, receiver) = mpsc::channel();
        let (iteration_sender, iteration_receiver) = mpsc::channel();
        let driver = thread::spawn(move || {
            run_driver_with_observers(
                AppState::default(),
                receiver,
                counted_reducer,
                |_| {},
                |_| {},
                |_| {},
                Arc::new(RwLock::new(BodyTable::new())),
                move || Duration::from_millis(driver_clock_ms.load(Ordering::SeqCst)),
                |_, _, _| {},
                move |_| {
                    driver_view_builds.fetch_add(1, Ordering::SeqCst);
                },
                Some(move |_| {
                    iteration_sender
                        .send(())
                        .expect("test should observe every driver iteration");
                }),
                move |_| processed.load(Ordering::SeqCst) >= expected_steps,
            )
        });

        let mut builds_after_step = Vec::with_capacity(expected_steps);
        for (time_ms, message) in steps {
            clock_ms.store(time_ms, Ordering::SeqCst);
            sender
                .send(message)
                .expect("driver should receive test message");
            iteration_receiver
                .recv_timeout(Duration::from_secs(3))
                .expect("driver should finish the test iteration");
            builds_after_step.push(view_builds.load(Ordering::SeqCst));
        }
        drop(sender);
        assert_eq!(driver.join().unwrap(), DriverTermination::Clean);
        builds_after_step
    }

    fn tick() -> Msg {
        Msg::tick_at(DateTime::UNIX_EPOCH)
    }

    #[test]
    fn clock_only_ticks_refresh_at_most_once_per_second() {
        let builds = run_driver_steps(
            vec![
                (0, tick()),
                (100, tick()),
                (450, tick()),
                (999, tick()),
                (1_000, tick()),
            ],
            |state, _| (state, Vec::new()),
        );

        assert_eq!(builds, vec![1, 1, 1, 1, 2]);
    }

    #[test]
    fn non_tick_message_always_builds_a_view() {
        let builds = run_driver_steps(
            vec![
                (0, tick()),
                (
                    0,
                    Msg::JobListModeSet {
                        mode: harvester_core::JobListMode::Last24Hours,
                    },
                ),
            ],
            |state, _| (state, Vec::new()),
        );

        assert_eq!(builds, vec![1, 2]);
    }

    #[test]
    fn tick_with_reducer_effect_always_builds_a_view() {
        let calls = Arc::new(AtomicUsize::new(0));
        let reducer_calls = Arc::clone(&calls);
        let builds = run_driver_steps(vec![(0, tick()), (0, tick())], move |state, _| {
            if reducer_calls.fetch_add(1, Ordering::SeqCst) == 1 {
                (
                    state,
                    vec![Effect::ShowArchiveDialog {
                        request_id: 1,
                        article_count: 0,
                        since_utc: None,
                        default_basename: "archive".into(),
                        default_file_exists: false,
                        export_dir: Default::default(),
                        pending_pre_triage_count: 0,
                        token_estimates: Default::default(),
                        signal_candidate_default: SignalCandidateDialogDefault::OffEmpty,
                        signal_candidate_count: 0,
                        signal_candidate_scoring_done: 0,
                        signal_candidate_scoring_total: 0,
                        signal_candidate_token_estimates: Default::default(),
                    }],
                )
            } else {
                (state, Vec::new())
            }
        });

        assert_eq!(builds, vec![1, 2]);
    }

    #[test]
    fn first_clock_only_iteration_builds_a_view() {
        let builds = run_driver_steps(vec![(0, tick())], |state, _| (state, Vec::new()));

        assert_eq!(builds, vec![1]);
    }

    fn run_three_tick_iterations<R, RO>(state: AppState, reducer: R, observer: RO) -> Vec<usize>
    where
        R: Fn(AppState, Msg) -> (AppState, Vec<Effect>),
        RO: FnMut(&str, Duration, &AppState),
    {
        let (sender, receiver) = mpsc::channel();
        sender.send(tick()).unwrap();
        let iterations = Cell::new(0);
        let builds = Cell::new(0);
        let mut builds_after_iteration = Vec::new();
        let termination = run_driver_with_observers(
            state,
            receiver,
            reducer,
            |effects| panic!("unexpected runner effects: {effects:?}"),
            |command| panic!("unexpected UI command: {command:?}"),
            |_| {},
            Arc::new(RwLock::new(BodyTable::new())),
            // Each iteration is less than 1 s apart, but beyond the snapshot
            // coalescing interval so changed views cannot leave a pending flush.
            || Duration::from_millis(iterations.get() * 100),
            observer,
            |_| builds.set(builds.get() + 1),
            Some(|_| {
                builds_after_iteration.push(builds.get());
                iterations.set(iterations.get() + 1);
                if iterations.get() < 3 {
                    sender.send(tick()).unwrap();
                }
            }),
            |_| iterations.get() == 3,
        );
        assert_eq!(termination, DriverTermination::Clean);
        builds_after_iteration
    }

    #[test]
    fn ticks_during_active_pipeline_always_build_a_view() {
        let (state, _) = harvester_core::update(
            AppState::default(),
            Msg::AiAvailabilityDetected {
                availability: harvester_core::AiAvailability::Unavailable {
                    reason: harvester_core::AiUnavailableReason::MissingApiKey,
                },
            },
        );
        let (state, _) = harvester_core::update(
            state,
            Msg::PipelineRunRequested {
                scope: harvester_core::PipelineRunScope::Full,
            },
        );
        let advances = Cell::new(0);
        let builds = run_three_tick_iterations(state, harvester_core::update, |kind, _, state| {
            assert_ne!(
                state.pipeline_run_phase(),
                harvester_core::PipelineRunPhase::Idle
            );
            assert_ne!(kind, "EvaluatePreTriageRefresh");
            if kind == "PipelineRunAdvance" {
                advances.set(advances.get() + 1);
            }
        });

        assert_eq!(advances.get(), 3);
        assert_eq!(builds, vec![1, 2, 3]);
    }

    #[test]
    fn tick_with_pre_triage_refresh_always_builds_a_view() {
        let ticks = Cell::new(0);
        let refreshes = Cell::new(0);
        let builds = run_three_tick_iterations(
            AppState::default(),
            |mut state, message| {
                assert!(matches!(message, Msg::Tick { .. }));
                ticks.set(ticks.get() + 1);
                if ticks.get() == 2 {
                    // Arrange real refresh demand after a view already exists.
                    let (restored, effects) = harvester_core::update(
                        state,
                        Msg::RestoreCompletedJobs(vec![harvester_core::CompletedJobSnapshot {
                            url: "https://refresh.invalid/article".into(),
                            tokens: Some(10),
                            bytes: Some(100),
                            links: Vec::new(),
                            fetched_utc: None,
                        }]),
                    );
                    assert!(effects.is_empty());
                    state = restored;
                }
                harvester_core::update(state, message)
            },
            |kind, _, state| {
                assert_eq!(
                    state.pipeline_run_phase(),
                    harvester_core::PipelineRunPhase::Idle
                );
                if kind == "EvaluatePreTriageRefresh" {
                    refreshes.set(refreshes.get() + 1);
                }
            },
        );

        assert_eq!(refreshes.get(), 1);
        assert_eq!(builds, vec![1, 2, 2]);
    }

    #[test]
    fn archive_dialog_is_host_serviced() {
        let effect = Effect::ShowArchiveDialog {
            request_id: 1,
            article_count: 2,
            since_utc: None,
            default_basename: "archive".into(),
            default_file_exists: false,
            export_dir: Default::default(),
            pending_pre_triage_count: 0,
            token_estimates: Default::default(),
            signal_candidate_default: SignalCandidateDialogDefault::OffEmpty,
            signal_candidate_count: 0,
            signal_candidate_scoring_done: 0,
            signal_candidate_scoring_total: 0,
            signal_candidate_token_estimates: Default::default(),
        };
        let (runner, commands) = partition_effects(vec![effect, Effect::LoadBriefingCheckpoint]);
        assert_eq!(runner, vec![Effect::LoadBriefingCheckpoint]);
        assert!(
            matches!(commands.as_slice(), [UiCommand::ShowArchiveDialog(request)] if request.token_estimates == ArchiveTokenEstimates::default())
        );
    }

    #[test]
    fn coalescing_keeps_the_last_burst_state_and_monotonic_generation() {
        let mut coalescer = SnapshotCoalescer::default();
        let first = coalescer.push(view(1), Duration::ZERO).unwrap().0;
        assert_eq!(first.generation, 1);
        assert!(coalescer.push(view(2), Duration::from_millis(1)).is_none());
        assert!(coalescer.push(view(3), Duration::from_millis(2)).is_none());
        assert!(coalescer.flush_due(Duration::from_millis(49)).is_none());
        let last = coalescer.flush_due(Duration::from_millis(50)).unwrap().0;
        assert_eq!(last.generation, 2);
        assert_eq!(last.view["job_count"], 3);
    }

    #[test]
    fn bursts_project_once_per_emitted_envelope() {
        static PROJECTED: AtomicUsize = AtomicUsize::new(0);
        fn counting_project(view: &AppViewModel) -> (ProjectedSnapshot, BodyTable) {
            PROJECTED.fetch_add(1, Ordering::SeqCst);
            project(view)
        }

        PROJECTED.store(0, Ordering::SeqCst);
        let mut coalescer = SnapshotCoalescer::with_projector(counting_project);
        assert!(coalescer.push(view(1), Duration::ZERO).is_some());
        assert!(coalescer.push(view(2), Duration::from_millis(1)).is_none());
        assert!(coalescer.push(view(3), Duration::from_millis(2)).is_none());
        assert_eq!(PROJECTED.load(Ordering::SeqCst), 1);
        assert!(coalescer.flush_due(Duration::from_millis(50)).is_some());
        assert_eq!(PROJECTED.load(Ordering::SeqCst), 2);
    }

    #[test]
    fn direct_emission_discards_an_older_deferred_snapshot() {
        let mut coalescer = SnapshotCoalescer::default();
        let first = coalescer.push(view(1), Duration::ZERO).unwrap().0;
        assert_eq!(first.generation, 1);
        assert!(coalescer.push(view(2), Duration::from_millis(10)).is_none());

        let latest = coalescer
            .push(view(3), Duration::from_millis(60))
            .unwrap()
            .0;
        assert_eq!(latest.generation, 2);
        assert_eq!(latest.view["job_count"], 3);
        assert!(coalescer.flush_due(Duration::from_millis(120)).is_none());
    }

    #[test]
    fn driver_flushes_the_trailing_snapshot_without_another_message() {
        let (sender, receiver) = mpsc::channel();
        let (signal_sender, signal_receiver) = mpsc::channel();
        let clock_calls = Arc::new(AtomicUsize::new(0));
        let driver_clock_calls = Arc::clone(&clock_calls);
        let driver = thread::spawn(move || {
            run_driver(
                AppState::default(),
                receiver,
                harvester_core::update,
                |_| {},
                |_| {},
                move |signal| {
                    let _ = signal_sender.send(signal);
                },
                Arc::new(RwLock::new(BodyTable::new())),
                move || match driver_clock_calls.fetch_add(1, Ordering::SeqCst) {
                    0..=1 => Duration::ZERO,
                    2..=4 => Duration::from_millis(49),
                    _ => Duration::from_millis(50),
                },
            )
        });

        sender.send(Msg::PipelineRunAdvance).unwrap();
        let first = signal_receiver
            .recv_timeout(Duration::from_millis(20))
            .unwrap();
        assert!(matches!(
            first,
            SnapshotSignal::Snapshot(SnapshotEnvelope { generation: 1, .. })
        ));

        sender
            .send(Msg::JobListModeSet {
                mode: harvester_core::JobListMode::Last24Hours,
            })
            .unwrap();
        let trailing = signal_receiver
            .recv_timeout(Duration::from_millis(20))
            .unwrap();
        assert!(matches!(
            trailing,
            SnapshotSignal::Snapshot(SnapshotEnvelope { generation: 2, ref view, .. })
                if view["desktop_job_list"]["mode"] == "Last24Hours"
        ));

        drop(sender);
        assert_eq!(driver.join().unwrap(), DriverTermination::Fatal);
    }

    #[test]
    fn panicking_reducer_is_a_fatal_termination() {
        let (sender, receiver) = mpsc::channel();
        sender
            .send(Msg::ArchiveDialogSubmitted {
                request_id: 7,
                basename: "archive".into(),
                set_checkpoint: false,
                submitted_at: DateTime::UNIX_EPOCH,
                use_summaries: false,
                use_signal_candidates: false,
            })
            .unwrap();
        drop(sender);
        let mut signals = Vec::new();
        let result = run_driver(
            AppState::default(),
            receiver,
            |_, _| panic!("test panic"),
            |_| {},
            |_| {},
            |signal| signals.push(signal),
            Arc::new(RwLock::new(BodyTable::new())),
            || Duration::ZERO,
        );
        assert_eq!(result, DriverTermination::Fatal);
        assert!(matches!(
            signals.last(),
            Some(SnapshotSignal::Fatal { message })
                if message.contains("last message: ArchiveDialogSubmitted")
        ));
    }

    #[test]
    fn driver_forwards_reducer_emitted_persistence_effect() {
        let (sender, receiver) = mpsc::channel();
        sender.send(Msg::tick_at(DateTime::UNIX_EPOCH)).unwrap();
        sender
            .send(Msg::JobDone {
                job_id: 1,
                result: harvester_core::JobResultKind::Success,
                extracted_links: Vec::new(),
                fetched_utc: None,
            })
            .unwrap();
        drop(sender);
        let mut effects = Vec::new();
        let _ = run_driver(
            AppState::default(),
            receiver,
            harvester_core::update,
            |emitted| effects.extend(emitted),
            |_| {},
            |_| {},
            Arc::new(RwLock::new(BodyTable::new())),
            || Duration::from_millis(100),
        );
        assert_eq!(
            effects
                .iter()
                .filter(|effect| matches!(effect, Effect::PersistRuntimeState { .. }))
                .count(),
            1
        );
    }
}
