use std::collections::HashMap;
use std::path::Path;
use std::sync::{atomic::AtomicBool, mpsc, Arc, Mutex, RwLock};
use std::thread;
use std::time::Duration;

use chrono::Utc;
use engine_logging::{engine_error, engine_info, engine_warn};
use harvester_core::{Effect, JobResultKind, Msg, PersistenceSnapshot};
use harvester_engine::llm::prompt::PromptId;
use harvester_engine::llm::{LlmHandle, PromptRegistry};
use harvester_engine::{
    EngineConfig, EngineEvent, EngineHandle, FailureKind, FetchSettings, UrlPolicy,
};

mod configuration;
mod dispatch;
mod poll;
mod worker;

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

fn is_actionable_job_failure(kind: &FailureKind) -> bool {
    matches!(
        kind,
        FailureKind::InvalidUrl
            | FailureKind::ProcessingError
            | FailureKind::ProcessingTimeout { .. }
            | FailureKind::UrlPolicyViolation { .. }
            | FailureKind::QuotaExceeded { .. }
            | FailureKind::PathPolicyViolation { .. }
            | FailureKind::LlmError { .. }
            | FailureKind::LlmValidationFailed { .. }
    )
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

/// Sink for reducer-emitted runtime persistence snapshots.
pub trait RuntimePersistenceSink: Send + Sync {
    fn enqueue(&self, snapshot: PersistenceSnapshot);
    fn set_message_sender(&self, _sender: mpsc::Sender<Msg>) {}
    /// Sinks suppressing runtime snapshots still service independent link effects.
    fn store_article_links(
        &self,
        output: &Path,
        url: String,
        links: Vec<harvester_engine::ExtractedLink>,
        observer: Option<FileWriteObserver>,
    ) {
        crate::article_links::store_observed(output, &url, &links, &observer);
    }
}
pub type FileWriteObserver = Arc<dyn Fn(&Path, u64, Duration) + Send + Sync>;

impl RuntimePersistenceSink for crate::PersistenceWorker {
    fn store_article_links(
        &self,
        output: &Path,
        url: String,
        links: Vec<harvester_engine::ExtractedLink>,
        observer: Option<FileWriteObserver>,
    ) {
        crate::PersistenceWorker::enqueue_links(self, output.to_path_buf(), url, links, observer);
    }
    fn set_message_sender(&self, sender: mpsc::Sender<Msg>) {
        crate::PersistenceWorker::set_message_sender(self, sender);
    }
    fn enqueue(&self, snapshot: PersistenceSnapshot) {
        crate::PersistenceWorker::enqueue(self, snapshot);
    }
}

/// No-op sink for modes whose contract forbids runtime-state writes.
pub struct NoOpRuntimePersistenceSink;

impl RuntimePersistenceSink for NoOpRuntimePersistenceSink {
    fn enqueue(&self, _snapshot: PersistenceSnapshot) {}
}

/// Effect runner that orchestrates IO effects.
///
pub struct EffectRunner {
    engine: EngineHandle,
    corpus_scan_index: Arc<Mutex<harvester_engine::CorpusScanIndex>>,
    corpus_scan_reset_requested: Arc<AtomicBool>,
    msg_tx: mpsc::Sender<Msg>,
    paths: RuntimePaths,
    url_policy: UrlPolicy,
    fetch_settings: FetchSettings,
    llm_handle: Option<LlmHandle>,
    llm_max_input_bytes: Option<usize>,
    prompt_registry: Arc<RwLock<PromptRegistry>>,
    llm_metadata_models: HashMap<PromptId, String>,
    platform_handler: Box<dyn PlatformEffectHandler>,
    /// Host-selected sink for reducer-emitted runtime persistence snapshots.
    persistence_sink: Box<dyn RuntimePersistenceSink>,
    file_write_observer: Option<FileWriteObserver>,
    result_sink: Box<dyn crate::result_sink::ResultSink>,
}

impl EffectRunner {
    pub fn new(
        paths: RuntimePaths,
        msg_tx: mpsc::Sender<Msg>,
        platform_handler: Box<dyn PlatformEffectHandler>,
        persistence_sink: Box<dyn RuntimePersistenceSink>,
    ) -> Self {
        let registry = Arc::new(RwLock::new(PromptRegistry::with_defaults()));
        Self::with_optional_llm(
            paths,
            msg_tx,
            None,
            None,
            registry,
            HashMap::new(),
            platform_handler,
            persistence_sink,
            None,
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
        platform_handler: Box<dyn PlatformEffectHandler>,
        persistence_sink: Box<dyn RuntimePersistenceSink>,
    ) -> Self {
        Self::with_optional_llm(
            paths,
            msg_tx,
            Some(llm_handle),
            Some(llm_max_input_bytes),
            prompt_registry,
            llm_metadata_models,
            platform_handler,
            persistence_sink,
            None,
        )
    }

    #[allow(clippy::too_many_arguments)]
    pub fn new_with_llm_and_file_write_observer(
        paths: RuntimePaths,
        msg_tx: mpsc::Sender<Msg>,
        llm_handle: LlmHandle,
        llm_max_input_bytes: usize,
        prompt_registry: Arc<RwLock<PromptRegistry>>,
        llm_metadata_models: HashMap<PromptId, String>,
        platform_handler: Box<dyn PlatformEffectHandler>,
        persistence_sink: Box<dyn RuntimePersistenceSink>,
        file_write_observer: FileWriteObserver,
    ) -> Self {
        Self::with_optional_llm(
            paths,
            msg_tx,
            Some(llm_handle),
            Some(llm_max_input_bytes),
            prompt_registry,
            llm_metadata_models,
            platform_handler,
            persistence_sink,
            Some(file_write_observer),
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
        platform_handler: Box<dyn PlatformEffectHandler>,
        persistence_sink: Box<dyn RuntimePersistenceSink>,
        file_write_observer: Option<FileWriteObserver>,
    ) -> Self {
        let mut config = EngineConfig::default_with_output(paths.output_dir.clone());
        config.fetched_utc = Arc::new(|| Utc::now().to_rfc3339());
        let url_policy = config.url_policy.clone();
        let fetch_settings = config.fetch_settings.clone();

        let engine = EngineHandle::new(config);

        let result_sink = Box::new(crate::result_sink::CoalescingResultSink::new(
            paths.clone(),
            msg_tx.clone(),
            file_write_observer.clone(),
        ));
        persistence_sink.set_message_sender(msg_tx.clone());
        let runner = Self {
            corpus_scan_index: Arc::new(Mutex::new(harvester_engine::CorpusScanIndex::default())),
            corpus_scan_reset_requested: Arc::new(AtomicBool::new(false)),
            engine,
            msg_tx: msg_tx.clone(),
            paths,
            url_policy,
            fetch_settings,
            llm_handle,
            llm_max_input_bytes,
            prompt_registry,
            llm_metadata_models,
            platform_handler,
            result_sink,
            persistence_sink,
            file_write_observer,
        };
        runner.spawn_event_loop(msg_tx);
        runner
    }

    /// Test-only constructor that accepts a pre-built [`EngineConfig`] so tests
    /// can override URL policy (e.g. `block_private_ips: false`) to reach a
    /// local mock server without going through the full public constructors.
    #[cfg(test)]
    fn with_engine_config(
        paths: RuntimePaths,
        msg_tx: mpsc::Sender<Msg>,
        engine_config: EngineConfig,
        platform_handler: Box<dyn PlatformEffectHandler>,
        persistence_sink: Box<dyn RuntimePersistenceSink>,
    ) -> Self {
        let url_policy = engine_config.url_policy.clone();
        let fetch_settings = engine_config.fetch_settings.clone();
        let engine = EngineHandle::new(engine_config);

        let result_sink = Box::new(crate::result_sink::CoalescingResultSink::new(
            paths.clone(),
            msg_tx.clone(),
            None,
        ));
        persistence_sink.set_message_sender(msg_tx.clone());
        let runner = Self {
            corpus_scan_index: Arc::new(Mutex::new(harvester_engine::CorpusScanIndex::default())),
            corpus_scan_reset_requested: Arc::new(AtomicBool::new(false)),
            engine,
            msg_tx: msg_tx.clone(),
            paths,
            url_policy,
            fetch_settings,
            llm_handle: None,
            llm_max_input_bytes: None,
            prompt_registry: Arc::new(RwLock::new(PromptRegistry::with_defaults())),
            llm_metadata_models: HashMap::new(),
            platform_handler,
            result_sink,
            persistence_sink,
            file_write_observer: None,
        };
        runner.spawn_event_loop(msg_tx);
        runner
    }

    pub fn with_result_sink(mut self, sink: Box<dyn crate::result_sink::ResultSink>) -> Self {
        self.result_sink = sink;
        self
    }
    pub fn flush_results(&self) -> std::io::Result<()> {
        self.result_sink.flush()
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
                        });
                    }
                    EngineEvent::JobCompleted { job_id, result } => {
                        let class = harvester_engine::classify_fetch_outcome(&result);
                        let failure_label = result.as_ref().err().map(|k| k.to_string());
                        // Stamp the time here, at the side-effect→action boundary,
                        // so the reducer that records it stays pure/deterministic.
                        let recorded_at = Utc::now();
                        // Emit the blacklist classification first so the job's URL
                        // is still resolvable when the reducer handles it.
                        let _ = engine_tx.send(Msg::FetchOutcomeClassified {
                            job_id,
                            class,
                            failure_label,
                            recorded_at,
                        });
                        let msg = match result {
                            Ok(outcome) => Msg::JobDone {
                                job_id,
                                result: JobResultKind::Success,
                                extracted_links: outcome.extracted_links,
                                fetched_utc: outcome.fetched_utc,
                            },
                            Err(failure_kind) => {
                                let reason = failure_kind.to_string();
                                if is_actionable_job_failure(&failure_kind) {
                                    engine_warn!("Job {} failed: {}", job_id, reason);
                                } else {
                                    engine_info!("Job {} failed: {}", job_id, reason);
                                }
                                Msg::JobDone {
                                    job_id,
                                    result: JobResultKind::Failed { reason },
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
        // Send appropriate failure message based on effect type
        match effect {
            Effect::EnqueueUrl { job_id, .. } => {
                engine_error!(
                    "[effect] operation=enqueue job_id={} rejected={}",
                    job_id,
                    reason
                );
                let _ = self.msg_tx.send(Msg::JobDone {
                    job_id,
                    result: JobResultKind::Failed { reason },
                    extracted_links: Vec::new(),
                    fetched_utc: None,
                });
            }
            Effect::RequestLlmCompletion {
                request_id,
                prompt_id,
                ..
            } => {
                engine_error!(
                    "[effect] operation=llm request_id={} prompt_id={:?} rejected={}",
                    request_id,
                    prompt_id,
                    reason
                );
                let _ = self.msg_tx.send(Msg::LlmCompleted {
                    request_id,
                    result: harvester_core::LlmResultKind::Failed { reason },
                    metadata: None,
                });
            }
            _ => {
                engine_error!("[effect] rejected={}", reason);
            }
        }
    }
}

pub(super) fn observe_file_write(
    observer: &Option<FileWriteObserver>,
    path: &Path,
    elapsed: Duration,
) {
    if let Some(observer) = observer {
        let bytes = std::fs::metadata(path)
            .map(|metadata| metadata.len())
            .unwrap_or(0);
        observer(path, bytes, elapsed);
    }
}

impl Drop for EffectRunner {
    fn drop(&mut self) {
        if let Err(error) = self.flush_results() {
            engine_error!("[results] runner drop flush failed: {}", error);
        }
        engine_info!("[effect] EffectRunner dropped, stopping engine");
        self.engine.stop(true);
    }
}

#[cfg(test)]
mod tests;
