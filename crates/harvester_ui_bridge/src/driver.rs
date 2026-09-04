use std::panic::{catch_unwind, AssertUnwindSafe};
use std::sync::{mpsc, Arc, RwLock};
use std::time::Duration;

use chrono::{DateTime, Utc};
use engine_logging::engine_error;
use harvester_core::{AppState, ArchiveTokenEstimates, Effect, Msg, SignalCandidateDialogDefault};
use harvester_io::{
    host_bootstrap::pump_pre_triage_refresh, requires_persistence_snapshot, PersistenceSnapshot,
};
use serde::{Deserialize, Serialize};

use crate::snapshot::{project, BodyTable, ProjectedSnapshot, SnapshotEnvelope};

pub const SNAPSHOT_MIN_INTERVAL_MS: u64 = 50;

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

struct PendingSnapshot {
    snapshot: ProjectedSnapshot,
    bodies: BodyTable,
}

pub struct SnapshotCoalescer {
    last_emitted_at: Option<Duration>,
    next_generation: u64,
    pending: Option<PendingSnapshot>,
}

impl Default for SnapshotCoalescer {
    fn default() -> Self {
        Self {
            last_emitted_at: None,
            next_generation: 1,
            pending: None,
        }
    }
}

impl SnapshotCoalescer {
    pub fn push(
        &mut self,
        snapshot: ProjectedSnapshot,
        bodies: BodyTable,
        now: Duration,
    ) -> Option<(SnapshotEnvelope, BodyTable)> {
        if self.is_due(now) {
            Some(self.assign(snapshot, bodies, now))
        } else {
            self.pending = Some(PendingSnapshot { snapshot, bodies });
            None
        }
    }

    pub fn flush_due(&mut self, now: Duration) -> Option<(SnapshotEnvelope, BodyTable)> {
        if !self.is_due(now) {
            return None;
        }
        let pending = self.pending.take()?;
        Some(self.assign(pending.snapshot, pending.bodies, now))
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

    fn assign(
        &mut self,
        snapshot: ProjectedSnapshot,
        bodies: BodyTable,
        now: Duration,
    ) -> (SnapshotEnvelope, BodyTable) {
        self.pending = None;
        let generation = self.next_generation;
        self.next_generation = self.next_generation.saturating_add(1);
        self.last_emitted_at = Some(now);
        (snapshot.with_generation(generation), bodies)
    }
}

#[allow(clippy::too_many_arguments)]
pub fn run_driver<R, ES, CS, SS, PS, N>(
    mut state: AppState,
    receiver: mpsc::Receiver<Msg>,
    reducer: R,
    mut effect_sink: ES,
    mut command_sink: CS,
    mut snapshot_sink: SS,
    mut persistence_sink: PS,
    bodies: Arc<RwLock<BodyTable>>,
    mut now: N,
) -> DriverTermination
where
    R: Fn(AppState, Msg) -> (AppState, Vec<Effect>),
    ES: FnMut(Vec<Effect>),
    CS: FnMut(UiCommand),
    SS: FnMut(SnapshotSignal),
    PS: FnMut(PersistenceSnapshot),
    N: FnMut() -> Duration,
{
    let mut last_message_kind = "startup".to_string();
    let body = AssertUnwindSafe(|| {
        let mut coalescer = SnapshotCoalescer::default();
        let mut last_view = None;
        loop {
            let first = if coalescer.has_pending() {
                match receiver.recv_timeout(coalescer.remaining_until_due(now())) {
                    Ok(message) => message,
                    Err(mpsc::RecvTimeoutError::Timeout) => {
                        if let Some((snapshot, table)) = coalescer.flush_due(now()) {
                            *bodies.write().expect("body table lock") = table;
                            snapshot_sink(SnapshotSignal::Snapshot(snapshot));
                        }
                        continue;
                    }
                    Err(mpsc::RecvTimeoutError::Disconnected) => return Err(()),
                }
            } else {
                receiver.recv().map_err(|_| ())?
            };
            let mut messages = vec![first];
            messages.extend(receiver.try_iter());
            for message in messages {
                last_message_kind = message_kind(&message);
                let persist = requires_persistence_snapshot(&message);
                let (next, effects) = reducer(state, message);
                state = next;
                if persist {
                    persistence_sink(PersistenceSnapshot::capture(&state));
                }
                dispatch_effects(effects, &mut effect_sink, &mut command_sink);
            }
            let (next, effects, _) = pump_pre_triage_refresh(state);
            state = next;
            dispatch_effects(effects, &mut effect_sink, &mut command_sink);
            let view = state.view();
            if last_view.as_ref() != Some(&view) {
                let (snapshot, table) = project(&view);
                if let Some((snapshot, table)) = coalescer.push(snapshot, table, now()) {
                    *bodies.write().expect("body table lock") = table;
                    snapshot_sink(SnapshotSignal::Snapshot(snapshot));
                }
                last_view = Some(view);
            }
            if let Some((snapshot, table)) = coalescer.flush_due(now()) {
                *bodies.write().expect("body table lock") = table;
                snapshot_sink(SnapshotSignal::Snapshot(snapshot));
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

fn message_kind(message: &Msg) -> String {
    let debug = format!("{message:?}");
    let end = debug
        .find(|character: char| character == '{' || character == '(' || character.is_whitespace())
        .unwrap_or(debug.len());
    debug[..end].to_string()
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::mpsc;
    use std::thread;

    use super::*;

    fn projected(value: u64) -> ProjectedSnapshot {
        ProjectedSnapshot {
            schema_version: crate::IPC_SCHEMA_VERSION,
            view: serde_json::json!({ "value": value }),
        }
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
        let (runner, commands) = partition_effects(vec![effect, Effect::LoadEntityIndex]);
        assert_eq!(runner, vec![Effect::LoadEntityIndex]);
        assert!(
            matches!(commands.as_slice(), [UiCommand::ShowArchiveDialog(request)] if request.token_estimates == ArchiveTokenEstimates::default())
        );
    }

    #[test]
    fn coalescing_keeps_the_last_burst_state_and_monotonic_generation() {
        let mut coalescer = SnapshotCoalescer::default();
        let first = coalescer
            .push(projected(1), BodyTable::new(), Duration::ZERO)
            .unwrap()
            .0;
        assert_eq!(first.generation, 1);
        assert!(coalescer
            .push(projected(2), BodyTable::new(), Duration::from_millis(1))
            .is_none());
        assert!(coalescer
            .push(projected(3), BodyTable::new(), Duration::from_millis(2))
            .is_none());
        assert!(coalescer.flush_due(Duration::from_millis(49)).is_none());
        let last = coalescer.flush_due(Duration::from_millis(50)).unwrap().0;
        assert_eq!(last.generation, 2);
        assert_eq!(last.view["value"], 3);
    }

    #[test]
    fn direct_emission_discards_an_older_deferred_snapshot() {
        let mut coalescer = SnapshotCoalescer::default();
        let first = coalescer
            .push(projected(1), BodyTable::new(), Duration::ZERO)
            .unwrap()
            .0;
        assert_eq!(first.generation, 1);
        assert!(coalescer
            .push(projected(2), BodyTable::new(), Duration::from_millis(10))
            .is_none());

        let latest = coalescer
            .push(projected(3), BodyTable::new(), Duration::from_millis(60))
            .unwrap()
            .0;
        assert_eq!(latest.generation, 2);
        assert_eq!(latest.view["value"], 3);
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
                |_| {},
                Arc::new(RwLock::new(BodyTable::new())),
                move || match driver_clock_calls.fetch_add(1, Ordering::SeqCst) {
                    0..=1 => Duration::ZERO,
                    2..=4 => Duration::from_millis(49),
                    _ => Duration::from_millis(50),
                },
            )
        });

        sender.send(Msg::NoOp).unwrap();
        let first = signal_receiver
            .recv_timeout(Duration::from_millis(20))
            .unwrap();
        assert!(matches!(
            first,
            SnapshotSignal::Snapshot(SnapshotEnvelope { generation: 1, .. })
        ));

        sender
            .send(Msg::WorkspaceViewSet {
                view: harvester_core::WorkspaceView::Blacklist,
            })
            .unwrap();
        let trailing = signal_receiver
            .recv_timeout(Duration::from_millis(20))
            .unwrap();
        assert!(matches!(
            trailing,
            SnapshotSignal::Snapshot(SnapshotEnvelope { generation: 2, ref view, .. })
                if view["workspace_view"] == "Blacklist"
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
            |_| {},
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
    fn driver_persists_successful_job_completion_but_not_tick() {
        let (sender, receiver) = mpsc::channel();
        sender.send(Msg::tick_at(DateTime::UNIX_EPOCH)).unwrap();
        sender
            .send(Msg::JobDone {
                job_id: 1,
                result: harvester_core::JobResultKind::Success,
                content_preview: None,
                extracted_links: Vec::new(),
                fetched_utc: None,
            })
            .unwrap();
        drop(sender);
        let mut persisted = 0;
        let _ = run_driver(
            AppState::default(),
            receiver,
            harvester_core::update,
            |_| {},
            |_| {},
            |_| {},
            |_| persisted += 1,
            Arc::new(RwLock::new(BodyTable::new())),
            || Duration::from_millis(100),
        );
        assert_eq!(persisted, 1);
    }
}
