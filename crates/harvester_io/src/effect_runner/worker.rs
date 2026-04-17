use std::path::PathBuf;
use std::sync::{mpsc, Arc, RwLock};
use std::time::Instant;

use engine_logging::{engine_debug, engine_error, engine_info, engine_warn};
use harvester_core::{LoadedArticle, Msg};
use harvester_engine::llm::PromptRegistry;
use harvester_engine::{load_and_prepare_articles_filtered_with_progress, ArticleScanProgress};

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
pub(super) fn run_triage_refresh_load(
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

    let guard = registry.read().unwrap();
    match load_and_prepare_articles_filtered_with_progress(
        &output_dir,
        max_input_bytes,
        &guard,
        &ordered_urls,
        since_utc,
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
        Ok((engine_articles, _)) => {
            let articles: Vec<LoadedArticle> = engine_articles
                .into_iter()
                .map(|article| LoadedArticle {
                    url: article.url,
                    source_title: article.source_title,
                    prepared_text: article.prepared_text,
                    content_hash: article.content_hash,
                    fetched_utc: article.fetched_utc,
                })
                .collect();
            let (files_scanned, files_total) = last_progress
                .map(|progress| (progress.files_scanned, progress.files_total))
                .unwrap_or((0, 0));
            engine_info!(
                "[pre-triage-refresh] load done request_id={} urls={} prepared={} files_scanned={} files_total={} elapsed_ms={}",
                request_id,
                ordered_urls.len(),
                articles.len(),
                files_scanned,
                files_total,
                load_started.elapsed().as_millis()
            );
            let _ = msg_tx.send(Msg::TriageArticlesLoaded {
                request_id,
                articles,
            });
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
