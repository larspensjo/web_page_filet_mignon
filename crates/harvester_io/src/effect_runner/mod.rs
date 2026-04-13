use std::collections::HashMap;
use std::sync::{mpsc, Arc, RwLock};
use std::thread;
use std::time::Duration;

use chrono::Utc;
use engine_logging::{engine_error, engine_info, engine_warn};
use harvester_core::{Effect, JobResultKind, Msg};
use harvester_engine::llm::prompt::PromptId;
use harvester_engine::llm::types::ProviderKind;
use harvester_engine::llm::{LlmHandle, PromptRegistry};
use harvester_engine::{
    is_confined_to, EngineConfig, EngineEvent, EngineHandle, FetchSettings, UrlPolicy,
};

mod dispatch;
mod poll;
mod worker;
use worker::{run_entity_index_worker, EntityIndexWorkerMsg};

use crate::effect_helpers::{map_llm_event, map_stage};
use crate::RuntimePaths;

const MAX_LOG_URL_LEN: usize = 96;

fn truncate_url_for_log(url: &str) -> String {
    if url.chars().count() <= MAX_LOG_URL_LEN {
        return url.to_string();
    }
    let mut short: String = url.chars().take(MAX_LOG_URL_LEN).collect();
    short.push_str("...");
    short
}

/// Trait for platform-specific effect handling (e.g., opening URLs in browser)
pub trait PlatformEffectHandler: Send + Sync {
    fn open_url(&self, url: &str);
}

/// No-op handler for batch/headless mode
pub struct NoOpPlatformHandler;

impl PlatformEffectHandler for NoOpPlatformHandler {
    fn open_url(&self, _url: &str) {
        engine_warn!("[effect] OpenUrlInBrowser ignored in headless mode");
    }
}

/// Effect runner that orchestrates IO effects.
///
/// # Entity index worker lifecycle
/// The runner spawns a dedicated single-threaded worker for entity index upserts.
/// Upserts are forwarded via `entity_index_worker_tx`. When `EffectRunner` is dropped,
/// the sender is dropped, which closes the channel and signals the worker to exit cleanly.
pub struct EffectRunner {
    engine: EngineHandle,
    msg_tx: mpsc::Sender<Msg>,
    paths: RuntimePaths,
    url_policy: UrlPolicy,
    fetch_settings: FetchSettings,
    llm_handle: Option<LlmHandle>,
    llm_max_input_bytes: Option<usize>,
    prompt_registry: Arc<RwLock<PromptRegistry>>,
    llm_metadata_models: HashMap<PromptId, String>,
    llm_provider: Option<Arc<dyn harvester_engine::llm::provider::LlmProvider>>,
    llm_default_provider: Option<ProviderKind>,
    platform_handler: Box<dyn PlatformEffectHandler>,
    /// Sender to the serialized entity-index worker. Dropping this closes the channel.
    entity_index_worker_tx: mpsc::SyncSender<EntityIndexWorkerMsg>,
}

impl EffectRunner {
    pub fn new(
        paths: RuntimePaths,
        msg_tx: mpsc::Sender<Msg>,
        platform_handler: Box<dyn PlatformEffectHandler>,
    ) -> Self {
        let registry = Arc::new(RwLock::new(PromptRegistry::with_defaults()));
        Self::with_optional_llm(
            paths,
            msg_tx,
            None,
            None,
            registry,
            HashMap::new(),
            None,
            None,
            platform_handler,
        )
    }

    #[allow(clippy::too_many_arguments)]
    pub fn new_with_llm(
        paths: RuntimePaths,
        msg_tx: mpsc::Sender<Msg>,
        llm_handle: LlmHandle,
        llm_max_input_bytes: usize,
        prompt_registry: Arc<RwLock<PromptRegistry>>,
        llm_metadata_models: HashMap<PromptId, String>,
        llm_provider: Arc<dyn harvester_engine::llm::provider::LlmProvider>,
        llm_default_provider: ProviderKind,
        platform_handler: Box<dyn PlatformEffectHandler>,
    ) -> Self {
        Self::with_optional_llm(
            paths,
            msg_tx,
            Some(llm_handle),
            Some(llm_max_input_bytes),
            prompt_registry,
            llm_metadata_models,
            Some(llm_provider),
            Some(llm_default_provider),
            platform_handler,
        )
    }

    #[allow(clippy::too_many_arguments)]
    fn with_optional_llm(
        paths: RuntimePaths,
        msg_tx: mpsc::Sender<Msg>,
        llm_handle: Option<LlmHandle>,
        llm_max_input_bytes: Option<usize>,
        prompt_registry: Arc<RwLock<PromptRegistry>>,
        llm_metadata_models: HashMap<PromptId, String>,
        llm_provider: Option<Arc<dyn harvester_engine::llm::provider::LlmProvider>>,
        llm_default_provider: Option<ProviderKind>,
        platform_handler: Box<dyn PlatformEffectHandler>,
    ) -> Self {
        let mut config = EngineConfig::default_with_output(paths.output_dir.clone());
        config.fetched_utc = Arc::new(|| Utc::now().to_rfc3339());
        let url_policy = config.url_policy.clone();
        let fetch_settings = config.fetch_settings.clone();

        let engine = EngineHandle::new(config);

        // Spawn the serialized entity-index worker.
        // All UpsertEntityIndexEntry effects are forwarded to this single-threaded worker,
        // which processes them sequentially (load â†’ merge â†’ atomic write).
        let entity_index_path = paths.entity_index_path.clone();
        let (worker_tx, worker_rx) = mpsc::sync_channel::<EntityIndexWorkerMsg>(256);
        thread::spawn(move || {
            run_entity_index_worker(worker_rx, entity_index_path);
        });

        let runner = Self {
            engine,
            msg_tx: msg_tx.clone(),
            paths,
            url_policy,
            fetch_settings,
            llm_handle,
            llm_max_input_bytes,
            prompt_registry,
            llm_metadata_models,
            llm_provider,
            llm_default_provider,
            platform_handler,
            entity_index_worker_tx: worker_tx,
        };
        runner.spawn_event_loop(msg_tx);
        runner
    }

    /// Block until all pending entity-index upserts have been written to disk.
    /// Only available in test builds for deterministic verification.
    #[cfg(test)]
    pub fn flush_entity_index_queue(&self) {
        let (done_tx, done_rx) = mpsc::sync_channel(0);
        let _ = self
            .entity_index_worker_tx
            .send(EntityIndexWorkerMsg::Flush { done: done_tx });
        let _ = done_rx.recv();
    }

    pub fn enqueue(&self, effects: Vec<Effect>) {
        for effect in effects {
            if let Err(reason) = self.validate_effect(&effect) {
                self.reject_effect(effect, reason);
                continue;
            }
            self.execute_effect(effect);
        }
    }

    fn spawn_event_loop(&self, msg_tx: mpsc::Sender<Msg>) {
        // Engine event loop
        let engine = self.engine.clone();
        let engine_tx = msg_tx.clone();
        thread::spawn(move || loop {
            if let Some(event) = engine.try_recv() {
                match event {
                    EngineEvent::Progress(progress) => {
                        let _ = engine_tx.send(Msg::JobProgress {
                            job_id: progress.job_id,
                            stage: map_stage(progress.stage),
                            tokens: progress.tokens,
                            bytes: progress.bytes,
                            content_preview: progress.content_preview.clone(),
                        });
                    }
                    EngineEvent::JobCompleted { job_id, result } => {
                        let msg = match result {
                            Ok(outcome) => Msg::JobDone {
                                job_id,
                                result: JobResultKind::Success,
                                content_preview: outcome.content_preview,
                                extracted_links: outcome.extracted_links,
                                fetched_utc: outcome.fetched_utc,
                            },
                            Err(failure_kind) => {
                                let reason = failure_kind.to_string();
                                engine_warn!("Job {} failed: {}", job_id, reason);
                                Msg::JobDone {
                                    job_id,
                                    result: JobResultKind::Failed { reason },
                                    content_preview: None,
                                    extracted_links: Vec::new(),
                                    fetched_utc: None,
                                }
                            }
                        };
                        let _ = engine_tx.send(msg);
                    }
                }
            } else {
                thread::sleep(Duration::from_millis(20));
            }
        });

        // LLM event loop
        if let Some(llm_handle) = &self.llm_handle {
            let llm_tx = msg_tx.clone();
            let receiver = llm_handle.event_receiver();
            thread::spawn(move || loop {
                let event = {
                    let guard = receiver.lock().expect("LLM event receiver lock");
                    guard.recv()
                };
                match event {
                    Ok(llm_event) => {
                        let msg = map_llm_event(llm_event);
                        if llm_tx.send(msg).is_err() {
                            break;
                        }
                    }
                    Err(_) => break,
                }
            });
        }
    }

    fn validate_effect(&self, effect: &Effect) -> Result<(), String> {
        match effect {
            Effect::EnqueueUrl { url, .. } => {
                let parsed =
                    url::Url::parse(url).map_err(|err| format!("invalid url {}: {}", url, err))?;
                self.url_policy
                    .check(&parsed)
                    .map_err(|violation| format!("url policy violation: {}", violation))?;
                Ok(())
            }
            Effect::DownloadLinkedPage { url, .. } => {
                let parsed = url::Url::parse(url)
                    .map_err(|err| format!("invalid linked page url {}: {}", url, err))?;
                self.url_policy.check(&parsed).map_err(|violation| {
                    format!("linked page url policy violation: {}", violation)
                })?;
                Ok(())
            }
            Effect::DeleteLinkedPage { path, .. } => {
                let linked_dir = self.paths.output_dir.join("linked");
                if !is_confined_to(path, &linked_dir) {
                    return Err(format!(
                        "delete linked page path violation: {:?} not in {:?}",
                        path, linked_dir
                    ));
                }
                Ok(())
            }
            Effect::RequestLlmCompletion { input_content, .. } => {
                if let Some(max) = self.llm_max_input_bytes {
                    if input_content.len() > max {
                        return Err(format!(
                            "LLM input too large: {} > {}",
                            input_content.len(),
                            max
                        ));
                    }
                }
                Ok(())
            }
            _ => Ok(()),
        }
    }

    fn reject_effect(&self, effect: Effect, reason: String) {
        engine_error!("[effect] Rejected: {:?} â€” {}", effect, reason);
        // Send appropriate failure message based on effect type
        match effect {
            Effect::EnqueueUrl { job_id, .. } => {
                let _ = self.msg_tx.send(Msg::JobDone {
                    job_id,
                    result: JobResultKind::Failed { reason },
                    content_preview: None,
                    extracted_links: Vec::new(),
                    fetched_utc: None,
                });
            }
            Effect::DownloadLinkedPage {
                job_id, link_index, ..
            } => {
                let _ = self.msg_tx.send(Msg::LinkDownloadFailed {
                    job_id,
                    link_index,
                    error: reason,
                });
            }
            Effect::DeleteLinkedPage {
                job_id, link_index, ..
            } => {
                // DeleteLinkedPage always sends LinkDeleted even on rejection (path confinement check happens here)
                let _ = self.msg_tx.send(Msg::LinkDeleted { job_id, link_index });
            }
            _ => {
                // For other effects, log the rejection without sending a message
            }
        }
    }
}

impl Drop for EffectRunner {
    fn drop(&mut self) {
        engine_info!("[effect] EffectRunner dropped, stopping engine");
        self.engine.stop(false);
        // `entity_index_worker_tx` is dropped here, closing the channel.
        // The worker thread sees RecvError and exits cleanly.
    }
}

#[cfg(test)]
mod tests;
