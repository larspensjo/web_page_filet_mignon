use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, Condvar, Mutex};
use std::thread;
use std::time::{Duration, Instant};

use engine_logging::{engine_info, engine_warn};
use harvester_core::blacklist::BlacklistState;
use harvester_core::{AppState, ArticleFilterKey, CompletedJobSnapshot, ManualDecision};

use crate::{persist_runtime_state, save_blacklist};

const DEBOUNCE_WINDOW: Duration = Duration::from_millis(350);
const MAX_FLUSH_INTERVAL: Duration = Duration::from_secs(2);

#[derive(Debug, Clone)]
pub struct PersistenceSnapshot {
    pub completed: Vec<CompletedJobSnapshot>,
    pub pre_triage_overrides: HashMap<ArticleFilterKey, ManualDecision>,
    pub blacklist: BlacklistState,
}

impl PersistenceSnapshot {
    pub fn capture(state: &AppState) -> Self {
        Self {
            completed: state.completed_jobs_snapshot(),
            pre_triage_overrides: state.pre_triage_manual_overrides().clone(),
            blacklist: state.blacklist().clone(),
        }
    }
}

#[derive(Debug)]
struct PendingSnapshot {
    snapshot: PersistenceSnapshot,
    seq: u64,
    first_enqueued_at: Instant,
    last_updated_at: Instant,
    overwritten_count: u64,
}

#[derive(Debug, Default)]
struct WorkerState {
    pending: Option<PendingSnapshot>,
    next_seq: u64,
    shutting_down: bool,
}

pub struct PersistenceWorker {
    shared: Arc<(Mutex<WorkerState>, Condvar)>,
    join_handle: Option<thread::JoinHandle<()>>,
}

impl PersistenceWorker {
    pub fn new(state_path: PathBuf, blacklist_path: PathBuf) -> Self {
        let shared = Arc::new((Mutex::new(WorkerState::default()), Condvar::new()));
        let worker_shared = Arc::clone(&shared);
        let join_handle =
            thread::spawn(move || run_worker(worker_shared, state_path, blacklist_path));
        Self {
            shared,
            join_handle: Some(join_handle),
        }
    }

    pub fn enqueue(&self, snapshot: PersistenceSnapshot) {
        let (lock, condvar) = &*self.shared;
        let mut state = lock.lock().expect("persistence worker lock");
        state.next_seq = state.next_seq.saturating_add(1);
        let seq = state.next_seq.max(1);
        let now = Instant::now();
        let pending = match state.pending.take() {
            Some(existing) => PendingSnapshot {
                snapshot,
                seq,
                first_enqueued_at: existing.first_enqueued_at,
                last_updated_at: now,
                overwritten_count: existing.overwritten_count.saturating_add(1),
            },
            None => PendingSnapshot {
                snapshot,
                seq,
                first_enqueued_at: now,
                last_updated_at: now,
                overwritten_count: 0,
            },
        };
        engine_info!(
            "[persist] enqueued seq={} overwritten_count={}",
            pending.seq,
            pending.overwritten_count
        );
        state.pending = Some(pending);
        condvar.notify_one();
    }

    pub fn shutdown(&mut self) {
        if self.join_handle.is_none() {
            return;
        }
        {
            let (lock, condvar) = &*self.shared;
            let mut state = lock.lock().expect("persistence worker lock");
            state.shutting_down = true;
            condvar.notify_one();
        }
        if let Some(join_handle) = self.join_handle.take() {
            let _ = join_handle.join();
        }
    }
}

impl Drop for PersistenceWorker {
    fn drop(&mut self) {
        self.shutdown();
    }
}

fn run_worker(
    shared: Arc<(Mutex<WorkerState>, Condvar)>,
    state_path: PathBuf,
    blacklist_path: PathBuf,
) {
    let mut last_flush_at = Instant::now();
    loop {
        let pending = {
            let (lock, condvar) = &*shared;
            let mut state = lock.lock().expect("persistence worker lock");
            loop {
                match (&state.pending, state.shutting_down) {
                    (None, false) => {
                        state = condvar.wait(state).expect("wait");
                    }
                    (None, true) => return,
                    (Some(_), true) => break,
                    (Some(existing), false) => {
                        let now = Instant::now();
                        let debounce_wait = DEBOUNCE_WINDOW
                            .checked_sub(now.saturating_duration_since(existing.last_updated_at));
                        let max_wait = MAX_FLUSH_INTERVAL
                            .checked_sub(now.saturating_duration_since(last_flush_at));
                        if let (Some(wait), Some(max_wait_remaining)) = (debounce_wait, max_wait) {
                            let sleep_for = wait.min(max_wait_remaining);
                            let (guard, timeout) = condvar
                                .wait_timeout(state, sleep_for)
                                .expect("wait_timeout");
                            state = guard;
                            if !timeout.timed_out() {
                                continue;
                            }
                        }
                        break;
                    }
                }
            }
            state.pending.take()
        };

        let Some(pending) = pending else {
            continue;
        };
        let flush_started = Instant::now();
        persist_runtime_state(
            &state_path,
            &pending.snapshot.completed,
            &pending.snapshot.pre_triage_overrides,
        );
        if let Err(err) = save_blacklist(&blacklist_path, &pending.snapshot.blacklist) {
            engine_warn!("[persist] failed to save blacklist: {}", err);
        }
        let flush_latency_ms = flush_started.elapsed().as_millis();
        engine_info!(
            "[persist] flushed seq={} queue_delay_ms={} flush_latency_ms={} overwritten_count={}",
            pending.seq,
            flush_started
                .saturating_duration_since(pending.first_enqueued_at)
                .as_millis(),
            flush_latency_ms,
            pending.overwritten_count
        );
        if pending.overwritten_count > 0 {
            engine_info!(
                "[persist] coalesced seq={} overwritten_count={}",
                pending.seq,
                pending.overwritten_count
            );
        }
        last_flush_at = Instant::now();
    }
}

#[cfg(test)]
mod tests {
    use std::path::Path;
    use std::thread;
    use std::time::Duration;

    use harvester_core::{update, ArticleFilterKey, ManualDecision, Msg};
    use harvester_engine::FetchOutcomeClass;

    use super::*;
    use crate::{load_blacklist, load_completed_jobs, load_pre_triage_overrides};

    fn state_path(dir: &Path) -> PathBuf {
        dir.join(".harvester_state.ron")
    }

    #[test]
    fn latest_wins_and_shutdown_flushes_latest_snapshot() {
        let temp = tempfile::tempdir().expect("tempdir");
        let path = state_path(temp.path());
        let mut worker =
            PersistenceWorker::new(path.clone(), temp.path().join(".domain_blacklist.ron"));

        let first = PersistenceSnapshot::capture(&harvester_core::AppState::new());
        let (state_with_overrides, _) = update(
            harvester_core::AppState::new(),
            Msg::PreTriageOverridesHydrated {
                overrides: HashMap::from([(
                    ArticleFilterKey {
                        url: "https://example.com/a".to_string(),
                        content_hash: 1,
                    },
                    ManualDecision::Include,
                )]),
            },
        );
        let second = PersistenceSnapshot::capture(&state_with_overrides);

        worker.enqueue(first);
        worker.enqueue(second);
        worker.shutdown();

        let overrides = load_pre_triage_overrides(&path);
        assert_eq!(overrides.len(), 1);
    }

    #[test]
    fn worker_persists_even_without_explicit_shutdown_once_debounce_elapsed() {
        let temp = tempfile::tempdir().expect("tempdir");
        let path = state_path(temp.path());
        let worker =
            PersistenceWorker::new(path.clone(), temp.path().join(".domain_blacklist.ron"));

        let (state_with_completed, _) = update(
            harvester_core::AppState::new(),
            Msg::RestoreCompletedJobs(vec![harvester_core::CompletedJobSnapshot {
                url: "https://example.com/x".to_string(),
                tokens: Some(1),
                bytes: Some(1),
                links: vec![],
                fetched_utc: None,
            }]),
        );
        let snapshot = PersistenceSnapshot::capture(&state_with_completed);
        worker.enqueue(snapshot);
        thread::sleep(Duration::from_millis(500));

        let completed = load_completed_jobs(&path);
        assert_eq!(completed.len(), 1);
    }

    #[test]
    fn capture_builds_full_snapshot_from_app_state() {
        let (state, _) = update(
            harvester_core::AppState::new(),
            Msg::RestoreCompletedJobs(vec![harvester_core::CompletedJobSnapshot {
                url: "https://example.com/x".to_string(),
                tokens: Some(1),
                bytes: Some(1),
                links: vec![],
                fetched_utc: None,
            }]),
        );
        let (state, _) = update(
            state,
            Msg::PreTriageOverridesHydrated {
                overrides: HashMap::from([(
                    ArticleFilterKey {
                        url: "https://example.com/x".to_string(),
                        content_hash: 1,
                    },
                    ManualDecision::Exclude,
                )]),
            },
        );

        let snapshot = PersistenceSnapshot::capture(&state);
        assert_eq!(snapshot.completed.len(), 1);
        assert_eq!(snapshot.pre_triage_overrides.len(), 1);
    }

    #[test]
    fn blacklist_entry_survives_worker_shutdown() {
        let temp = tempfile::tempdir().expect("tempdir");
        let s_path = state_path(temp.path());
        let bl_path = temp.path().join(".domain_blacklist.ron");
        let mut worker = PersistenceWorker::new(s_path.clone(), bl_path.clone());

        let t0 = chrono::DateTime::from_timestamp(0, 0).unwrap();
        let mut blacklist = BlacklistState::default();
        for _ in 0..3 {
            blacklist.record_outcome(
                "example.com",
                FetchOutcomeClass::PermanentBlock,
                Some("http status 403"),
                t0,
            );
        }
        let snapshot = PersistenceSnapshot {
            completed: vec![],
            pre_triage_overrides: HashMap::new(),
            blacklist,
        };
        worker.enqueue(snapshot);
        worker.shutdown();

        let loaded = load_blacklist(&bl_path);
        assert!(
            loaded.is_blocked("example.com", t0),
            "3-strike entry must survive shutdown and reload"
        );
    }
}
