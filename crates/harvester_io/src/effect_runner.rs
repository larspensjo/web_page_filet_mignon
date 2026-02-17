use std::collections::HashMap;
use std::fs;
use std::sync::{mpsc, Arc, RwLock};
use std::thread;
use std::time::Duration;

use chrono::Utc;
use engine_logging::{engine_error, engine_info, engine_warn};
use harvester_core::{Effect, JobResultKind, LlmResultKind, LoadedArticle, Msg, StopPolicy};
use harvester_engine::llm::load_context_file;
use harvester_engine::llm::prompt::{PromptId, PromptTemplateOwned, PROMPT_VERSION_DRAFT};
use harvester_engine::llm::prompt_context::{ContextMeta, PromptContextFile};
use harvester_engine::llm::types::ProviderKind;
use harvester_engine::llm::{LlmCommand, LlmHandle, PromptRegistry};
use harvester_engine::{
    is_confined_to, load_and_prepare_articles_filtered, poll_curated_source, poll_file_source,
    EngineConfig, EngineEvent, EngineHandle, FetchSettings, SourceType, UrlPolicy,
};

use crate::effect_helpers::{
    build_local_model_catalog, download_link_page, handle_rss_source_poll, map_llm_event,
    map_stage, prompt_context_filename, PollGuard, RssPollContext,
};
use crate::{load_seen_set, load_sources, RuntimePaths};

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

/// Effect runner that orchestrates IO effects
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
        };
        runner.spawn_event_loop(msg_tx);
        runner
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
                engine_info!("EnqueueUrl job_id={} url_len={} url={}", job_id, url.len(), url);
                self.engine.enqueue(job_id, url);
            }
            Effect::StartSession => {
                // no-op; engine starts on first enqueue
            }
            Effect::StopFinish { policy } => {
                let immediate = matches!(policy, StopPolicy::Immediate);
                self.engine.stop(immediate);
            }
            Effect::ArchiveRequested => {
                engine_info!("Archive requested: enqueue export job");
                self.engine.request_export();
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
                    engine_info!("[prompt-lab] resolve requested resolve_id={} url={}", resolve_id, url);
                    let guard = registry.read().unwrap();
                    match load_and_prepare_articles_filtered(
                        &output_dir,
                        max_input_bytes,
                        &guard,
                        std::slice::from_ref(&url),
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
                                        .map(|name| harvester_engine::llm::types::ModelId::new(provider_kind, name))
                                        .collect();
                                    (models, harvester_core::ModelCatalogSource::Remote)
                                }
                                Err(err) => {
                                    engine_warn!(
                                        "[prompt-lab-model] remote discovery failed: {}",
                                        err
                                    );
                                    (local_fallback_models, harvester_core::ModelCatalogSource::LocalFallback)
                                }
                            },
                            Err(err) => {
                                engine_warn!("[prompt-lab-model] tokio runtime creation failed: {}", err);
                                (local_fallback_models, harvester_core::ModelCatalogSource::LocalFallback)
                            }
                        }
                    } else {
                        (local_fallback_models, harvester_core::ModelCatalogSource::LocalFallback)
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
                        engine_error!("[prompt-lab-context] SavePromptContextFile {} prompt_id={:?}", reason, prompt_id);
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
                                engine_error!("[prompt-lab-context] SavePromptContextFile {} prompt_id={:?}", reason, prompt_id);
                                let _ = msg_tx.send(Msg::PromptLabContextSaveFailed { prompt_id, reason });
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

                    let ctx_file = PromptContextFile { meta: meta.clone(), variables };

                    let mut toml_string = match toml::to_string(&ctx_file) {
                        Ok(serialized) => serialized,
                        Err(err) => {
                            let reason = format!("failed to serialize context: {}", err);
                            engine_error!("[prompt-lab-context] SavePromptContextFile {} prompt_id={:?}", reason, prompt_id);
                            let _ = msg_tx.send(Msg::PromptLabContextSaveFailed { prompt_id, reason });
                            return;
                        }
                    };
                    toml_string.push('\n');

                    let tmp_path = path.with_extension("toml.tmp");
                    if let Err(err) = fs::write(&tmp_path, toml_string) {
                        let reason = format!("failed to write temp file: {}", err);
                        engine_error!("[prompt-lab-context] SavePromptContextFile {} prompt_id={:?}", reason, prompt_id);
                        let _ = msg_tx.send(Msg::PromptLabContextSaveFailed { prompt_id, reason });
                        return;
                    }

                    if let Err(err) = fs::rename(&tmp_path, &path) {
                        let reason = format!("failed to rename temp file: {}", err);
                        engine_error!("[prompt-lab-context] SavePromptContextFile {} prompt_id={:?}", reason, prompt_id);
                        let _ = msg_tx.send(Msg::PromptLabContextSaveFailed { prompt_id, reason });
                        return;
                    }

                    engine_info!("[prompt-lab-context] Saved context for {:?} to {:?}", prompt_id, path);
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
                    match crate::save_prompt_template(&prompts_dir, prompt_id, &system_template, &user_template, &description, &expected_format) {
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
                            let _ = msg_tx.send(Msg::PromptLabTemplateSaveFailed { prompt_id, reason });
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
                        },
                    ));
                    if let Err(err) = handle.send(cmd) {
                        engine_warn!("LLM completion request failed to dispatch: request_id={} error={:?}", request_id, err);
                        let _ = self.msg_tx.send(Msg::LlmCompleted {
                            request_id,
                            result: LlmResultKind::Failed {
                                reason: "LLM worker unavailable".to_string(),
                            },
                            metadata: None,
                        });
                    } else {
                        engine_info!("[llm-dispatch] request_id={} prompt_id={:?}", request_id, prompt_id);
                    }
                } else {
                    engine_warn!("LLM completion requested without handle: request_id={}", request_id);
                    let _ = self.msg_tx.send(Msg::LlmCompleted {
                        request_id,
                        result: LlmResultKind::Failed {
                            reason: "LLM not configured".to_string(),
                        },
                        metadata: None,
                    });
                }
            }
            Effect::LoadArticlesForBriefing { ordered_urls } => {
                let msg_tx = self.msg_tx.clone();
                let output_dir = self.paths.output_dir.clone();
                let max_input_bytes = self.llm_max_input_bytes.unwrap_or(100_000);
                let registry = self.prompt_registry.clone();
                thread::spawn(move || {
                    let guard = registry.read().unwrap();
                    match load_and_prepare_articles_filtered(&output_dir, max_input_bytes, &guard, &ordered_urls) {
                        Ok((articles, collection_text)) => {
                            let loaded_articles: Vec<LoadedArticle> = articles
                                .into_iter()
                                .map(|article| LoadedArticle {
                                    url: article.url,
                                    source_title: article.source_title,
                                    prepared_text: article.prepared_text,
                                    content_hash: article.content_hash,
                                })
                                .collect();
                            engine_info!("[briefing-loader] prepared {} article(s)", loaded_articles.len());
                            let _ = msg_tx.send(Msg::ArticlesLoaded {
                                articles: loaded_articles,
                                collection_text,
                            });
                        }
                        Err(reason) => {
                            engine_warn!("[briefing-loader] load failed: {}", reason);
                            let _ = msg_tx.send(Msg::ArticlesLoadFailed { reason });
                        }
                    }
                });
            }
            Effect::LoadArticlesForBriefingPrereq { ordered_urls } => {
                let msg_tx = self.msg_tx.clone();
                let output_dir = self.paths.output_dir.clone();
                let max_input_bytes = self.llm_max_input_bytes.unwrap_or(100_000);
                let registry = self.prompt_registry.clone();
                thread::spawn(move || {
                    let guard = registry.read().unwrap();
                    match load_and_prepare_articles_filtered(&output_dir, max_input_bytes, &guard, &ordered_urls) {
                        Ok((engine_articles, _)) => {
                            let articles: Vec<LoadedArticle> = engine_articles
                                .into_iter()
                                .map(|article| LoadedArticle {
                                    url: article.url,
                                    source_title: article.source_title,
                                    prepared_text: article.prepared_text,
                                    content_hash: article.content_hash,
                                })
                                .collect();
                            let _ = msg_tx.send(Msg::BriefingPrereqArticlesLoaded { articles });
                        }
                        Err(reason) => {
                            let _ = msg_tx.send(Msg::BriefingPrereqLoadFailed { reason });
                        }
                    }
                });
            }
            Effect::LoadArticlesForTriage { ordered_urls } => {
                let msg_tx = self.msg_tx.clone();
                let output_dir = self.paths.output_dir.clone();
                let max_input_bytes = self.llm_max_input_bytes.unwrap_or(100_000);
                let registry = self.prompt_registry.clone();
                thread::spawn(move || {
                    let guard = registry.read().unwrap();
                    match load_and_prepare_articles_filtered(&output_dir, max_input_bytes, &guard, &ordered_urls) {
                        Ok((engine_articles, _)) => {
                            let articles: Vec<LoadedArticle> = engine_articles
                                .into_iter()
                                .map(|article| LoadedArticle {
                                    url: article.url,
                                    source_title: article.source_title,
                                    prepared_text: article.prepared_text,
                                    content_hash: article.content_hash,
                                })
                                .collect();
                            let _ = msg_tx.send(Msg::TriageArticlesLoaded { articles });
                        }
                        Err(reason) => {
                            let _ = msg_tx.send(Msg::TriageArticlesLoadFailed { reason });
                        }
                    }
                });
            }
            Effect::LoadPromptContexts => {
                let msg_tx = self.msg_tx.clone();
                let contexts_dir = self.paths.contexts_dir.clone();
                thread::spawn(move || {
                    if !contexts_dir.exists() {
                        engine_warn!("[PromptContext] contexts directory not found at {:?}", contexts_dir);
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
                                engine_warn!("[prompt-lab-template] Failed to load saved template: {}", reason);
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
                            engine_warn!("[summary-cache] Failed to persist cache to {:?}: {}", path, err);
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
                            engine_warn!("[triage-cache] Failed to persist cache to {:?}: {}", path, err);
                        }
                    }
                    // Fire-and-forget, no message sent
                    let _ = msg_tx;
                });
            }
            Effect::PollAllSources => {
                self.execute_poll_all_sources();
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
                            },
                            Err(failure_kind) => {
                                let reason = failure_kind.to_string();
                                engine_warn!("Job {} failed: {}", job_id, reason);
                                Msg::JobDone {
                                    job_id,
                                    result: JobResultKind::Failed { reason },
                                    content_preview: None,
                                    extracted_links: Vec::new(),
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
                let parsed = url::Url::parse(url)
                    .map_err(|err| format!("invalid url {}: {}", url, err))?;
                self.url_policy
                    .check(&parsed)
                    .map_err(|violation| format!("url policy violation: {}", violation))?;
                Ok(())
            }
            Effect::DownloadLinkedPage { url, .. } => {
                let parsed = url::Url::parse(url)
                    .map_err(|err| format!("invalid linked page url {}: {}", url, err))?;
                self.url_policy
                    .check(&parsed)
                    .map_err(|violation| format!("linked page url policy violation: {}", violation))?;
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
                });
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

            engine_info!("[source-config] loading {}", sources_path.display());
            let registry = load_sources(&sources_path);
            let config_dir = sources_path
                .parent()
                .unwrap_or_else(|| std::path::Path::new("."))
                .to_path_buf();
            let allowed_dirs = vec![config_dir.clone(), output_dir.clone()];

            let mut seen_set = load_seen_set(&seen_set_path);

            for config in registry.sources.into_iter().filter(|s| s.enabled) {
                let source_id = config.id.clone();
                match config.source_type {
                    SourceType::File { path } => match poll_file_source(
                        source_id.clone(),
                        &path,
                        &config_dir,
                        &allowed_dirs,
                        config.max_urls_per_poll,
                    ) {
                        Ok(result) => {
                            engine_info!("[file-poll] {} => {} URL(s)", source_id, result.urls.len());
                            let _ = msg_tx.send(Msg::SourcePollCompleted {
                                source_id,
                                urls: result.urls,
                            });
                        }
                        Err(err) => {
                            engine_warn!("[file-poll] {} failed: {}", source_id, err);
                            let _ = msg_tx.send(Msg::SourcePollFailed {
                                source_id,
                                error: err.to_string(),
                            });
                        }
                    },
                    SourceType::CuratedList { urls } => {
                        let result = poll_curated_source(
                            source_id.clone(),
                            &urls,
                            config.max_urls_per_poll,
                        );
                        engine_info!("[curated-poll] {} => {} URL(s)", source_id, result.urls.len());
                        let _ = msg_tx.send(Msg::SourcePollCompleted {
                            source_id,
                            urls: result.urls,
                        });
                    }
                    SourceType::Script { .. } => {
                        engine_warn!("[poll-all] Script sources not yet supported: {}", source_id);
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
                    }
                }
            }
        });
    }
}

impl Drop for EffectRunner {
    fn drop(&mut self) {
        engine_info!("[effect] EffectRunner dropped, stopping engine");
        self.engine.stop(false);
    }
}
