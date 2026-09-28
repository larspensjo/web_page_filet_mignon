use std::path::PathBuf;
use std::sync::{
    atomic::{AtomicBool, Ordering},
    mpsc, Arc, RwLock,
};
use std::time::Instant;

use engine_logging::{engine_debug, engine_error, engine_info, engine_warn};
use harvester_core::Msg;
use harvester_engine::llm::PromptRegistry;
use harvester_engine::ArticleScanProgress;

use crate::effect_runner::FileWriteObserver;
use crate::entity_index_store::EntityIndexPatch;

/// Messages for the serialized entity-index worker.
pub(super) enum EntityIndexWorkerMsg {
    Upsert {
        url: String,
        patch: EntityIndexPatch,
    },
    /// Used in tests to flush the queue and confirm all prior upserts are persisted.
    #[cfg(test)]
    Flush { done: mpsc::SyncSender<()> },
}

/// Serialized entity-index worker.
///
/// Each cycle takes every upsert already queued and applies them with one
/// load → merge → atomic-write, so a burst costs one rewrite of the index file
/// rather than one per entry. Writes stay serialized on this thread.
///
/// Exits when the sender (`entity_index_worker_tx`) is dropped (channel closed)
/// and returns the number of index writes it performed.
#[cfg(test)]
pub(super) fn run_entity_index_worker(
    rx: mpsc::Receiver<EntityIndexWorkerMsg>,
    path: PathBuf,
) -> usize {
    run_entity_index_worker_with_observer(rx, path, None)
}

pub(super) fn run_entity_index_worker_with_observer(
    rx: mpsc::Receiver<EntityIndexWorkerMsg>,
    path: PathBuf,
    file_write_observer: Option<FileWriteObserver>,
) -> usize {
    engine_info!("[entity-index] worker started");
    let mut writes = 0;
    while let Ok(first) = rx.recv() {
        let mut batch = Vec::new();
        let mut next = Some(first);
        while let Some(msg) = next.take() {
            match msg {
                EntityIndexWorkerMsg::Upsert { url, patch } => batch.push((url, patch)),
                #[cfg(test)]
                EntityIndexWorkerMsg::Flush { done } => {
                    // Everything queued before the flush is written before it is acknowledged.
                    writes += write_entity_index_batch_observed(
                        &path,
                        std::mem::take(&mut batch),
                        file_write_observer.as_ref(),
                    );
                    let _ = done.send(());
                }
            }
            next = rx.try_recv().ok();
        }
        writes += write_entity_index_batch_observed(&path, batch, file_write_observer.as_ref());
    }
    engine_info!("[entity-index] worker exited cleanly writes={writes}");
    writes
}

fn write_entity_index_batch_observed(
    path: &std::path::Path,
    batch: Vec<(String, EntityIndexPatch)>,
    file_write_observer: Option<&FileWriteObserver>,
) -> usize {
    if batch.is_empty() {
        return 0;
    }
    let started = Instant::now();
    let entries = batch.len();
    let mut index = crate::entity_index_store::load_entity_index(path);
    for (url, patch) in batch {
        crate::entity_index_store::upsert_entry(&mut index, &url, patch);
    }
    match crate::entity_index_store::save_entity_index(path, &index) {
        Ok(_) => {
            let elapsed = started.elapsed();
            engine_debug!(
                "[entity-index] upserted entries={} elapsed_ms={}",
                entries,
                elapsed.as_millis()
            );
            if let Some(observer) = file_write_observer {
                let bytes = std::fs::metadata(path)
                    .map(|metadata| metadata.len())
                    .unwrap_or(0);
                observer(path, bytes, elapsed);
            }
        }
        Err(e) => engine_error!(
            "[entity-index] worker failed to save after upserting entries={}: {}",
            entries,
            e
        ),
    }
    1
}

/// Execute a single pre-triage article load in the calling thread.
///
/// Batching and quiet-period policy are handled by the reducer-owned
/// `PreTriageRefreshCoordinator`; this function is pure IO.
#[allow(clippy::too_many_arguments)]
pub(super) fn run_triage_refresh_load(
    index: Arc<std::sync::Mutex<harvester_engine::CorpusScanIndex>>,
    reset_requested: Arc<AtomicBool>,
    held: Vec<harvester_engine::HeldArticle>,
    request_id: u64,
    ordered_urls: Vec<String>,
    since_utc: Option<chrono::DateTime<chrono::Utc>>,
    msg_tx: mpsc::Sender<Msg>,
    output_dir: PathBuf,
    registry: Arc<RwLock<PromptRegistry>>,
    max_input_bytes: usize,
) {
    let load_started = Instant::now();
    let progress_tx = msg_tx.clone();
    let mut last_progress: Option<ArticleScanProgress> = None;
    engine_info!(
        "[pre-triage-refresh] load start request_id={} urls={}",
        request_id,
        ordered_urls.len(),
    );

    let guard = registry
        .read()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    let mut index = index
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    if reset_requested.swap(false, Ordering::AcqRel) {
        index.clear();
    }
    match index.load_delta(
        &output_dir,
        max_input_bytes,
        &guard,
        &ordered_urls,
        since_utc,
        &held,
        |progress| {
            last_progress = Some(progress);
            let should_emit = progress.files_scanned == 1
                || progress.files_scanned == progress.files_total
                || progress.files_scanned % 25 == 0;
            if !should_emit {
                return;
            }

            let _ = progress_tx.send(Msg::TriageArticlesLoadProgress {
                request_id,
                files_scanned: progress.files_scanned,
                files_total: progress.files_total,
            });
        },
    ) {
        Ok((delta, stats)) => {
            engine_info!("[corpus-index] request_id={} files={} reused={} read={} reprepared={} removed={} budget={} elapsed_ms={}",
                request_id, stats.files, stats.reused, stats.read, stats.reprepared, stats.removed,
                delta.preparation_budget, load_started.elapsed().as_millis());
            let _ = msg_tx.send(Msg::TriageArticlesLoaded { request_id, delta });
        }
        Err(reason) => {
            let (files_scanned, files_total) = last_progress
                .map(|progress| (progress.files_scanned, progress.files_total))
                .unwrap_or((0, 0));
            engine_warn!(
                "[pre-triage-refresh] load failed request_id={} urls={} files_scanned={} files_total={} elapsed_ms={} reason={}",
                request_id,
                ordered_urls.len(),
                files_scanned,
                files_total,
                load_started.elapsed().as_millis(),
                reason
            );
            let _ = msg_tx.send(Msg::TriageArticlesLoadFailed { request_id, reason });
        }
    }
}
