use std::collections::HashMap;
use std::fs;
use std::path::PathBuf;
use std::sync::{mpsc, Arc, RwLock};
use std::thread;
use std::time::{Duration, Instant};

use chrono::Utc;
use engine_logging::{engine_debug, engine_error, engine_info, engine_warn};
use harvester_core::{Effect, JobResultKind, LlmResultKind, LoadedArticle, Msg, StopPolicy};
use harvester_engine::llm::load_context_file;
use harvester_engine::llm::prompt::{PromptId, PromptTemplateOwned, PROMPT_VERSION_DRAFT};
use harvester_engine::llm::prompt_context::{ContextMeta, PromptContextFile};
use harvester_engine::llm::types::ProviderKind;
use harvester_engine::llm::{LlmCommand, LlmHandle, PromptRegistry};
use harvester_engine::{
    build_triage_archive, import_saved_webpages, is_confined_to,
    load_and_prepare_articles_filtered, poll_curated_source, poll_file_source,
    scan_archive_article_metadata, EngineConfig, EngineEvent, EngineHandle, ExportOptions,
    FetchSettings, ImportOptions, SourceType, UrlPolicy,
};

use crate::effect_helpers::{
    build_local_model_catalog, download_link_page, handle_rss_source_poll, map_llm_event,
    map_stage, prompt_context_filename, PollGuard, RssPollContext,
};
use crate::{load_seen_set, load_sources, RuntimePaths};

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

/// Messages for the serialized entity-index worker.
enum EntityIndexWorkerMsg {
    Upsert {
        url: String,
        patch: crate::entity_index_store::EntityIndexPatch,
    },
    /// Used in tests to flush the queue and confirm all prior upserts are persisted.
    #[cfg(test)]
    Flush { done: mpsc::SyncSender<()> },
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
        // which processes them sequentially (load → merge → atomic write).
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

    fn execute_effect(&self, effect: Effect) {
        match effect {
            Effect::EnqueueUrl { job_id, url } => {
                engine_info!(
                    "EnqueueUrl job_id={} url_len={} url={}",
                    job_id,
                    url.len(),
                    truncate_url_for_log(&url)
                );
                self.engine.enqueue(job_id, url);
            }
            Effect::StartSession => {
                // no-op; engine starts on first enqueue
            }
            Effect::StopFinish { policy } => {
                let immediate = matches!(policy, StopPolicy::Immediate);
                self.engine.stop(immediate);
            }
            Effect::ArchiveRequested {
                ordered_urls,
                since_utc,
            } => {
                let output_dir = self.paths.output_dir.clone();
                thread::spawn(move || {
                    let options = ExportOptions {
                        output_filename: "archive.md".to_string(),
                        manifest_filename: None,
                        ..ExportOptions::default()
                    };
                    match build_triage_archive(&output_dir, &ordered_urls, since_utc, options) {
                        Ok(summary) => engine_info!(
                            "[archive] archive written docs={} path={}",
                            summary.doc_count,
                            summary.output_path.display()
                        ),
                        Err(err) => engine_warn!("[archive] archive failed: {}", err),
                    }
                });
            }
            Effect::OpenUrlInBrowser { url } => {
                self.platform_handler.open_url(&url);
            }
            Effect::ResolvePromptLabInputFromUrl { resolve_id, url } => {
                let msg_tx = self.msg_tx.clone();
                let output_dir = self.paths.output_dir.clone();
                let registry = self.prompt_registry.clone();
                let max_input_bytes = self.llm_max_input_bytes.unwrap_or(100_000);
                thread::spawn(move || {
                    engine_info!(
                        "[prompt-lab] resolve requested resolve_id={} url={}",
                        resolve_id,
                        url
                    );
                    let guard = registry.read().unwrap();
                    match load_and_prepare_articles_filtered(
                        &output_dir,
                        max_input_bytes,
                        &guard,
                        std::slice::from_ref(&url),
                        None,
                    ) {
                        Ok((mut articles, _collection_text)) => {
                            if let Some(article) = articles.pop() {
                                let _ = msg_tx.send(Msg::PromptLabInputResolved {
                                    resolve_id,
                                    result: Ok(article.prepared_text),
                                });
                            } else {
                                let reason = "article missing after resolution".to_string();
                                engine_warn!("[prompt-lab] resolve failed: {}", reason);
                                let _ = msg_tx.send(Msg::PromptLabInputResolved {
                                    resolve_id,
                                    result: Err(reason),
                                });
                            }
                        }
                        Err(reason) => {
                            engine_warn!("[prompt-lab] resolve failed: {}", reason);
                            let _ = msg_tx.send(Msg::PromptLabInputResolved {
                                resolve_id,
                                result: Err(reason),
                            });
                        }
                    }
                });
            }
            Effect::LoadPromptLabModelCatalog => {
                let msg_tx = self.msg_tx.clone();
                let provider = self.llm_provider.clone();
                let default_provider_kind = self.llm_default_provider;
                let effective_models = self.llm_metadata_models.clone();

                thread::spawn(move || {
                    engine_info!("[prompt-lab-model] loading model catalog");

                    let local_fallback_models =
                        build_local_model_catalog(default_provider_kind, &effective_models);

                    let (models, source) = if let (Some(provider), Some(provider_kind)) =
                        (provider, default_provider_kind)
                    {
                        match tokio::runtime::Runtime::new() {
                            Ok(runtime) => match runtime.block_on(provider.list_models()) {
                                Ok(mut model_names) => {
                                    model_names.sort();
                                    model_names.dedup();

                                    engine_info!(
                                        "[prompt-lab-model] remote discovery succeeded: {} models found: {}",
                                        model_names.len(),
                                        model_names.join(", ")
                                    );

                                    let models: Vec<_> = model_names
                                        .into_iter()
                                        .map(|name| {
                                            harvester_engine::llm::types::ModelId::new(
                                                provider_kind,
                                                name,
                                            )
                                        })
                                        .collect();
                                    (models, harvester_core::ModelCatalogSource::Remote)
                                }
                                Err(err) => {
                                    engine_warn!(
                                        "[prompt-lab-model] remote discovery failed: {}",
                                        err
                                    );
                                    (
                                        local_fallback_models,
                                        harvester_core::ModelCatalogSource::LocalFallback,
                                    )
                                }
                            },
                            Err(err) => {
                                engine_warn!(
                                    "[prompt-lab-model] tokio runtime creation failed: {}",
                                    err
                                );
                                (
                                    local_fallback_models,
                                    harvester_core::ModelCatalogSource::LocalFallback,
                                )
                            }
                        }
                    } else {
                        (
                            local_fallback_models,
                            harvester_core::ModelCatalogSource::LocalFallback,
                        )
                    };

                    let _ = msg_tx.send(Msg::PromptLabModelCatalogLoaded { models, source });
                });
            }
            Effect::DownloadLinkedPage {
                job_id,
                link_index,
                url,
            } => {
                engine_info!(
                    "DownloadLinkedPage job_id={} link_index={} url={}",
                    job_id,
                    link_index,
                    url
                );
                let msg_tx = self.msg_tx.clone();
                let output_dir = self.paths.output_dir.clone();
                let url_policy = self.url_policy.clone();
                let fetch_settings = self.fetch_settings.clone();
                thread::spawn(move || {
                    match download_link_page(&url, &output_dir, &url_policy, &fetch_settings) {
                        Ok(path) => {
                            engine_info!("Linked page saved: {}", path.display());
                            let _ = msg_tx.send(Msg::LinkDownloadCompleted {
                                job_id,
                                link_index,
                                path,
                            });
                        }
                        Err(error) => {
                            engine_warn!("Linked page download failed: {}", error);
                            let _ = msg_tx.send(Msg::LinkDownloadFailed {
                                job_id,
                                link_index,
                                error,
                            });
                        }
                    }
                });
            }
            Effect::DeleteLinkedPage {
                job_id,
                link_index,
                path,
            } => {
                engine_info!(
                    "Delete linked page job_id={} link_index={} path={}",
                    job_id,
                    link_index,
                    path.display()
                );
                let msg_tx = self.msg_tx.clone();
                let output_dir = self.paths.output_dir.clone();
                thread::spawn(move || {
                    if is_confined_to(&path, &output_dir) {
                        let absolute_path = output_dir.join(&path);
                        let _ = fs::remove_file(&absolute_path);
                    } else {
                        engine_warn!(
                            "DeleteLinkedPage rejected unsafe path job_id={} link_index={} path={}",
                            job_id,
                            link_index,
                            path.display()
                        );
                    }
                    let _ = msg_tx.send(Msg::LinkDeleted { job_id, link_index });
                });
            }
            Effect::SavePromptContextFile {
                prompt_id,
                mut context_pairs,
            } => {
                let msg_tx = self.msg_tx.clone();
                let contexts_dir = self.paths.contexts_dir.clone();
                thread::spawn(move || {
                    if let Err(err) = fs::create_dir_all(&contexts_dir) {
                        let reason = format!("failed to create contexts directory: {}", err);
                        engine_error!(
                            "[prompt-lab-context] SavePromptContextFile {} prompt_id={:?}",
                            reason,
                            prompt_id
                        );
                        let _ = msg_tx.send(Msg::PromptLabContextSaveFailed { prompt_id, reason });
                        return;
                    }

                    let filename = prompt_context_filename(prompt_id);
                    let path = contexts_dir.join(filename);

                    let existing_meta = if path.exists() {
                        match load_context_file(&path) {
                            Ok(file) => file.meta,
                            Err(err) => {
                                let reason = format!("failed to read existing context: {}", err);
                                engine_error!(
                                    "[prompt-lab-context] SavePromptContextFile {} prompt_id={:?}",
                                    reason,
                                    prompt_id
                                );
                                let _ = msg_tx
                                    .send(Msg::PromptLabContextSaveFailed { prompt_id, reason });
                                return;
                            }
                        }
                    } else {
                        ContextMeta {
                            prompt_id: prompt_id.to_string(),
                            schema_version: 1,
                            version: 0,
                            updated: Utc::now().to_rfc3339(),
                            description: None,
                            changelog: None,
                        }
                    };

                    let mut meta = existing_meta;
                    meta.schema_version = 1;
                    meta.prompt_id = prompt_id.to_string();
                    meta.version = meta.version.saturating_add(1);
                    meta.updated = Utc::now().to_rfc3339();

                    context_pairs.sort_by(|a, b| a.0.cmp(&b.0));
                    let variables = context_pairs.into_iter().collect::<HashMap<_, _>>();

                    let ctx_file = PromptContextFile {
                        meta: meta.clone(),
                        variables,
                    };

                    let mut toml_string = match toml::to_string(&ctx_file) {
                        Ok(serialized) => serialized,
                        Err(err) => {
                            let reason = format!("failed to serialize context: {}", err);
                            engine_error!(
                                "[prompt-lab-context] SavePromptContextFile {} prompt_id={:?}",
                                reason,
                                prompt_id
                            );
                            let _ =
                                msg_tx.send(Msg::PromptLabContextSaveFailed { prompt_id, reason });
                            return;
                        }
                    };
                    toml_string.push('\n');

                    let tmp_path = path.with_extension("toml.tmp");
                    if let Err(err) = fs::write(&tmp_path, toml_string) {
                        let reason = format!("failed to write temp file: {}", err);
                        engine_error!(
                            "[prompt-lab-context] SavePromptContextFile {} prompt_id={:?}",
                            reason,
                            prompt_id
                        );
                        let _ = msg_tx.send(Msg::PromptLabContextSaveFailed { prompt_id, reason });
                        return;
                    }

                    if let Err(err) = fs::rename(&tmp_path, &path) {
                        let reason = format!("failed to rename temp file: {}", err);
                        engine_error!(
                            "[prompt-lab-context] SavePromptContextFile {} prompt_id={:?}",
                            reason,
                            prompt_id
                        );
                        let _ = msg_tx.send(Msg::PromptLabContextSaveFailed { prompt_id, reason });
                        return;
                    }

                    engine_info!(
                        "[prompt-lab-context] Saved context for {:?} to {:?}",
                        prompt_id,
                        path
                    );
                    let _ = msg_tx.send(Msg::PromptLabContextSaved {
                        prompt_id,
                        path: path.display().to_string(),
                        version: meta.version as u64,
                    });
                });
            }
            Effect::SavePromptTemplateFile {
                prompt_id,
                system_template,
                user_template,
                description,
                expected_format,
            } => {
                let prompts_dir = self.paths.prompts_dir.clone();
                let registry = self.prompt_registry.clone();
                let msg_tx = self.msg_tx.clone();
                thread::spawn(move || {
                    match crate::save_prompt_template(
                        &prompts_dir,
                        prompt_id,
                        &system_template,
                        &user_template,
                        &description,
                        &expected_format,
                    ) {
                        Ok((version, path)) => {
                            let overlay = PromptTemplateOwned {
                                id: prompt_id,
                                version,
                                system_template: system_template.clone(),
                                user_template: user_template.clone(),
                                description: description.clone(),
                                expected_format: expected_format.clone(),
                            };
                            if let Ok(mut reg) = registry.write() {
                                reg.register_overlay(overlay);
                            }
                            engine_info!("[prompt-lab-template] Saved template prompt_id={:?} path={} version={}", prompt_id, path.display(), version);
                            let _ = msg_tx.send(Msg::PromptLabTemplateSaved {
                                prompt_id,
                                version,
                                path: path.display().to_string(),
                            });
                        }
                        Err(reason) => {
                            engine_error!("[prompt-lab-template] SavePromptTemplateFile failed prompt_id={:?} reason={}", prompt_id, reason);
                            let _ =
                                msg_tx.send(Msg::PromptLabTemplateSaveFailed { prompt_id, reason });
                        }
                    }
                });
            }
            Effect::RequestLlmCompletion {
                request_id,
                prompt_id,
                prompt_version,
                model_override,
                input_content,
                context,
                template_override,
                extra_template_vars,
            } => {
                if let Some(handle) = &self.llm_handle {
                    let cmd = LlmCommand::Complete(Box::new(
                        harvester_engine::llm::LlmCompletionCommand {
                            request_id,
                            prompt_id,
                            prompt_version,
                            model_override,
                            input_content,
                            context,
                            template_override,
                            extra_template_vars,
                        },
                    ));
                    if let Err(err) = handle.send(cmd) {
                        engine_warn!(
                            "LLM completion request failed to dispatch: request_id={} error={:?}",
                            request_id,
                            err
                        );
                        let _ = self.msg_tx.send(Msg::LlmCompleted {
                            request_id,
                            result: LlmResultKind::Failed {
                                reason: "LLM worker unavailable".to_string(),
                            },
                            metadata: None,
                        });
                    } else {
                        engine_info!(
                            "[llm-dispatch] request_id={} prompt_id={:?}",
                            request_id,
                            prompt_id
                        );
                    }
                } else {
                    engine_warn!(
                        "LLM completion requested without handle: request_id={}",
                        request_id
                    );
                    let _ = self.msg_tx.send(Msg::LlmCompleted {
                        request_id,
                        result: LlmResultKind::Failed {
                            reason: "LLM not configured".to_string(),
                        },
                        metadata: None,
                    });
                }
            }
            Effect::LoadArticlesForBriefing {
                ordered_urls,
                since_utc,
            } => {
                let msg_tx = self.msg_tx.clone();
                let output_dir = self.paths.output_dir.clone();
                let max_input_bytes = self.llm_max_input_bytes.unwrap_or(100_000);
                let registry = self.prompt_registry.clone();
                thread::spawn(move || {
                    let load_started = Instant::now();
                    engine_info!(
                        "[articles-load] briefing start urls={} since_filter={}",
                        ordered_urls.len(),
                        since_utc.is_some()
                    );
                    let guard = registry.read().unwrap();
                    match load_and_prepare_articles_filtered(
                        &output_dir,
                        max_input_bytes,
                        &guard,
                        &ordered_urls,
                        since_utc,
                    ) {
                        Ok((articles, collection_text)) => {
                            let loaded_articles: Vec<LoadedArticle> = articles
                                .into_iter()
                                .map(|article| LoadedArticle {
                                    url: article.url,
                                    source_title: article.source_title,
                                    prepared_text: article.prepared_text,
                                    content_hash: article.content_hash,
                                    fetched_utc: article.fetched_utc,
                                })
                                .collect();
                            engine_info!(
                                "[briefing-loader] prepared {} article(s)",
                                loaded_articles.len()
                            );
                            engine_info!(
                                "[articles-load] briefing done urls={} prepared={} elapsed_ms={}",
                                ordered_urls.len(),
                                loaded_articles.len(),
                                load_started.elapsed().as_millis()
                            );
                            let _ = msg_tx.send(Msg::ArticlesLoaded {
                                articles: loaded_articles,
                                collection_text,
                            });
                        }
                        Err(reason) => {
                            engine_warn!(
                                "[articles-load] briefing failed urls={} elapsed_ms={} reason={}",
                                ordered_urls.len(),
                                load_started.elapsed().as_millis(),
                                reason
                            );
                            engine_warn!("[briefing-loader] load failed: {}", reason);
                            let _ = msg_tx.send(Msg::ArticlesLoadFailed { reason });
                        }
                    }
                });
            }
            Effect::LoadArticlesForBriefingPrereq {
                ordered_urls,
                since_utc,
            } => {
                let msg_tx = self.msg_tx.clone();
                let output_dir = self.paths.output_dir.clone();
                let max_input_bytes = self.llm_max_input_bytes.unwrap_or(100_000);
                let registry = self.prompt_registry.clone();
                thread::spawn(move || {
                    let load_started = Instant::now();
                    engine_info!(
                        "[articles-load] briefing-prereq start urls={} since_filter={}",
                        ordered_urls.len(),
                        since_utc.is_some()
                    );
                    let guard = registry.read().unwrap();
                    match load_and_prepare_articles_filtered(
                        &output_dir,
                        max_input_bytes,
                        &guard,
                        &ordered_urls,
                        since_utc,
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
                            engine_info!(
                                "[articles-load] briefing-prereq done urls={} prepared={} elapsed_ms={}",
                                ordered_urls.len(),
                                articles.len(),
                                load_started.elapsed().as_millis()
                            );
                            let _ = msg_tx.send(Msg::BriefingPrereqArticlesLoaded { articles });
                        }
                        Err(reason) => {
                            engine_warn!(
                                "[articles-load] briefing-prereq failed urls={} elapsed_ms={} reason={}",
                                ordered_urls.len(),
                                load_started.elapsed().as_millis(),
                                reason
                            );
                            let _ = msg_tx.send(Msg::BriefingPrereqLoadFailed { reason });
                        }
                    }
                });
            }
            Effect::LoadArticlesForTriage {
                request_id,
                ordered_urls,
                since_utc,
            } => {
                let msg_tx = self.msg_tx.clone();
                let output_dir = self.paths.output_dir.clone();
                let registry = Arc::clone(&self.prompt_registry);
                let max_input_bytes = self.llm_max_input_bytes.unwrap_or(100_000);
                thread::spawn(move || {
                    run_triage_refresh_load(
                        request_id,
                        ordered_urls,
                        since_utc,
                        msg_tx,
                        output_dir,
                        registry,
                        max_input_bytes,
                    );
                });
            }
            Effect::LoadPromptContexts => {
                let msg_tx = self.msg_tx.clone();
                let contexts_dir = self.paths.contexts_dir.clone();
                thread::spawn(move || {
                    if !contexts_dir.exists() {
                        engine_warn!(
                            "[PromptContext] contexts directory not found at {:?}",
                            contexts_dir
                        );
                        let _ = msg_tx.send(Msg::PromptContextsLoaded {
                            contexts: HashMap::new(),
                        });
                        return;
                    }

                    let mut contexts = HashMap::new();
                    let prompt_ids = [
                        PromptId::ArticleTriage,
                        PromptId::ArticleSummary,
                        PromptId::AggregateBriefing,
                    ];

                    for prompt_id in prompt_ids {
                        let filename = prompt_context_filename(prompt_id);
                        let path = contexts_dir.join(filename);

                        if !path.exists() {
                            continue;
                        }

                        match load_context_file(&path) {
                            Ok(ctx_file) => {
                                let vec: Vec<(String, String)> =
                                    ctx_file.variables.into_iter().collect();
                                contexts.insert(prompt_id, vec);
                            }
                            Err(e) => {
                                engine_warn!("[PromptContext] Failed to load {:?}: {}", path, e);
                            }
                        }
                    }

                    let _ = msg_tx.send(Msg::PromptContextsLoaded { contexts });
                });
            }
            Effect::LoadPromptTemplateFiles => {
                let prompts_dir = self.paths.prompts_dir.clone();
                let registry = self.prompt_registry.clone();
                thread::spawn(move || {
                    for entry in crate::load_prompt_templates(&prompts_dir) {
                        let loaded_template = match entry {
                            Ok(lt) => lt,
                            Err(reason) => {
                                engine_warn!(
                                    "[prompt-lab-template] Failed to load saved template: {}",
                                    reason
                                );
                                continue;
                            }
                        };

                        if loaded_template.template_file.version == PROMPT_VERSION_DRAFT {
                            engine_warn!("[prompt-lab-template] skipping draft saved template prompt_id={:?} path={}", loaded_template.prompt_id, loaded_template.path.display());
                            continue;
                        }

                        let overlay = PromptTemplateOwned {
                            id: loaded_template.prompt_id,
                            version: loaded_template.template_file.version,
                            system_template: loaded_template.template_file.system_template,
                            user_template: loaded_template.template_file.user_template,
                            description: loaded_template.template_file.description,
                            expected_format: loaded_template.template_file.expected_format,
                        };

                        if let Ok(mut guard) = registry.write() {
                            guard.register_overlay(overlay);
                        }
                        engine_info!("[prompt-lab-template] Loaded saved template prompt_id={:?} version={} path={}", loaded_template.prompt_id, loaded_template.template_file.version, loaded_template.path.display());
                    }
                });
            }
            Effect::LoadLlmMetadata => {
                let msg_tx = self.msg_tx.clone();
                let registry = self.prompt_registry.clone();
                let models = self.llm_metadata_models.clone();
                thread::spawn(move || {
                    use harvester_core::PromptLabTemplateSnapshot;

                    let (active_versions, templates) = {
                        let guard = registry.read().unwrap();
                        let versions = guard.active_versions_map();
                        let prompt_ids = &[
                            PromptId::ArticleTriage,
                            PromptId::ArticleSummary,
                            PromptId::AggregateBriefing,
                        ];
                        let templates = prompt_ids
                            .iter()
                            .filter_map(|&prompt_id| {
                                guard.active_effective(prompt_id).map(|effective| {
                                    (
                                        prompt_id,
                                        PromptLabTemplateSnapshot {
                                            template: effective.to_owned(),
                                            source: effective.source(),
                                        },
                                    )
                                })
                            })
                            .collect::<HashMap<_, _>>();
                        (versions, templates)
                    };
                    let effective_models = models;

                    engine_info!(
                        "[llm-metadata] metadata prepared (versions={}, models={} templates={})",
                        active_versions.len(),
                        effective_models.len(),
                        templates.len(),
                    );

                    let _ = msg_tx.send(Msg::LlmMetadataLoaded {
                        active_versions,
                        effective_models,
                        templates,
                    });
                });
            }
            Effect::PersistSummaryCache { cache } => {
                let msg_tx = self.msg_tx.clone();
                let path = self.paths.summary_cache_path.clone();
                thread::spawn(move || {
                    match crate::persist_summary_cache(&cache, &path) {
                        Ok(_) => {
                            engine_info!("[summary-cache] Persisted cache to {:?}", path);
                        }
                        Err(err) => {
                            engine_warn!(
                                "[summary-cache] Failed to persist cache to {:?}: {}",
                                path,
                                err
                            );
                        }
                    }
                    // Fire-and-forget, no message sent
                    let _ = msg_tx;
                });
            }
            Effect::PersistTriageCache { cache } => {
                let msg_tx = self.msg_tx.clone();
                let path = self.paths.triage_cache_path.clone();
                thread::spawn(move || {
                    match crate::persist_triage_cache(&cache, &path) {
                        Ok(_) => {
                            engine_info!("[triage-cache] Persisted cache to {:?}", path);
                        }
                        Err(err) => {
                            engine_warn!(
                                "[triage-cache] Failed to persist cache to {:?}: {}",
                                path,
                                err
                            );
                        }
                    }
                    // Fire-and-forget, no message sent
                    let _ = msg_tx;
                });
            }
            Effect::PollAllSources => {
                self.execute_poll_all_sources();
            }
            Effect::LoadBriefingHistory => {
                let msg_tx = self.msg_tx.clone();
                let path = self.paths.briefing_history_path.clone();
                thread::spawn(move || {
                    // load_briefing_history already returns [] and logs on failure —
                    // always send BriefingHistoryLoaded (no separate failure Msg).
                    let entries = crate::load_briefing_history(&path);
                    let _ = msg_tx.send(Msg::BriefingHistoryLoaded { entries });
                });
            }
            Effect::SaveBriefingHistory { entries } => {
                let path = self.paths.briefing_history_path.clone();
                thread::spawn(move || {
                    if let Err(e) = crate::save_briefing_history(&path, &entries) {
                        engine_error!("[briefing-history] Save failed: {}", e);
                        // Non-fatal: no Msg sent on failure
                    }
                });
            }
            Effect::LoadBriefingCheckpoint => {
                let msg_tx = self.msg_tx.clone();
                let path = self.paths.briefing_checkpoint_path.clone();
                thread::spawn(move || {
                    let since_utc = crate::load_briefing_checkpoint(&path);
                    let _ = msg_tx.send(Msg::BriefingCheckpointLoaded { since_utc });
                });
            }
            Effect::SaveBriefingCheckpoint { since_utc } => {
                let path = self.paths.briefing_checkpoint_path.clone();
                thread::spawn(move || {
                    let s = since_utc.map(|dt| dt.to_rfc3339());
                    if let Err(e) = crate::save_briefing_checkpoint(&path, s.as_deref()) {
                        engine_error!("[briefing-checkpoint] save failed: {}", e);
                    }
                });
            }
            Effect::LoadEntityIndex => {
                let path = self.paths.entity_index_path.clone();
                let msg_tx = self.msg_tx.clone();
                thread::spawn(move || {
                    let index = crate::entity_index_store::load_entity_index(&path);
                    // `load_entity_index` already logs and returns default on parse/IO errors.
                    // Distinguish parse failures from successful-but-empty by checking if the
                    // file exists. If it does not exist, treat as a fresh (empty) index — loaded,
                    // not failed.
                    engine_info!(
                        "[entity-index] LoadEntityIndex: {} entries",
                        index.entries.len()
                    );
                    let _ = msg_tx.send(Msg::EntityIndexLoaded { index });
                });
            }
            Effect::RebuildEntityIndex => {
                let msg_tx = self.msg_tx.clone();
                let output_dir = self.paths.output_dir.clone();
                let triage_cache_path = self.paths.triage_cache_path.clone();
                let summary_cache_path = self.paths.summary_cache_path.clone();
                let entity_index_path = self.paths.entity_index_path.clone();
                thread::spawn(move || {
                    engine_info!("[entity-index] starting rebuild");

                    // Step 1: scan archive for article metadata (url, fetched_utc, content_hash)
                    let article_metas = match scan_archive_article_metadata(&output_dir) {
                        Ok(metas) => metas,
                        Err(e) => {
                            engine_error!("[entity-index] rebuild: scan failed: {}", e);
                            let _ = msg_tx.send(Msg::EntityIndexRebuildFailed {
                                reason: format!("scan failed: {e}"),
                            });
                            return;
                        }
                    };

                    // Step 2: load triage cache and build content_hash → tags map.
                    // Use the first entry per content_hash (any version is fine for rebuild).
                    let triage_cache =
                        crate::triage_cache_store::load_triage_cache(&triage_cache_path);
                    let mut themes_map: std::collections::HashMap<String, Vec<String>> =
                        std::collections::HashMap::new();
                    for (key, entry) in triage_cache.iter() {
                        themes_map
                            .entry(key.content_hash.clone())
                            .or_insert_with(|| entry.result.tags.clone());
                    }

                    // Step 3: load summary cache and build content_hash → entities map.
                    // Only V4+ entries will have non-empty entities; older entries → empty.
                    let summary_cache =
                        crate::summary_cache_store::load_summary_cache(&summary_cache_path);
                    let mut entities_map: std::collections::HashMap<
                        String,
                        harvester_engine::llm::SummaryEntities,
                    > = std::collections::HashMap::new();
                    for (key, entry) in summary_cache.iter() {
                        entities_map
                            .entry(key.content_hash.clone())
                            .or_insert_with(|| entry.result.entities.clone());
                    }

                    // Step 4: build EntityIndex by joining on content_hash.
                    let mut index =
                        crate::entity_index_store::load_entity_index(&entity_index_path);
                    for meta in &article_metas {
                        let content_hash = meta.content_hash.as_deref().unwrap_or("");
                        let entities = entities_map.get(content_hash);
                        let themes = themes_map.get(content_hash);
                        let entry = harvester_core::entity_index::EntityIndexEntry {
                            fetched_utc: meta.fetched_utc.clone(),
                            content_hash: meta.content_hash.clone(),
                            companies: entities.map(|e| e.companies.clone()).unwrap_or_default(),
                            technologies: entities
                                .map(|e| e.technologies.clone())
                                .unwrap_or_default(),
                            products: entities.map(|e| e.products.clone()).unwrap_or_default(),
                            themes: themes.cloned().unwrap_or_default(),
                        };
                        index.entries.insert(meta.url.clone(), entry);
                    }

                    // Step 5: atomically write the rebuilt index.
                    if let Err(e) =
                        crate::entity_index_store::save_entity_index(&entity_index_path, &index)
                    {
                        engine_error!("[entity-index] rebuild: save failed: {}", e);
                        let _ = msg_tx.send(Msg::EntityIndexRebuildFailed {
                            reason: format!("save failed: {e}"),
                        });
                        return;
                    }

                    engine_info!(
                        "[entity-index] rebuild complete: {} entries",
                        index.entries.len()
                    );
                    let _ = msg_tx.send(Msg::EntityIndexRebuilt { index });
                });
            }
            Effect::UpsertEntityIndexEntry {
                url,
                fetched_utc,
                content_hash,
                summary_entities,
                themes,
            } => {
                let patch = crate::entity_index_store::EntityIndexPatch {
                    fetched_utc,
                    content_hash,
                    summary_entities,
                    themes,
                };
                if let Err(e) = self
                    .entity_index_worker_tx
                    .send(EntityIndexWorkerMsg::Upsert { url, patch })
                {
                    engine_error!("[entity-index] worker channel closed, upsert dropped: {e}");
                }
            }

            // --- Import saved webpages ---
            Effect::ImportSavedWebpages { dir, request_id } => {
                let msg_tx = self.msg_tx.clone();
                let archive_dir = self.paths.output_dir.clone();
                thread::spawn(move || {
                    engine_info!(
                        "[import-saved-web] start id={request_id} dir={}",
                        dir.display()
                    );
                    let options = ImportOptions { archive_dir };
                    let report = import_saved_webpages(&dir, &options);
                    engine_info!(
                        "[import-saved-web] done id={request_id} imported={} failed={}",
                        report.imported_entries.len(),
                        report.failures.len()
                    );
                    let _ = msg_tx.send(harvester_core::Msg::ImportSavedWebpagesCompleted {
                        request_id,
                        report,
                    });
                });
            }
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
        engine_error!("[effect] Rejected: {:?} — {}", effect, reason);
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

    fn execute_poll_all_sources(&self) {
        let msg_tx = self.msg_tx.clone();
        let sources_path = self.paths.sources_path.clone();
        let seen_set_path = self.paths.seen_set_path.clone();
        let output_dir = self.paths.output_dir.clone();
        let url_policy = self.url_policy.clone();
        let fetch_settings = self.fetch_settings.clone();

        thread::spawn(move || {
            let _guard = PollGuard::new(msg_tx.clone());
            let poll_started = Instant::now();

            engine_info!("[source-config] loading {}", sources_path.display());
            let registry = load_sources(&sources_path);
            let config_dir = sources_path
                .parent()
                .unwrap_or_else(|| std::path::Path::new("."))
                .to_path_buf();
            let allowed_dirs = vec![config_dir.clone(), output_dir.clone()];

            let mut seen_set = load_seen_set(&seen_set_path);

            let mut enabled_sources = 0usize;
            for config in registry.sources.into_iter().filter(|s| s.enabled) {
                enabled_sources += 1;
                let source_id = config.id.clone();
                let source_started = Instant::now();
                match config.source_type {
                    SourceType::File { path } => match poll_file_source(
                        source_id.clone(),
                        &path,
                        &config_dir,
                        &allowed_dirs,
                        config.max_urls_per_poll,
                    ) {
                        Ok(result) => {
                            engine_info!(
                                "[file-poll] {} => {} URL(s)",
                                source_id,
                                result.urls.len()
                            );
                            engine_info!(
                                "[poll-all-timing] source={} kind=file status=ok urls={} elapsed_ms={}",
                                source_id,
                                result.urls.len(),
                                source_started.elapsed().as_millis()
                            );
                            let _ = msg_tx.send(Msg::SourcePollCompleted {
                                source_id,
                                urls: result.urls,
                            });
                        }
                        Err(err) => {
                            engine_warn!("[file-poll] {} failed: {}", source_id, err);
                            engine_warn!(
                                "[poll-all-timing] source={} kind=file status=err elapsed_ms={} error={}",
                                source_id,
                                source_started.elapsed().as_millis(),
                                err
                            );
                            let _ = msg_tx.send(Msg::SourcePollFailed {
                                source_id,
                                error: err.to_string(),
                            });
                        }
                    },
                    SourceType::CuratedList { urls } => {
                        let result =
                            poll_curated_source(source_id.clone(), &urls, config.max_urls_per_poll);
                        engine_info!(
                            "[curated-poll] {} => {} URL(s)",
                            source_id,
                            result.urls.len()
                        );
                        engine_info!(
                            "[poll-all-timing] source={} kind=curated status=ok urls={} elapsed_ms={}",
                            source_id,
                            result.urls.len(),
                            source_started.elapsed().as_millis()
                        );
                        let _ = msg_tx.send(Msg::SourcePollCompleted {
                            source_id,
                            urls: result.urls,
                        });
                    }
                    SourceType::Script { .. } => {
                        engine_warn!("[poll-all] Script sources not yet supported: {}", source_id);
                        engine_warn!(
                            "[poll-all-timing] source={} kind=script status=unsupported elapsed_ms={}",
                            source_id,
                            source_started.elapsed().as_millis()
                        );
                        let _ = msg_tx.send(Msg::SourcePollFailed {
                            source_id,
                            error: "Script sources not implemented".to_string(),
                        });
                    }
                    SourceType::Rss { feed_url } => {
                        let mut context = RssPollContext {
                            seen_set: &mut seen_set,
                            seen_set_path: &seen_set_path,
                            msg_tx: &msg_tx,
                        };
                        handle_rss_source_poll(
                            &source_id,
                            &feed_url,
                            &url_policy,
                            &fetch_settings,
                            &mut context,
                            config.max_urls_per_poll,
                        );
                        engine_info!(
                            "[poll-all-timing] source={} kind=rss elapsed_ms={}",
                            source_id,
                            source_started.elapsed().as_millis()
                        );
                    }
                }
            }
            engine_info!(
                "[poll-all-timing] all-sources completed enabled_sources={} elapsed_ms={}",
                enabled_sources,
                poll_started.elapsed().as_millis()
            );
        });
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

/// Serialized entity-index worker.
///
/// Processes `EntityIndexWorkerMsg::Upsert` messages one at a time, performing
/// a full load → merge → atomic-write cycle per message to ensure no concurrent writes.
///
/// Exits when the sender (`entity_index_worker_tx`) is dropped (channel closed).
fn run_entity_index_worker(rx: mpsc::Receiver<EntityIndexWorkerMsg>, path: PathBuf) {
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
fn run_triage_refresh_load(
    request_id: u64,
    ordered_urls: Vec<String>,
    since_utc: Option<chrono::DateTime<chrono::Utc>>,
    msg_tx: mpsc::Sender<Msg>,
    output_dir: PathBuf,
    registry: Arc<RwLock<PromptRegistry>>,
    max_input_bytes: usize,
) {
    let load_started = Instant::now();
    engine_info!(
        "[pre-triage-refresh] load start request_id={} urls={}",
        request_id,
        ordered_urls.len(),
    );

    let guard = registry.read().unwrap();
    match load_and_prepare_articles_filtered(
        &output_dir,
        max_input_bytes,
        &guard,
        &ordered_urls,
        since_utc,
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
            engine_info!(
                "[pre-triage-refresh] load done request_id={} urls={} prepared={} elapsed_ms={}",
                request_id,
                ordered_urls.len(),
                articles.len(),
                load_started.elapsed().as_millis()
            );
            let _ = msg_tx.send(Msg::TriageArticlesLoaded {
                request_id,
                articles,
            });
        }
        Err(reason) => {
            engine_warn!(
                "[pre-triage-refresh] load failed request_id={} urls={} elapsed_ms={} reason={}",
                request_id,
                ordered_urls.len(),
                load_started.elapsed().as_millis(),
                reason
            );
            let _ = msg_tx.send(Msg::TriageArticlesLoadFailed { request_id, reason });
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use harvester_engine::llm::load_context_file;
    use harvester_engine::llm::types::ProviderKind;
    use harvester_engine::llm::DEFAULT_BRIEFING_MODEL;
    use harvester_engine::llm::{LlmCompletionError, LlmEvent};
    use std::fs;
    use std::path::Path;
    use std::sync::mpsc;
    use std::time::Duration;
    use tempfile::tempdir;

    fn make_test_runtime_paths(base: &Path) -> RuntimePaths {
        RuntimePaths {
            output_dir: base.to_path_buf(),
            contexts_dir: base.join("contexts"),
            prompts_dir: base.join("prompts"),
            sources_path: base.join("sources.ron"),
            seen_set_path: base.join("seen_set.ron"),
            summary_cache_path: base.join("summary_cache.ron"),
            triage_cache_path: base.join("triage_cache.ron"),
            state_path: base.join("state.json"),
            briefing_history_path: base.join(".briefing_history.ron"),
            briefing_checkpoint_path: base.join(".briefing_checkpoint.ron"),
            entity_index_path: base.join(".entity_index.ron"),
        }
    }

    fn runner_with_receiver(base: &Path) -> (EffectRunner, mpsc::Receiver<Msg>) {
        let (tx, rx) = mpsc::channel();
        let paths = make_test_runtime_paths(base);
        let platform_handler = Box::new(NoOpPlatformHandler);
        (EffectRunner::new(paths, tx, platform_handler), rx)
    }

    fn write_markdown(dir: &Path, filename: &str, url: &str) {
        use harvester_engine::{build_markdown_document, WhitespaceTokenCounter};
        let counter = WhitespaceTokenCounter;
        let (_, markdown) = build_markdown_document(
            url,
            Some("Title"),
            "utf-8",
            "2026-02-14T00:00:00Z",
            "body",
            &counter,
        );
        fs::write(dir.join(filename), markdown).expect("write markdown");
    }

    fn write_markdown_with_fetched_utc(dir: &Path, filename: &str, url: &str, fetched_utc: &str) {
        use harvester_engine::{build_markdown_document, WhitespaceTokenCounter};
        let counter = WhitespaceTokenCounter;
        let (_, markdown) =
            build_markdown_document(url, Some("Title"), "utf-8", fetched_utc, "body", &counter);
        fs::write(dir.join(filename), markdown).expect("write markdown");
    }

    #[test]
    fn build_local_model_catalog_uses_effective_models_with_dedup_and_sort() {
        let mut effective_models = HashMap::new();
        effective_models.insert(PromptId::ArticleTriage, "gpt-4o-mini".to_string());
        effective_models.insert(PromptId::ArticleSummary, "o3-mini".to_string());
        effective_models.insert(
            PromptId::AggregateBriefing,
            DEFAULT_BRIEFING_MODEL.to_string(),
        );

        let models = build_local_model_catalog(Some(ProviderKind::OpenAi), &effective_models);
        let names: Vec<_> = models.iter().map(|m| m.model_name().to_string()).collect();

        assert_eq!(
            names,
            vec![
                "gpt-4o-mini".to_string(),
                DEFAULT_BRIEFING_MODEL.to_string(),
                "o3-mini".to_string()
            ]
        );
    }

    #[test]
    fn build_local_model_catalog_returns_empty_without_provider_kind() {
        let mut effective_models = HashMap::new();
        effective_models.insert(PromptId::ArticleTriage, "gpt-4o-mini".to_string());

        let models = build_local_model_catalog(None, &effective_models);

        assert!(models.is_empty());
    }

    #[test]
    fn download_link_page_rejects_disallowed_scheme_before_request() {
        let temp = tempdir().expect("tempdir");
        let fetch_settings = FetchSettings::default();
        let policy = UrlPolicy::default();
        let err = download_link_page("file:///etc/passwd", temp.path(), &policy, &fetch_settings)
            .unwrap_err();

        assert!(
            err.contains("url policy violation"),
            "expected url policy error, got '{}'",
            err
        );
    }

    #[test]
    fn enqueue_url_effect_is_rejected_by_url_policy() {
        let temp = tempdir().expect("tempdir");
        let (runner, rx) = runner_with_receiver(temp.path());
        let job_id = 42;
        runner.enqueue(vec![Effect::EnqueueUrl {
            job_id,
            url: "file:///etc/passwd".to_string(),
        }]);

        let msg = rx
            .recv_timeout(Duration::from_secs(1))
            .expect("expected job done msg");

        match msg {
            Msg::JobDone {
                job_id: received,
                result: JobResultKind::Failed { reason },
                ..
            } => {
                assert_eq!(received, job_id);
                assert!(reason.contains("url policy"));
            }
            other => panic!("unexpected message: {:?}", other),
        }
    }

    #[test]
    fn download_link_page_effect_is_rejected_by_authorization() {
        let temp = tempdir().expect("tempdir");
        let (runner, rx) = runner_with_receiver(temp.path());
        let job_id = 7;
        let link_index = 1;
        runner.enqueue(vec![Effect::DownloadLinkedPage {
            job_id,
            link_index,
            url: "file:///tmp/secret".to_string(),
        }]);

        let msg = rx
            .recv_timeout(Duration::from_secs(1))
            .expect("expected link download failed msg");

        match msg {
            Msg::LinkDownloadFailed {
                job_id: received,
                link_index: received_index,
                error,
            } => {
                assert_eq!(received, job_id);
                assert_eq!(received_index, link_index);
                assert!(error.contains("url policy"));
            }
            other => panic!("unexpected message: {:?}", other),
        }
    }

    #[test]
    fn delete_linked_page_effect_is_rejected_on_unsafe_path() {
        let temp = tempdir().expect("tempdir");
        let (runner, rx) = runner_with_receiver(temp.path());
        let job_id = 11;
        let link_index = 3;
        runner.enqueue(vec![Effect::DeleteLinkedPage {
            job_id,
            link_index,
            path: std::path::PathBuf::from("../outside.md"),
        }]);

        let msg = rx
            .recv_timeout(Duration::from_secs(1))
            .expect("expected link deleted msg");

        match msg {
            Msg::LinkDeleted {
                job_id: received,
                link_index: received_index,
            } => {
                assert_eq!(received, job_id);
                assert_eq!(received_index, link_index);
            }
            other => panic!("unexpected message: {:?}", other),
        }
    }

    #[test]
    fn save_prompt_context_file_writes_file_and_dispatches_saved_msg() {
        let temp = tempdir().expect("tempdir");
        let (runner, rx) = runner_with_receiver(temp.path());
        let prompt_id = PromptId::ArticleTriage;
        runner.enqueue(vec![Effect::SavePromptContextFile {
            prompt_id,
            context_pairs: vec![("foo".into(), "bar".into())],
        }]);

        let msg = rx
            .recv_timeout(Duration::from_secs(1))
            .expect("expected context saved msg");

        match msg {
            Msg::PromptLabContextSaved {
                prompt_id: received,
                path,
                version,
            } => {
                assert_eq!(received, prompt_id);
                assert_eq!(version, 1);
                let saved =
                    load_context_file(&std::path::PathBuf::from(&path)).expect("load saved");
                assert_eq!(saved.variables.get("foo").map(String::as_str), Some("bar"));
            }
            other => panic!("unexpected message: {:?}", other),
        }
    }

    #[test]
    fn save_prompt_context_file_reports_failure_when_existing_file_invalid() {
        let temp = tempdir().expect("tempdir");
        let contexts_dir = temp.path().join("contexts");
        fs::create_dir_all(&contexts_dir).expect("create contexts dir");
        fs::write(contexts_dir.join("article_triage.toml"), "bad toml")
            .expect("write invalid file");

        let (runner, rx) = runner_with_receiver(temp.path());
        runner.enqueue(vec![Effect::SavePromptContextFile {
            prompt_id: PromptId::ArticleTriage,
            context_pairs: vec![("foo".into(), "bar".into())],
        }]);

        let msg = rx
            .recv_timeout(Duration::from_secs(1))
            .expect("expected save failed msg");

        match msg {
            Msg::PromptLabContextSaveFailed { prompt_id, reason } => {
                assert_eq!(prompt_id, PromptId::ArticleTriage);
                assert!(reason.contains("failed to read existing context"));
            }
            other => panic!("unexpected message: {:?}", other),
        }
    }

    #[test]
    fn save_prompt_template_file_writes_file_and_dispatches_saved_msg() {
        let temp = tempdir().expect("tempdir");
        let (runner, rx) = runner_with_receiver(temp.path());
        let prompt_id = PromptId::ArticleTriage;
        runner.enqueue(vec![Effect::SavePromptTemplateFile {
            prompt_id,
            system_template: "system {{context}}".to_string(),
            user_template: "user {{context}}".to_string(),
            description: "desc".to_string(),
            expected_format: "json".to_string(),
        }]);

        let msg = rx
            .recv_timeout(Duration::from_secs(1))
            .expect("expected template saved msg");

        match msg {
            Msg::PromptLabTemplateSaved {
                prompt_id: received,
                version,
                path,
            } => {
                assert_eq!(received, prompt_id);
                assert_eq!(version, 1);
                let file = fs::read_to_string(std::path::PathBuf::from(&path))
                    .expect("read saved template file");
                assert!(file.contains("system {{context}}"));
                assert!(file.contains("user {{context}}"));
            }
            other => panic!("unexpected message: {:?}", other),
        }
    }

    #[test]
    fn load_articles_for_briefing_prereq_dispatches_loaded_message() {
        let temp = tempdir().expect("tempdir");
        write_markdown(temp.path(), "a.md", "https://example.com/a");
        let (runner, rx) = runner_with_receiver(temp.path());
        runner.enqueue(vec![Effect::LoadArticlesForBriefingPrereq {
            ordered_urls: vec!["https://example.com/a".to_string()],
            since_utc: None,
        }]);

        let msg = rx
            .recv_timeout(Duration::from_secs(1))
            .expect("expected prereq loaded message");
        match msg {
            Msg::BriefingPrereqArticlesLoaded { articles } => {
                assert_eq!(articles.len(), 1);
                assert_eq!(articles[0].url, "https://example.com/a");
            }
            other => panic!("unexpected message: {:?}", other),
        }
    }

    #[test]
    fn load_articles_for_triage_respects_since_utc_filter() {
        let temp = tempdir().expect("tempdir");
        write_markdown_with_fetched_utc(
            temp.path(),
            "old.md",
            "https://example.com/old",
            "2026-03-20T00:00:00Z",
        );
        write_markdown_with_fetched_utc(
            temp.path(),
            "new.md",
            "https://example.com/new",
            "2026-03-22T00:00:00Z",
        );
        let (runner, rx) = runner_with_receiver(temp.path());
        let since = chrono::DateTime::parse_from_rfc3339("2026-03-21T18:17:00Z")
            .unwrap()
            .with_timezone(&chrono::Utc);

        runner.enqueue(vec![Effect::LoadArticlesForTriage {
            request_id: 1,
            ordered_urls: vec![
                "https://example.com/old".to_string(),
                "https://example.com/new".to_string(),
            ],
            since_utc: Some(since),
        }]);

        let msg = rx
            .recv_timeout(Duration::from_secs(1))
            .expect("expected triage loaded message");
        match msg {
            Msg::TriageArticlesLoaded {
                request_id,
                articles,
            } => {
                assert_eq!(request_id, 1);
                assert_eq!(articles.len(), 1);
                assert_eq!(articles[0].url, "https://example.com/new");
            }
            other => panic!("unexpected message: {:?}", other),
        }
    }

    #[test]
    fn load_articles_for_briefing_with_empty_ordered_urls_dispatches_empty_articles_loaded() {
        let temp = tempdir().expect("tempdir");
        write_markdown(temp.path(), "a.md", "https://example.com/a");
        let (runner, rx) = runner_with_receiver(temp.path());
        runner.enqueue(vec![Effect::LoadArticlesForBriefing {
            ordered_urls: Vec::new(),
            since_utc: None,
        }]);

        let msg = rx
            .recv_timeout(Duration::from_secs(1))
            .expect("expected articles loaded message");
        match msg {
            Msg::ArticlesLoaded {
                articles,
                collection_text,
            } => {
                assert!(articles.is_empty());
                assert!(collection_text.is_empty());
            }
            other => panic!("unexpected message: {:?}", other),
        }
    }

    // --- map_llm_event failure metadata propagation tests ---

    fn make_failure_metadata() -> harvester_engine::llm::run_metadata::LlmFailureMetadata {
        use harvester_engine::llm::run_metadata::LlmFailureMetadata;
        LlmFailureMetadata {
            prompt_id: PromptId::ArticleTriage,
            prompt_version: 1,
            resolved_model: Some("gpt-4o-mini".to_string()),
            input_bytes: 100,
            wall_ms: Some(200),
            timestamp_utc: "2026-02-15T00:00:00Z".to_string(),
        }
    }

    #[test]
    fn map_llm_event_validation_failed_with_metadata_propagates_it() {
        let failure_metadata = make_failure_metadata();
        let event = LlmEvent::Completed {
            request_id: 1,
            result: Err(LlmCompletionError::ValidationFailed {
                reason: "bad json".to_string(),
                raw_response: "{}".to_string(),
                failure_metadata: Some(failure_metadata),
            }),
        };
        let msg = map_llm_event(event);
        if let Msg::LlmCompleted { metadata, .. } = msg {
            assert!(
                metadata.is_some(),
                "ValidationFailed with metadata should propagate it"
            );
            assert!(!metadata.unwrap().parse_ok);
        } else {
            panic!("expected LlmCompleted");
        }
    }

    #[test]
    fn map_llm_event_quota_exhausted_with_metadata_propagates_it() {
        let failure_metadata = make_failure_metadata();
        let event = LlmEvent::Completed {
            request_id: 1,
            result: Err(LlmCompletionError::QuotaExhausted {
                description: "rate limited".to_string(),
                failure_metadata: Some(failure_metadata),
            }),
        };
        let msg = map_llm_event(event);
        if let Msg::LlmCompleted { metadata, .. } = msg {
            assert!(
                metadata.is_some(),
                "QuotaExhausted with metadata should propagate it"
            );
        } else {
            panic!("expected LlmCompleted");
        }
    }

    #[test]
    fn map_llm_event_persistence_failed_with_metadata_propagates_it() {
        let failure_metadata = make_failure_metadata();
        let event = LlmEvent::Completed {
            request_id: 1,
            result: Err(LlmCompletionError::PersistenceFailed {
                detail: "disk full".to_string(),
                failure_metadata: Some(failure_metadata),
            }),
        };
        let msg = map_llm_event(event);
        if let Msg::LlmCompleted { metadata, .. } = msg {
            assert!(
                metadata.is_some(),
                "PersistenceFailed with metadata should propagate it"
            );
        } else {
            panic!("expected LlmCompleted");
        }
    }

    #[test]
    fn map_llm_event_unsupported_model_has_none_metadata() {
        use harvester_engine::llm::types::{ModelId, ProviderKind};
        let event = LlmEvent::Completed {
            request_id: 1,
            result: Err(LlmCompletionError::UnsupportedModel {
                model: ModelId::new(ProviderKind::OpenAi, "bad-model"),
                reason: "unknown".to_string(),
            }),
        };
        let msg = map_llm_event(event);
        if let Msg::LlmCompleted {
            metadata, result, ..
        } = msg
        {
            assert!(
                metadata.is_none(),
                "UnsupportedModel is pre-flight so metadata=None"
            );
            assert!(matches!(result, LlmResultKind::Failed { .. }));
        } else {
            panic!("expected LlmCompleted");
        }
    }

    #[test]
    fn resolve_effect_success_emits_ok_msg() {
        let temp = tempdir().expect("tempdir");
        write_markdown(temp.path(), "a.md", "https://example.com/a");
        let (runner, rx) = runner_with_receiver(temp.path());
        runner.enqueue(vec![Effect::ResolvePromptLabInputFromUrl {
            resolve_id: 7,
            url: "https://example.com/a".to_string(),
        }]);

        let msg = rx
            .recv_timeout(Duration::from_secs(2))
            .expect("expected prompt lab resolve msg");
        match msg {
            Msg::PromptLabInputResolved {
                resolve_id,
                result: Ok(snapshot),
            } => {
                assert_eq!(resolve_id, 7);
                assert!(!snapshot.is_empty());
            }
            other => panic!("unexpected message: {:?}", other),
        }
    }

    #[test]
    fn resolve_effect_failure_emits_err_msg() {
        let temp = tempdir().expect("tempdir");
        let (runner, rx) = runner_with_receiver(temp.path());
        runner.enqueue(vec![Effect::ResolvePromptLabInputFromUrl {
            resolve_id: 8,
            url: "https://example.com/missing".to_string(),
        }]);

        let msg = rx
            .recv_timeout(Duration::from_secs(2))
            .expect("expected prompt lab resolve msg");
        match msg {
            Msg::PromptLabInputResolved {
                resolve_id,
                result: Err(reason),
            } => {
                assert_eq!(resolve_id, 8);
                assert!(!reason.is_empty());
            }
            other => panic!("unexpected message: {:?}", other),
        }
    }

    #[test]
    fn archive_requested_writes_archive_markdown_for_selected_urls() {
        let temp = tempdir().expect("tempdir");
        let output = temp.path();
        let md_a = "---\nurl: \"https://example.com/a\"\ntitle: \"A\"\ntoken_count: 1\nfetched_utc: \"2026-02-10T00:00:00Z\"\nencoding: \"UTF-8\"\n---\n\nA body\n";
        let md_b = "---\nurl: \"https://example.com/b\"\ntitle: \"B\"\ntoken_count: 2\nfetched_utc: \"2026-02-11T00:00:00Z\"\nencoding: \"UTF-8\"\n---\n\nB body\n";
        fs::write(output.join("a.md"), md_a).expect("write a");
        fs::write(output.join("b.md"), md_b).expect("write b");

        let (runner, _rx) = runner_with_receiver(output);
        runner.enqueue(vec![Effect::ArchiveRequested {
            ordered_urls: vec!["https://example.com/b".to_string()],
            since_utc: None,
        }]);

        let mut archive_path = None;
        for _ in 0..40 {
            if let Ok(entries) = fs::read_dir(output) {
                for entry in entries.flatten() {
                    let path = entry.path();
                    let is_archive = path
                        .file_name()
                        .and_then(|n| n.to_str())
                        .map(|n| n.starts_with("archive-") && n.ends_with(".md"))
                        .unwrap_or(false);
                    if is_archive {
                        archive_path = Some(path);
                        break;
                    }
                }
            }
            if archive_path.is_some() {
                break;
            }
            thread::sleep(Duration::from_millis(25));
        }

        let archive_path = archive_path.expect("archive-[FROM]-[TO].md should be written");
        let archive_name = archive_path.file_name().and_then(|n| n.to_str()).unwrap();
        assert_eq!(archive_name, "archive-all-2026-02-11.md");
        let archive = fs::read_to_string(&archive_path).expect("read archive");
        assert!(!archive.contains("url: https://example.com/b\n"));
        assert!(!archive.contains("url: https://example.com/a\n"));
        assert!(archive.contains("url: \"https://example.com/b\""));
        assert!(!archive.contains("url: \"https://example.com/a\""));
    }
}
