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
/// Processes `EntityIndexWorkerMsg::Upsert` messages one at a time, performing
/// a full load → merge → atomic-write cycle per message to ensure no concurrent writes.
///
/// Exits when the sender (`entity_index_worker_tx`) is dropped (channel closed).
pub(super) fn run_entity_index_worker(rx: mpsc::Receiver<EntityIndexWorkerMsg>, path: PathBuf) {
    engine_info!("[entity-index] worker started");
    for msg in rx {
        match msg {
            EntityIndexWorkerMsg::Upsert { url, patch } => {
                let mut index = crate::entity_index_store::load_entity_index(&path);
                crate::entity_index_store::upsert_entry(&mut index, &url, patch);
                if let Err(e) = crate::entity_index_store::save_entity_index(&path, &index) {
                    engine_error!(
                        "[entity-index] worker failed to save after upsert for '{}': {}",
                        url,
                        e
                    );
                } else {
                    engine_debug!("[entity-index] upserted entry for '{}'", url);
                }
            }
            #[cfg(test)]
            EntityIndexWorkerMsg::Flush { done } => {
                // All prior messages have been processed by the time we reach here.
                let _ = done.send(());
            }
        }
    }
    engine_info!("[entity-index] worker exited cleanly");
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
