use std::path::{Path, PathBuf};
use std::sync::{Arc, Condvar, Mutex};
use std::thread;
use std::time::{Duration, Instant};

use engine_logging::{engine_info, engine_warn};
use harvester_core::PersistenceSnapshot;

use crate::save_blacklist;
use crate::FileWriteObserver;

const DEBOUNCE_WINDOW: Duration = Duration::from_millis(350);
const MAX_FLUSH_INTERVAL: Duration = Duration::from_secs(2);

#[derive(Debug)]
struct PendingSnapshot {
    snapshot: PersistenceSnapshot,
    seq: u64,
    first_enqueued_at: Instant,
    last_updated_at: Instant,
    overwritten_count: u64,
}

struct PendingLinks {
    output: PathBuf,
    url: String,
    links: Vec<harvester_engine::ExtractedLink>,
    observer: Option<FileWriteObserver>,
}

#[derive(Default)]
struct WorkerState {
    pending: Option<PendingSnapshot>,
    links: std::collections::VecDeque<PendingLinks>,
    next_seq: u64,
    shutting_down: bool,
    message_sender: Option<std::sync::mpsc::Sender<harvester_core::Msg>>,
}

pub struct PersistenceWorker {
    shared: Arc<(Mutex<WorkerState>, Condvar)>,
    join_handle: Option<thread::JoinHandle<()>>,
}

impl PersistenceWorker {
    pub(crate) fn enqueue_links(
        &self,
        output: PathBuf,
        url: String,
        links: Vec<harvester_engine::ExtractedLink>,
        observer: Option<FileWriteObserver>,
    ) {
        if links.is_empty() {
            return;
        }
        let (lock, condvar) = &*self.shared;
        lock.lock()
            .expect("persistence worker lock")
            .links
            .push_back(PendingLinks {
                output,
                url,
                links,
                observer,
            });
        condvar.notify_one();
    }
    pub fn set_message_sender(&self, sender: std::sync::mpsc::Sender<harvester_core::Msg>) {
        self.shared
            .0
            .lock()
            .expect("persistence worker lock")
            .message_sender = Some(sender);
    }
    pub fn new(state_path: PathBuf, blacklist_path: PathBuf) -> Self {
        Self::with_file_write_observer(state_path, blacklist_path, None)
    }

    pub fn new_with_file_write_observer(
        state_path: PathBuf,
        blacklist_path: PathBuf,
        observer: FileWriteObserver,
    ) -> Self {
        Self::with_file_write_observer(state_path, blacklist_path, Some(observer))
    }

    fn with_file_write_observer(
        state_path: PathBuf,
        blacklist_path: PathBuf,
        observer: Option<FileWriteObserver>,
    ) -> Self {
        let shared = Arc::new((Mutex::new(WorkerState::default()), Condvar::new()));
        let worker_shared = Arc::clone(&shared);
        let join_handle =
            thread::spawn(move || run_worker(worker_shared, state_path, blacklist_path, observer));
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
    observer: Option<FileWriteObserver>,
) {
    let mut last_flush_at = Instant::now();
    loop {
        let (pending, links) = {
            let (lock, condvar) = &*shared;
            let mut state = lock.lock().expect("persistence worker lock");
            loop {
                if !state.links.is_empty() {
                    break;
                }
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
            // Link arrivals should not defeat snapshot coalescing. Drain them
            // promptly, then return to the normal debounce unless shutting down.
            let pending = if !state.links.is_empty() && !state.shutting_down {
                None
            } else {
                state.pending.take()
            };
            (pending, std::mem::take(&mut state.links))
        };

        // Link effects are FIFO and never coalesced. Publish them before the
        // runtime snapshot captured after those completions, including at shutdown.
        for pending in links {
            crate::article_links::store_observed(
                &pending.output,
                &pending.url,
                &pending.links,
                &pending.observer,
            );
        }

        let Some(pending) = pending else {
            continue;
        };
        let state_write_started = Instant::now();
        engine_info!(
            "[persist] flushing seq={} path={} jobs={} queue_delay_ms={}",
            pending.seq,
            state_path.display(),
            pending.snapshot.completed.len(),
            state_write_started
                .saturating_duration_since(pending.first_enqueued_at)
                .as_millis()
        );
        let sender = shared
            .0
            .lock()
            .expect("persistence worker lock")
            .message_sender
            .clone();
        if let Err(error) = crate::persistence::persist_snapshot_with_notices(
            &state_path,
            &pending.snapshot.completed,
            &pending.snapshot.pending_intake,
            pending.snapshot.fetch_time_recovery_done,
            |message| {
                if let Some(sender) = &sender {
                    let _ = sender.send(harvester_core::Msg::RuntimeStateNotice { message });
                }
            },
        ) {
            engine_warn!(
                "[persist] failed to save runtime state {}: {}",
                state_path.display(),
                error
            );
        } else {
            observe_write(&observer, &state_path, state_write_started.elapsed());
        }
        let blacklist_write_started = Instant::now();
        if let Err(err) = save_blacklist(&blacklist_path, &pending.snapshot.blacklist) {
            engine_warn!("[persist] failed to save blacklist: {}", err);
        } else {
            observe_write(
                &observer,
                &blacklist_path,
                blacklist_write_started.elapsed(),
            );
        }
        let flush_started = state_write_started;
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

fn observe_write(observer: &Option<FileWriteObserver>, path: &Path, elapsed: Duration) {
    if let Some(observer) = observer {
        let bytes = std::fs::metadata(path)
            .map(|metadata| metadata.len())
            .unwrap_or(0);
        observer(path, bytes, elapsed);
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn enqueue_and_model_completion_do_not_wait_for_an_in_flight_save() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(".harvester_state.ron");
        let (entered_tx, entered_rx) = std::sync::mpsc::channel();
        let gate = std::sync::Arc::new((std::sync::Mutex::new(false), std::sync::Condvar::new()));
        let writer_gate = gate.clone();
        let observer: crate::FileWriteObserver = std::sync::Arc::new(move |_, _, _| {
            let _ = entered_tx.send(());
            let (lock, changed) = &*writer_gate;
            let mut released = lock.lock().unwrap();
            while !*released {
                released = changed.wait(released).unwrap();
            }
        });
        let worker = super::PersistenceWorker::new_with_file_write_observer(
            path.clone(),
            dir.path().join(".domain_blacklist.ron"),
            observer,
        );
        worker.enqueue(harvester_core::PersistenceSnapshot::capture(
            &harvester_core::AppState::new(),
        ));
        entered_rx
            .recv_timeout(std::time::Duration::from_secs(5))
            .unwrap();
        let (messages, replies) = std::sync::mpsc::channel();
        let runner = crate::EffectRunner::new(
            crate::RuntimePaths::with_defaults(dir.path().to_path_buf()),
            messages,
            Box::new(crate::NoOpPlatformHandler),
            Box::new(worker),
        );
        let (done, finished) = std::sync::mpsc::channel();
        let handle = std::thread::spawn(move || {
            runner.enqueue(vec![
                harvester_core::Effect::PersistRuntimeState {
                    snapshot: harvester_core::PersistenceSnapshot::capture(
                        &harvester_core::AppState::new(),
                    ),
                },
                harvester_core::Effect::RequestLlmCompletion {
                    request_id: 1,
                    prompt_id: harvester_engine::llm::prompt::PromptId::ArticleTriage,
                    prompt_version: None,
                    input_content: "fixture".into(),
                    context: vec![],
                    extra_template_vars: vec![],
                },
            ]);
            done.send(()).unwrap();
            runner
        });
        let responsive = finished
            .recv_timeout(std::time::Duration::from_secs(1))
            .is_ok();
        let reply = replies.recv_timeout(std::time::Duration::from_secs(1));
        *gate.0.lock().unwrap() = true;
        gate.1.notify_all();
        drop(handle.join().unwrap());
        assert!(responsive, "effect loop waited for the persistence worker");
        assert!(matches!(
            reply,
            Ok(harvester_core::Msg::LlmCompleted { request_id: 1, .. })
        ));
    }
    use std::path::Path;
    use std::thread;
    use std::time::Duration;

    use harvester_core::{update, Msg};
    use harvester_engine::FetchOutcomeClass;

    use super::*;
    use crate::{load_blacklist, load_completed_jobs};

    fn state_path(dir: &Path) -> PathBuf {
        dir.join(".harvester_state.ron")
    }

    #[test]
    fn latest_wins_and_shutdown_flushes_latest_snapshot() {
        let temp = tempfile::tempdir().expect("tempdir");
        let path = state_path(temp.path());
        let mut worker =
            PersistenceWorker::new(path.clone(), temp.path().join(".domain_blacklist.ron"));
        let first_jobs = vec![harvester_core::CompletedJobSnapshot {
            url: "https://example.com/a".to_string(),
            tokens: Some(1),
            bytes: Some(1),
            links: vec![],
            fetched_utc: None,
        }];
        let second_jobs = vec![
            first_jobs[0].clone(),
            harvester_core::CompletedJobSnapshot {
                url: "https://example.com/b".to_string(),
                tokens: Some(2),
                bytes: Some(2),
                links: vec![],
                fetched_utc: None,
            },
        ];
        let (first_state, _) = update(
            harvester_core::AppState::new(),
            Msg::RestoreCompletedJobs(first_jobs),
        );
        let (second_state, _) = update(
            harvester_core::AppState::new(),
            Msg::RestoreCompletedJobs(second_jobs.clone()),
        );

        worker.enqueue(PersistenceSnapshot::capture(&first_state));
        worker.enqueue(PersistenceSnapshot::capture(&second_state));
        worker.shutdown();

        assert_eq!(load_completed_jobs(&path), second_jobs);
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
        let t0 = chrono::DateTime::from_timestamp(0, 0).unwrap();
        let mut blacklist = harvester_core::blacklist::BlacklistState::default();
        blacklist.record_outcome(
            "example.com",
            FetchOutcomeClass::PermanentBlock,
            Some("http status 403"),
            t0,
        );
        let mut state = state;
        state.set_blacklist(blacklist.clone());

        let snapshot = PersistenceSnapshot::capture(&state);
        assert_eq!(snapshot.completed.len(), 1);
        assert_eq!(snapshot.blacklist, blacklist);
    }

    #[test]
    fn blacklist_entry_survives_worker_shutdown() {
        let temp = tempfile::tempdir().expect("tempdir");
        let s_path = state_path(temp.path());
        let bl_path = temp.path().join(".domain_blacklist.ron");
        let mut worker = PersistenceWorker::new(s_path.clone(), bl_path.clone());

        let t0 = chrono::DateTime::from_timestamp(0, 0).unwrap();
        let mut blacklist = harvester_core::blacklist::BlacklistState::default();
        for _ in 0..3 {
            blacklist.record_outcome(
                "example.com",
                FetchOutcomeClass::PermanentBlock,
                Some("http status 403"),
                t0,
            );
        }
        let snapshot = PersistenceSnapshot {
            fetch_time_recovery_done: false,
            completed: vec![],
            pending_intake: vec![],
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

    #[test]
    fn queued_links_publish_in_order_before_runtime_snapshot_and_shutdown_drains_them() {
        let dir = tempfile::tempdir().unwrap();
        let writes = Arc::new(Mutex::new(Vec::<PathBuf>::new()));
        let observed = writes.clone();
        let observer: FileWriteObserver =
            Arc::new(move |path, _, _| observed.lock().unwrap().push(path.to_path_buf()));
        let mut worker = PersistenceWorker::new_with_file_write_observer(
            state_path(dir.path()),
            dir.path().join(".domain_blacklist.ron"),
            observer.clone(),
        );
        let url = "https://example.com/article";
        for target in ["https://example.com/first", "https://example.com/latest"] {
            worker.enqueue_links(
                dir.path().to_path_buf(),
                url.into(),
                vec![harvester_engine::ExtractedLink {
                    url: target.into(),
                    text: None,
                    kind: harvester_engine::LinkKind::Hyperlink,
                }],
                Some(observer.clone()),
            );
        }
        let (state, _) = update(
            harvester_core::AppState::new(),
            Msg::RestoreCompletedJobs(vec![harvester_core::CompletedJobSnapshot {
                url: url.into(),
                tokens: None,
                bytes: None,
                links: vec![],
                fetched_utc: None,
            }]),
        );
        worker.enqueue(PersistenceSnapshot::capture(&state));
        worker.shutdown();
        assert_eq!(
            crate::load_article_links(dir.path(), url)[0].url,
            "https://example.com/latest"
        );
        assert_eq!(load_completed_jobs(&state_path(dir.path())).len(), 1);
        let writes = writes.lock().unwrap();
        assert_eq!(writes.len(), 4);
        assert_eq!(writes[0], writes[1]);
        assert_eq!(writes[0].extension().unwrap(), "json");
        assert_eq!(writes[2], state_path(dir.path()));
    }
}
