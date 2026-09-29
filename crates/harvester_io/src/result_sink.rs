//! One ordered, coalescing writer for reducer-emitted paid results.
use std::io;
use std::sync::{mpsc, Mutex};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use engine_logging::{engine_error, engine_info};
use harvester_core::{Msg, SavedResult};

use crate::{result_store::AppendFile, FileWriteObserver, RuntimePaths};

pub const RESULT_SAVE_WINDOW: Duration = Duration::from_secs(2);

pub trait ResultSink: Send + Sync {
    fn enqueue(&self, records: Vec<SavedResult>);
    fn flush(&self) -> io::Result<()>;
}

pub struct NoOpResultSink;
impl ResultSink for NoOpResultSink {
    fn enqueue(&self, _: Vec<SavedResult>) {}
    fn flush(&self) -> io::Result<()> {
        Ok(())
    }
}

enum Command {
    Records(Vec<SavedResult>),
    Flush(mpsc::Sender<io::Result<()>>),
    Shutdown,
    #[cfg(test)]
    Crash,
}

pub struct CoalescingResultSink {
    tx: mpsc::Sender<Command>,
    worker: Mutex<Option<JoinHandle<()>>>,
}

impl CoalescingResultSink {
    pub fn new(
        paths: RuntimePaths,
        messages: mpsc::Sender<Msg>,
        observer: Option<FileWriteObserver>,
    ) -> Self {
        Self::with_window(paths, messages, observer, RESULT_SAVE_WINDOW)
    }

    fn with_window(
        paths: RuntimePaths,
        messages: mpsc::Sender<Msg>,
        observer: Option<FileWriteObserver>,
        window: Duration,
    ) -> Self {
        let (tx, rx) = mpsc::channel();
        let worker = thread::spawn(move || run(rx, paths, messages, observer, window));
        Self {
            tx,
            worker: Mutex::new(Some(worker)),
        }
    }
}

impl ResultSink for CoalescingResultSink {
    fn enqueue(&self, records: Vec<SavedResult>) {
        if !records.is_empty() && self.tx.send(Command::Records(records)).is_err() {
            engine_error!("[results] result sink stopped before accepting records");
        }
    }
    fn flush(&self) -> io::Result<()> {
        let (tx, rx) = mpsc::channel();
        self.tx
            .send(Command::Flush(tx))
            .map_err(|e| io::Error::other(e.to_string()))?;
        rx.recv().map_err(|e| io::Error::other(e.to_string()))?
    }
}

impl Drop for CoalescingResultSink {
    fn drop(&mut self) {
        let _ = self.tx.send(Command::Shutdown);
        if let Some(worker) = self
            .worker
            .get_mut()
            .unwrap_or_else(|e| e.into_inner())
            .take()
        {
            if worker.join().is_err() {
                engine_error!("[results] result sink panicked during shutdown");
            }
        }
    }
}

#[derive(Default)]
struct Writers {
    triage: Option<AppendFile>,
    summary: Option<AppendFile>,
    signal: Option<AppendFile>,
    triage_reported: bool,
    summary_reported: bool,
    signal_reported: bool,
}

#[derive(Default)]
struct Pending {
    triage: Vec<SavedResult>,
    summary: Vec<SavedResult>,
    signal: Vec<SavedResult>,
}

impl Pending {
    fn is_empty(&self) -> bool {
        self.triage.is_empty() && self.summary.is_empty() && self.signal.is_empty()
    }

    fn extend(&mut self, records: Vec<SavedResult>) {
        for record in records {
            match record {
                SavedResult::Triage(..) => self.triage.push(record),
                SavedResult::Summary(..) => self.summary.push(record),
                SavedResult::SignalCandidate(..) => self.signal.push(record),
            }
        }
    }
}

fn save(
    writers: &mut Writers,
    paths: &RuntimePaths,
    pending: &mut Pending,
    messages: &mpsc::Sender<Msg>,
    observer: &Option<FileWriteObserver>,
) -> io::Result<()> {
    // Preserve arrival order within each kind, including multiple versions of one key.
    let mut first_error = None;
    macro_rules! append {
        ($slot:ident, $reported:ident, $path:ident, $legacy:path, $variant:ident, $key:ty, $entry:ty) => {
            if !pending.$slot.is_empty() {
                let records: Vec<_> = pending
                    .$slot
                    .iter()
                    .map(|record| {
                        let SavedResult::$variant(key, entry) = record else {
                            unreachable!()
                        };
                        (key, entry)
                    })
                    .collect();
                let started = Instant::now();
                let result = (|| {
                    if writers.$slot.is_none() {
                        writers.$slot = Some(AppendFile::open::<_, _, $key, $entry>(
                            &paths.$path,
                            $legacy,
                        )?);
                    }
                    writers
                        .$slot
                        .as_mut()
                        .expect("opened writer")
                        .append::<_, _, $key, $entry>(&records)
                })();
                match result {
                    Ok(()) => {
                        if let Some(observer) = observer {
                            let bytes: usize = records
                                .iter()
                                .map(|r| serde_json::to_vec(r).map_or(0, |b| b.len() + 1))
                                .sum();
                            observer(&paths.$path, bytes as u64, started.elapsed());
                        }
                        engine_info!(
                            "[results] appended {} records to {}",
                            records.len(),
                            paths.$path.display()
                        );
                        pending.$slot.clear();
                    }
                    Err(e) => {
                        // Reopen on retry to recover a partial last line. A complete
                        // record in this failed kind may repeat under later-lines-win.
                        writers.$slot = None;
                        if !writers.$reported {
                            let reason = e.to_string();
                            engine_error!("[results] refusing further AI work: {}", reason);
                            let _ = messages.send(Msg::ResultStoreUnavailable { reason });
                            writers.$reported = true;
                        }
                        if first_error.is_none() {
                            first_error = Some(e);
                        }
                    }
                }
            }
        };
    }
    append!(
        triage,
        triage_reported,
        triage_cache_path,
        crate::triage_cache_store::legacy,
        Triage,
        crate::triage_cache_store::PersistedTriageCacheKey,
        crate::triage_cache_store::PersistedTriageEntry
    );
    append!(
        summary,
        summary_reported,
        summary_cache_path,
        crate::summary_cache_store::legacy,
        Summary,
        crate::summary_cache_store::PersistedCacheKey,
        crate::summary_cache_store::PersistedCacheEntry
    );
    append!(
        signal,
        signal_reported,
        signal_candidate_cache_path,
        crate::signal_candidate_cache_store::legacy,
        SignalCandidate,
        crate::signal_candidate_cache_store::PersistedKey,
        crate::signal_candidate_cache_store::PersistedEntry
    );
    first_error.map_or(Ok(()), Err)
}

fn run(
    rx: mpsc::Receiver<Command>,
    paths: RuntimePaths,
    messages: mpsc::Sender<Msg>,
    observer: Option<FileWriteObserver>,
    window: Duration,
) {
    let mut writers = Writers::default();
    let mut pending = Pending::default();
    let mut deadline: Option<Instant> = None;
    let flush = |writers: &mut Writers, pending: &mut Pending| {
        if pending.is_empty() {
            return Ok(());
        }
        save(writers, &paths, pending, &messages, &observer)
    };
    loop {
        let command = if let Some(at) = deadline {
            if Instant::now() >= at {
                let _ = flush(&mut writers, &mut pending);
                deadline = if pending.is_empty() {
                    None
                } else {
                    Some(Instant::now() + window)
                };
                continue;
            }
            rx.recv_timeout(at.saturating_duration_since(Instant::now()))
        } else {
            rx.recv().map_err(|_| mpsc::RecvTimeoutError::Disconnected)
        };
        match command {
            #[cfg(test)]
            Ok(Command::Crash) => break,
            Ok(Command::Records(records)) => {
                if pending.is_empty() {
                    deadline = Some(Instant::now() + window);
                }
                pending.extend(records);
            }
            Ok(Command::Flush(reply)) => {
                let result = flush(&mut writers, &mut pending);
                deadline = if pending.is_empty() {
                    None
                } else {
                    Some(Instant::now() + window)
                };
                let _ = reply.send(result);
            }
            Ok(Command::Shutdown) | Err(mpsc::RecvTimeoutError::Disconnected) => {
                let _ = flush(&mut writers, &mut pending);
                break;
            }
            Err(mpsc::RecvTimeoutError::Timeout) => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    fn record() -> SavedResult {
        let path = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../harvester_batch/tests/fixtures/carry_over/.summary_cache.ron");
        let (key, entry) = crate::summary_cache_store::legacy(&path).unwrap().remove(0);
        SavedResult::Summary(key, entry)
    }
    fn count(paths: &RuntimePaths) -> usize {
        crate::load_summary_cache(&paths.summary_cache_path)
            .unwrap()
            .len()
    }
    fn with_hash(mut record: SavedResult, hash: &str) -> SavedResult {
        if let SavedResult::Summary(key, _) = &mut record {
            key.content_hash = hash.to_owned();
        }
        record
    }

    #[test]
    fn coalesces_records_within_two_seconds_of_first_unsaved_record() {
        let dir = tempfile::tempdir().unwrap();
        let paths = RuntimePaths::with_defaults(dir.path().to_owned());
        let (messages, _) = mpsc::channel();
        let (written, writes) = mpsc::channel();
        let observer: FileWriteObserver = std::sync::Arc::new(move |_, _, _| {
            let _ = written.send(Instant::now());
        });
        let sink = CoalescingResultSink::new(paths.clone(), messages, Some(observer));
        let started = Instant::now();
        sink.enqueue(vec![with_hash(record(), "first")]);
        sink.enqueue(vec![with_hash(record(), "second")]);
        assert!(
            !paths.summary_cache_path.exists(),
            "enqueue should coalesce, not write synchronously"
        );
        let saved_at = writes.recv_timeout(Duration::from_secs(5)).unwrap();
        assert!(saved_at.duration_since(started) >= Duration::from_millis(1900));
        assert!(saved_at.duration_since(started) < Duration::from_secs(5));
        assert_eq!(count(&paths), 2);
        assert!(writes.try_recv().is_err(), "one append for the burst");
    }

    #[test]
    fn continued_arrivals_do_not_postpone_the_first_record_deadline() {
        let dir = tempfile::tempdir().unwrap();
        let paths = RuntimePaths::with_defaults(dir.path().to_owned());
        let (messages, _) = mpsc::channel();
        let (written, writes) = mpsc::channel();
        let observer: FileWriteObserver = std::sync::Arc::new(move |_, _, _| {
            let _ = written.send(());
        });
        let sink = CoalescingResultSink::with_window(
            paths.clone(),
            messages,
            Some(observer),
            Duration::from_millis(80),
        );
        let tx = sink.tx.clone();
        let sample = record();
        let feeder = thread::spawn(move || {
            for index in 0..50 {
                tx.send(Command::Records(vec![with_hash(
                    sample.clone(),
                    &index.to_string(),
                )]))
                .unwrap();
                thread::sleep(Duration::from_millis(10));
            }
        });
        writes.recv_timeout(Duration::from_millis(450)).unwrap();
        assert!(
            !feeder.is_finished(),
            "records must save while arrivals continue"
        );
        feeder.join().unwrap();
        sink.flush().unwrap();
        assert_eq!(count(&paths), 50);
    }

    #[test]
    fn explicit_flush_saves_every_queued_record_in_order() {
        let dir = tempfile::tempdir().unwrap();
        let paths = RuntimePaths::with_defaults(dir.path().to_owned());
        let (messages, _) = mpsc::channel();
        let sink = CoalescingResultSink::new(paths.clone(), messages, None);
        let original = record();
        let mut newer = original.clone();
        if let SavedResult::Summary(_, entry) = &mut newer {
            entry.result.summary = "replacement provenance".into();
        }
        sink.enqueue(vec![original]);
        sink.enqueue(vec![newer.clone()]);
        sink.flush().unwrap();
        let SavedResult::Summary(key, entry) = newer else {
            unreachable!()
        };
        assert_eq!(
            crate::load_summary_cache(&paths.summary_cache_path)
                .unwrap()
                .lookup(&key),
            Some(&entry)
        );
        assert_eq!(
            std::fs::read_to_string(&paths.summary_cache_path)
                .unwrap()
                .lines()
                .count(),
            2
        );
    }

    #[test]
    fn drop_waits_for_pending_records_to_reach_disk() {
        let dir = tempfile::tempdir().unwrap();
        let paths = RuntimePaths::with_defaults(dir.path().to_owned());
        let (messages, _) = mpsc::channel();
        let sink = CoalescingResultSink::new(paths.clone(), messages, None);
        sink.enqueue(vec![record()]);
        drop(sink);
        assert_eq!(count(&paths), 1);
    }

    #[test]
    fn failed_summary_does_not_repeat_triage_or_block_signal_across_windows() {
        let dir = tempfile::tempdir().unwrap();
        let paths = RuntimePaths::with_defaults(dir.path().to_owned());
        std::fs::create_dir(&paths.summary_cache_path).unwrap();
        let fixtures = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../harvester_batch/tests/fixtures/carry_over");
        let (triage_key, triage_entry) =
            crate::triage_cache_store::legacy(&fixtures.join(".triage_cache.ron"))
                .unwrap()
                .remove(0);
        let (signal_key, signal_entry) = crate::signal_candidate_cache_store::legacy(
            &fixtures.join(".signal_candidate_cache.ron"),
        )
        .unwrap()
        .remove(0);
        let (messages, received) = mpsc::channel();
        let sink = CoalescingResultSink::with_window(
            paths.clone(),
            messages,
            None,
            Duration::from_millis(40),
        );
        sink.enqueue(vec![
            SavedResult::Triage(triage_key, triage_entry),
            record(),
            SavedResult::SignalCandidate(signal_key, signal_entry),
        ]);
        thread::sleep(Duration::from_millis(120));
        let lines = || {
            std::fs::read_to_string(&paths.triage_cache_path)
                .unwrap()
                .lines()
                .count()
        };
        assert_eq!(lines(), 1);
        assert_eq!(
            std::fs::read_to_string(&paths.signal_candidate_cache_path)
                .unwrap()
                .lines()
                .count(),
            1
        );
        thread::sleep(Duration::from_millis(120));
        assert_eq!(lines(), 1, "successful kinds must leave the pending queue");
        assert!(sink.flush().is_err());
        assert_eq!(
            received.try_iter().count(),
            1,
            "report a failing store once"
        );
    }

    #[test]
    fn simulated_crash_keeps_every_result_older_than_the_window() {
        let dir = tempfile::tempdir().unwrap();
        let paths = RuntimePaths::with_defaults(dir.path().to_owned());
        let (messages, _) = mpsc::channel();
        let (written, writes) = mpsc::channel();
        let observer: FileWriteObserver = std::sync::Arc::new(move |_, _, _| {
            let _ = written.send(());
        });
        let mut sink = CoalescingResultSink::with_window(
            paths.clone(),
            messages,
            Some(observer),
            Duration::from_millis(30),
        );
        sink.enqueue(vec![with_hash(record(), "saved")]);
        writes.recv_timeout(Duration::from_secs(2)).unwrap();
        sink.enqueue(vec![with_hash(record(), "recent-unsaved")]);
        sink.tx.send(Command::Crash).unwrap();
        sink.worker
            .get_mut()
            .unwrap()
            .take()
            .unwrap()
            .join()
            .unwrap();
        assert_eq!(count(&paths), 1);
        assert_eq!(
            crate::load_summary_cache(&paths.summary_cache_path)
                .unwrap()
                .iter()
                .next()
                .unwrap()
                .0
                .content_hash,
            "saved"
        );
    }

    #[test]
    fn runner_uses_injected_sink_and_flushes_on_drop() {
        struct CapturingSink(std::sync::Arc<Mutex<Vec<&'static str>>>);
        impl ResultSink for CapturingSink {
            fn enqueue(&self, records: Vec<SavedResult>) {
                assert_eq!(records.len(), 1);
                self.0.lock().unwrap().push("record");
            }
            fn flush(&self) -> io::Result<()> {
                self.0.lock().unwrap().push("flush");
                Ok(())
            }
        }
        let dir = tempfile::tempdir().unwrap();
        let (messages, _) = mpsc::channel();
        let events = std::sync::Arc::new(Mutex::new(Vec::new()));
        let runner = crate::EffectRunner::new(
            RuntimePaths::with_defaults(dir.path().to_owned()),
            messages,
            Box::new(crate::NoOpPlatformHandler),
            Box::new(crate::NoOpRuntimePersistenceSink),
        )
        .with_result_sink(Box::new(CapturingSink(events.clone())));
        runner.enqueue(vec![
            harvester_core::Effect::SaveResults {
                records: vec![record()],
            },
            harvester_core::Effect::FlushResults,
        ]);
        drop(runner);
        assert_eq!(*events.lock().unwrap(), vec!["record", "flush", "flush"]);
    }
}
