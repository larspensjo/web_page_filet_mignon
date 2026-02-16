use super::{prompt_template_store, seen_set_store, source_loader};
use std::collections::HashMap;
use std::fs;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::sync::{
    atomic::{AtomicUsize, Ordering},
    mpsc, Arc, RwLock,
};
use std::thread;
use std::time::Duration;
use std::{error::Error as StdError, fmt};

use chrono::Utc;
use engine_logging::{engine_error, engine_info, engine_warn};
use harvester_core::{
    Effect, JobResultKind, LlmResultKind, LoadedArticle, Msg, PromptLabTemplateSnapshot, Stage,
    StopPolicy,
};
use harvester_engine::llm::load_context_file;
use harvester_engine::llm::prompt::{PromptId, PromptTemplateOwned, PROMPT_VERSION_DRAFT};
use harvester_engine::llm::prompt_context::{ContextMeta, PromptContextFile};
use harvester_engine::{
    build_markdown_document, decode_html, deterministic_filename, ensure_output_dir,
    is_confined_to,
    llm::{LlmCommand, LlmCompletionError, LlmEvent, LlmHandle, LlmRunMetadata, PromptRegistry},
    load_and_prepare_articles_filtered, poll_curated_source, poll_file_source, poll_rss_source,
    AtomicFileWriter, Converter, DecodeError, EngineConfig, EngineEvent, EngineHandle, Extractor,
    FetchSettings, LinkExtractingConverter, ReadabilityLikeExtractor, RssSeenSet, SourceId,
    SourceType, UrlPolicy, WhitespaceTokenCounter,
};
use reqwest::blocking::Client;
use reqwest::header::{ACCEPT, CONTENT_TYPE};
use reqwest::redirect::Policy;
use url::Url;
const MAX_FEED_RESPONSE_BYTES: usize = 2 * 1024 * 1024;
const FEED_ACCEPT_HEADER: &str =
    "application/rss+xml, application/atom+xml, application/feed+json, application/json, application/xml, text/xml";

pub(crate) fn default_output_dir() -> std::path::PathBuf {
    std::env::current_dir()
        .unwrap_or_else(|_| std::path::PathBuf::from("."))
        .join("output")
}

fn contexts_directory() -> PathBuf {
    PathBuf::from("contexts")
}

fn prompt_context_filename(prompt_id: PromptId) -> &'static str {
    match prompt_id {
        PromptId::ArticleTriage => "article_triage.toml",
        PromptId::ArticleSummary => "article_summary.toml",
        PromptId::AggregateBriefing => "aggregate_briefing.toml",
    }
}

struct RssPollContext<'a> {
    seen_set: &'a mut RssSeenSet,
    seen_set_path: &'a Path,
    msg_tx: &'a mpsc::Sender<Msg>,
}

fn handle_rss_source_poll(
    source_id: &SourceId,
    feed_url: &str,
    url_policy: &UrlPolicy,
    fetch_settings: &FetchSettings,
    context: &mut RssPollContext,
    max_urls_per_poll: Option<usize>,
) {
    let bytes = match fetch_feed(feed_url, url_policy, fetch_settings) {
        Ok(bytes) => bytes,
        Err(reason) => {
            engine_warn!("[rss-poll] fetch failed for {}: {}", source_id, reason);
            let _ = context.msg_tx.send(Msg::SourcePollFailed {
                source_id: source_id.clone(),
                error: reason,
            });
            return;
        }
    };
    let result = match poll_rss_source(
        source_id.clone(),
        &bytes,
        feed_url,
        context.seen_set,
        max_urls_per_poll,
    ) {
        Ok(result) => result,
        Err(err) => {
            engine_warn!("[rss-poll] {} failed: {}", source_id, err);
            let _ = context.msg_tx.send(Msg::SourcePollFailed {
                source_id: source_id.clone(),
                error: err.to_string(),
            });
            return;
        }
    };
    if let Err(err) = seen_set_store::save_seen_set(context.seen_set, context.seen_set_path) {
        engine_warn!(
            "[rss-poll] failed to persist seen set for {}: {}",
            source_id,
            err
        );
    }
    engine_info!(
        "[rss-poll] {} => {} new URL(s)",
        source_id,
        result.urls.len()
    );
    let _ = context.msg_tx.send(Msg::SourcePollCompleted {
        source_id: source_id.clone(),
        urls: result.urls,
    });
}

pub struct EffectRunner {
    engine: EngineHandle,
    msg_tx: mpsc::Sender<Msg>,
    output_dir: PathBuf,
    url_policy: UrlPolicy,
    fetch_settings: FetchSettings,
    llm_handle: Option<LlmHandle>,
    llm_max_input_bytes: Option<usize>,
    prompt_registry: Arc<RwLock<PromptRegistry>>,
    llm_metadata_models: HashMap<PromptId, String>,
    llm_provider: Option<Arc<dyn harvester_engine::llm::provider::LlmProvider>>,
    llm_default_provider: Option<harvester_engine::llm::types::ProviderKind>,
}

impl EffectRunner {
    pub fn new(msg_tx: mpsc::Sender<Msg>) -> Self {
        let registry = Arc::new(RwLock::new(PromptRegistry::with_defaults()));
        Self::with_optional_llm(msg_tx, None, None, registry, HashMap::new(), None, None)
    }

    pub fn new_with_llm(
        msg_tx: mpsc::Sender<Msg>,
        llm_handle: LlmHandle,
        llm_max_input_bytes: usize,
        prompt_registry: Arc<RwLock<PromptRegistry>>,
        llm_metadata_models: HashMap<PromptId, String>,
        llm_provider: Arc<dyn harvester_engine::llm::provider::LlmProvider>,
        llm_default_provider: harvester_engine::llm::types::ProviderKind,
    ) -> Self {
        Self::with_optional_llm(
            msg_tx,
            Some(llm_handle),
            Some(llm_max_input_bytes),
            prompt_registry,
            llm_metadata_models,
            Some(llm_provider),
            Some(llm_default_provider),
        )
    }

    fn with_optional_llm(
        msg_tx: mpsc::Sender<Msg>,
        llm_handle: Option<LlmHandle>,
        llm_max_input_bytes: Option<usize>,
        prompt_registry: Arc<RwLock<PromptRegistry>>,
        llm_metadata_models: HashMap<PromptId, String>,
        llm_provider: Option<Arc<dyn harvester_engine::llm::provider::LlmProvider>>,
        llm_default_provider: Option<harvester_engine::llm::types::ProviderKind>,
    ) -> Self {
        let output_dir = default_output_dir();

        let mut config = EngineConfig::default_with_output(output_dir.clone());
        config.fetched_utc = std::sync::Arc::new(|| Utc::now().to_rfc3339());
        let url_policy = config.url_policy.clone();
        let fetch_settings = config.fetch_settings.clone();

        let engine = EngineHandle::new(config);
        let runner = Self {
            engine,
            msg_tx: msg_tx.clone(),
            output_dir,
            url_policy,
            fetch_settings,
            llm_handle,
            llm_max_input_bytes,
            prompt_registry,
            llm_metadata_models,
            llm_provider,
            llm_default_provider,
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
                engine_info!(
                    "EnqueueUrl job_id={} url_len={} url={}",
                    job_id,
                    url.len(),
                    url
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
            Effect::ArchiveRequested => {
                engine_info!("Archive requested: enqueue export job");
                self.engine.request_export();
            }
            Effect::ResolvePromptLabInputFromUrl { resolve_id, url } => {
                let msg_tx = self.msg_tx.clone();
                let output_dir = self.output_dir.clone();
                let registry = self.prompt_registry.clone();
                let max_input_bytes = self.llm_max_input_bytes.unwrap_or(100_000);
                let url = url.clone();
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

                thread::spawn(move || {
                    use harvester_core::ModelCatalogSource;
                    use harvester_engine::llm::types::ModelId;

                    engine_info!("[prompt-lab-model] loading model catalog");

                    let (models, source) = if let (Some(provider), Some(provider_kind)) =
                        (provider, default_provider_kind)
                    {
                        // Try remote discovery using a new tokio runtime
                        match tokio::runtime::Runtime::new() {
                            Ok(runtime) => {
                                match runtime.block_on(provider.list_models()) {
                                    Ok(mut model_names) => {
                                        engine_info!(
                                            "[prompt-lab-model] remote discovery succeeded: {} models found",
                                            model_names.len()
                                        );

                                        // Deduplicate and sort
                                        model_names.sort();
                                        model_names.dedup();

                                        // Convert to ModelId with configured provider
                                        let models: Vec<ModelId> = model_names
                                            .into_iter()
                                            .map(|name| ModelId::new(provider_kind, name))
                                            .collect();

                                        (models, ModelCatalogSource::Remote)
                                    }
                                    Err(err) => {
                                        engine_warn!(
                                            "[prompt-lab-model] remote discovery failed: {}; falling back to local",
                                            err
                                        );
                                        // Fallback to local - for now, just empty catalog
                                        // In a full implementation, we'd call local_dispatchable_model_names
                                        (Vec::new(), ModelCatalogSource::LocalFallback)
                                    }
                                }
                            }
                            Err(err) => {
                                engine_warn!(
                                    "[prompt-lab-model] failed to create runtime: {}; falling back to local",
                                    err
                                );
                                (Vec::new(), ModelCatalogSource::LocalFallback)
                            }
                        }
                    } else {
                        engine_warn!("[prompt-lab-model] LLM not configured; empty catalog");
                        (Vec::new(), ModelCatalogSource::LocalFallback)
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
                    "Download linked page job_id={} link_index={} url_len={}",
                    job_id,
                    link_index,
                    url.len()
                );
                let msg_tx = self.msg_tx.clone();
                let output_dir = self.output_dir.clone();
                let url_policy = self.url_policy.clone();
                let fetch_settings = self.fetch_settings.clone();
                thread::spawn(move || {
                    let _ = msg_tx.send(Msg::LinkDownloadStarted { job_id, link_index });
                    match download_link_page(&url, &output_dir, &url_policy, &fetch_settings) {
                        Ok(absolute_path) => {
                            let relative_path = absolute_path
                                .strip_prefix(&output_dir)
                                .map(|p| p.to_path_buf())
                                .unwrap_or_else(|_| absolute_path.clone());
                            let _ = msg_tx.send(Msg::LinkDownloadCompleted {
                                job_id,
                                link_index,
                                path: relative_path,
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
            Effect::SavePromptContextFile {
                prompt_id,
                mut context_pairs,
            } => {
                let contexts_dir = contexts_directory();
                if let Err(err) = fs::create_dir_all(&contexts_dir) {
                    let reason = format!("failed to create contexts directory: {}", err);
                    engine_error!(
                        "[prompt-lab-context] SavePromptContextFile {} prompt_id={:?}",
                        reason,
                        prompt_id
                    );
                    let _ = self
                        .msg_tx
                        .send(Msg::PromptLabContextSaveFailed { prompt_id, reason });
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
                            let _ = self
                                .msg_tx
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
                        let _ = self
                            .msg_tx
                            .send(Msg::PromptLabContextSaveFailed { prompt_id, reason });
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
                    let _ = self
                        .msg_tx
                        .send(Msg::PromptLabContextSaveFailed { prompt_id, reason });
                    return;
                }

                if let Err(err) = fs::rename(&tmp_path, &path) {
                    let reason = format!("failed to rename temp file: {}", err);
                    engine_error!(
                        "[prompt-lab-context] SavePromptContextFile {} prompt_id={:?}",
                        reason,
                        prompt_id
                    );
                    let _ = self
                        .msg_tx
                        .send(Msg::PromptLabContextSaveFailed { prompt_id, reason });
                    return;
                }

                engine_info!(
                    "[prompt-lab-context] Saved context for {:?} to {:?}",
                    prompt_id,
                    path
                );
                let _ = self.msg_tx.send(Msg::PromptLabContextSaved {
                    prompt_id,
                    path: path.display().to_string(),
                    version: meta.version as u64,
                });
            }
            Effect::SavePromptTemplateFile {
                prompt_id,
                system_template,
                user_template,
                description,
                expected_format,
            } => {
                match prompt_template_store::save_template_file(
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
                        if let Ok(mut registry) = self.prompt_registry.write() {
                            registry.register_overlay(overlay);
                        } else {
                            engine_warn!(
                                "[prompt-lab-template] register_overlay lock poisoned prompt_id={:?}",
                                prompt_id
                            );
                        }
                        engine_info!(
                            "[prompt-lab-template] Saved template prompt_id={:?} path={} version={}",
                            prompt_id,
                            path.display(),
                            version
                        );
                        let _ = self.msg_tx.send(Msg::PromptLabTemplateSaved {
                            prompt_id,
                            version,
                            path: path.display().to_string(),
                        });
                    }
                    Err(reason) => {
                        engine_error!(
                            "[prompt-lab-template] SavePromptTemplateFile failed prompt_id={:?} reason={}",
                            prompt_id,
                            reason
                        );
                        let _ = self
                            .msg_tx
                            .send(Msg::PromptLabTemplateSaveFailed { prompt_id, reason });
                    }
                }
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
                let output_dir = self.output_dir.clone();
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
            Effect::LoadArticlesForBriefing { ordered_urls } => {
                let msg_tx = self.msg_tx.clone();
                let output_dir = self.output_dir.clone();
                let max_input_bytes = self.llm_max_input_bytes.unwrap_or(100_000);
                let registry = self.prompt_registry.clone();
                thread::spawn(move || {
                    let guard = registry.read().unwrap();
                    match load_and_prepare_articles_filtered(
                        &output_dir,
                        max_input_bytes,
                        &guard,
                        &ordered_urls,
                    ) {
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
                            engine_info!(
                                "[briefing-loader] prepared {} article(s)",
                                loaded_articles.len()
                            );
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
                let output_dir = self.output_dir.clone();
                let max_input_bytes = self.llm_max_input_bytes.unwrap_or(100_000);
                let registry = self.prompt_registry.clone();
                thread::spawn(move || {
                    let guard = registry.read().unwrap();
                    match load_and_prepare_articles_filtered(
                        &output_dir,
                        max_input_bytes,
                        &guard,
                        &ordered_urls,
                    ) {
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
            Effect::LoadPromptContexts => {
                let msg_tx = self.msg_tx.clone();
                thread::spawn(move || {
                    use harvester_engine::llm::{load_context_file, PromptId};
                    use std::collections::HashMap;
                    use std::path::PathBuf;

                    // Load contexts from a default directory
                    // TODO: Make this configurable
                    let contexts_dir = PathBuf::from("contexts");

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
                            engine_info!(
                                "[PromptContext] Context file not found: {:?}, skipping",
                                path
                            );
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
                                // Continue with other contexts
                            }
                        }
                    }

                    let _ = msg_tx.send(Msg::PromptContextsLoaded { contexts });
                });
            }
            Effect::LoadPromptTemplateFiles => {
                let prompts_dir = prompt_template_store::prompts_directory();
                for entry in prompt_template_store::load_prompt_template_files(&prompts_dir) {
                    match entry {
                        Ok((prompt_id, template_file, path)) => {
                            if template_file.version == PROMPT_VERSION_DRAFT {
                                engine_warn!(
                                    "[prompt-lab-template] skipping draft saved template prompt_id={:?} path={}",
                                    prompt_id,
                                    path.display()
                                );
                                continue;
                            }
                            let overlay = PromptTemplateOwned {
                                id: prompt_id,
                                version: template_file.version,
                                system_template: template_file.system_template,
                                user_template: template_file.user_template,
                                description: template_file.description,
                                expected_format: template_file.expected_format,
                            };
                            if let Ok(mut guard) = self.prompt_registry.write() {
                                guard.register_overlay(overlay);
                            } else {
                                engine_warn!(
                                    "[prompt-lab-template] register_overlay lock poisoned prompt_id={:?}",
                                    prompt_id
                                );
                            }
                            engine_info!(
                                "[prompt-lab-template] Loaded saved template prompt_id={:?} version={} path={}",
                                prompt_id,
                                template_file.version,
                                path.display()
                            );
                        }
                        Err(reason) => {
                            engine_warn!(
                                "[prompt-lab-template] Failed to load saved template: {}",
                                reason
                            );
                        }
                    }
                }
            }
            Effect::LoadLlmMetadata => {
                let msg_tx = self.msg_tx.clone();
                let registry = self.prompt_registry.clone();
                let models = self.llm_metadata_models.clone();
                thread::spawn(move || {
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
                use super::summary_cache_store::{default_summary_cache_path, save_summary_cache};

                let msg_tx = self.msg_tx.clone();
                thread::spawn(move || {
                    let path = default_summary_cache_path();
                    match save_summary_cache(&cache, &path) {
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
                    // No message needed on completion - fire and forget
                    let _ = msg_tx;
                });
            }
            Effect::PersistTriageCache { cache } => {
                use super::triage_cache_store::{default_triage_cache_path, save_triage_cache};

                let msg_tx = self.msg_tx.clone();
                thread::spawn(move || {
                    let path = default_triage_cache_path();
                    match save_triage_cache(&cache, &path) {
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
                    let _ = msg_tx;
                });
            }
            Effect::LoadArticlesForTriage { ordered_urls } => {
                let msg_tx = self.msg_tx.clone();
                let output_dir = self.output_dir.clone();
                let max_input_bytes = self.llm_max_input_bytes.unwrap_or(100_000);
                let registry = self.prompt_registry.clone();
                thread::spawn(move || {
                    let guard = registry.read().unwrap();
                    match load_and_prepare_articles_filtered(
                        &output_dir,
                        max_input_bytes,
                        &guard,
                        &ordered_urls,
                    ) {
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
                            engine_info!("[triage-loader] prepared {} article(s)", articles.len());
                            let _ = msg_tx.send(Msg::TriageArticlesLoaded { articles });
                        }
                        Err(reason) => {
                            engine_warn!("[triage-loader] failed: {}", reason);
                            let _ = msg_tx.send(Msg::TriageArticlesLoadFailed { reason });
                        }
                    }
                });
            }
            Effect::PollAllSources => {
                let msg_tx = self.msg_tx.clone();
                let output_dir = self.output_dir.clone();
                let url_policy = self.url_policy.clone();
                let fetch_settings = self.fetch_settings.clone();
                thread::spawn(move || {
                    let _guard = PollGuard::new(msg_tx.clone());
                    let config_path = source_loader::default_source_config_path();
                    engine_info!("[source-config] loading {}", config_path.display());
                    let registry = source_loader::load_source_registry(&config_path);
                    let config_dir = config_path
                        .parent()
                        .unwrap_or_else(|| Path::new("."))
                        .to_path_buf();
                    let allowed_dirs = vec![config_dir.clone(), output_dir.clone()];
                    let seen_set_path = seen_set_store::default_seen_set_path();
                    let mut seen_set = seen_set_store::load_seen_set(&seen_set_path);
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
                                    let _ = msg_tx.send(Msg::SourcePollCompleted {
                                        source_id: source_id.clone(),
                                        urls: result.urls,
                                    });
                                }
                                Err(err) => {
                                    engine_warn!("[source-poll] {} failed: {}", source_id, err);
                                    let _ = msg_tx.send(Msg::SourcePollFailed {
                                        source_id: source_id.clone(),
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
                                let _ = msg_tx.send(Msg::SourcePollCompleted {
                                    source_id: source_id.clone(),
                                    urls: result.urls,
                                });
                            }
                            SourceType::Script { .. } => {
                                engine_warn!(
                                    "[source-poll] scripts not supported yet for {}",
                                    source_id
                                );
                                let _ = msg_tx.send(Msg::SourcePollFailed {
                                    source_id: source_id.clone(),
                                    error: "script sources not implemented".to_string(),
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
            Effect::OpenUrlInBrowser { url } => {
                use std::ffi::OsStr;
                use std::os::windows::ffi::OsStrExt;
                use windows::core::PCWSTR;
                use windows::Win32::UI::Shell::ShellExecuteW;
                use windows::Win32::UI::WindowsAndMessaging::SW_SHOWNORMAL;

                engine_info!("[browser] Opening URL: {}", url);

                let operation: Vec<u16> = OsStr::new("open").encode_wide().chain(Some(0)).collect();
                let url_wide: Vec<u16> = OsStr::new(&url).encode_wide().chain(Some(0)).collect();

                let result = unsafe {
                    ShellExecuteW(
                        None,
                        PCWSTR(operation.as_ptr()),
                        PCWSTR(url_wide.as_ptr()),
                        None,
                        None,
                        SW_SHOWNORMAL,
                    )
                };

                if result.0 as isize <= 32 {
                    engine_warn!(
                        "[browser] ShellExecuteW failed for URL '{}', error code: {}",
                        url,
                        result.0 as isize
                    );
                }
            }
        }
    }

    fn spawn_event_loop(&self, msg_tx: mpsc::Sender<Msg>) {
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

        if let Some(handle) = self.llm_handle.clone() {
            let llm_tx = msg_tx.clone();
            let receiver = handle.event_receiver();
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
}

impl Drop for EffectRunner {
    fn drop(&mut self) {
        if let Some(handle) = self.llm_handle.take() {
            engine_info!("[llm-shutdown] stopping llm worker");
            handle.drain_and_stop();
        }
    }
}

impl EffectRunner {
    fn validate_effect(&self, effect: &Effect) -> Result<(), String> {
        match effect {
            Effect::EnqueueUrl { url, .. } | Effect::DownloadLinkedPage { url, .. } => {
                let parsed = reqwest::Url::parse(url)
                    .map_err(|err| format!("url parsing failed for {}: {}", url, err))?;
                if let Err(violation) = self.url_policy.check(&parsed) {
                    Err(format!(
                        "url policy violation for url '{}': {}",
                        url, violation
                    ))
                } else {
                    Ok(())
                }
            }
            Effect::DeleteLinkedPage { path, .. } => {
                if is_confined_to(path, &self.output_dir) {
                    Ok(())
                } else {
                    Err(format!(
                        "path policy violation for linked delete path '{}'",
                        path.display()
                    ))
                }
            }
            Effect::RequestLlmCompletion { input_content, .. } => {
                if let Some(limit) = self.llm_max_input_bytes {
                    if input_content.len() > limit {
                        return Err(format!(
                            "LLM input too large ({} > {} characters)",
                            input_content.len(),
                            limit
                        ));
                    }
                }
                Ok(())
            }
            _ => Ok(()),
        }
    }

    fn reject_effect(&self, effect: Effect, reason: String) {
        match effect {
            Effect::EnqueueUrl { job_id, .. } => {
                if reason.starts_with("url parsing failed") {
                    engine_info!("EnqueueUrl rejected job_id={} reason={}", job_id, reason);
                } else {
                    engine_warn!("EnqueueUrl rejected job_id={} reason={}", job_id, reason);
                }
                if let Err(err) = self.msg_tx.send(Msg::JobDone {
                    job_id,
                    result: JobResultKind::Failed {
                        reason: reason.clone(),
                    },
                    content_preview: None,
                    extracted_links: Vec::new(),
                }) {
                    engine_warn!(
                        "Failed to notify job failure for job_id={}: {}",
                        job_id,
                        err
                    );
                }
            }
            Effect::DownloadLinkedPage {
                job_id, link_index, ..
            } => {
                engine_warn!(
                    "DownloadLinkedPage rejected job_id={} link_index={} reason={}",
                    job_id,
                    link_index,
                    reason
                );
                if let Err(err) = self.msg_tx.send(Msg::LinkDownloadFailed {
                    job_id,
                    link_index,
                    error: reason,
                }) {
                    engine_warn!(
                        "Failed to report download failure for job_id={} link_index={}: {}",
                        job_id,
                        link_index,
                        err
                    );
                }
            }
            Effect::DeleteLinkedPage {
                job_id,
                link_index,
                path,
            } => {
                engine_warn!(
                    "DeleteLinkedPage rejected job_id={} link_index={} path={} reason={}",
                    job_id,
                    link_index,
                    path.display(),
                    reason
                );
                let _ = self.msg_tx.send(Msg::LinkDeleted { job_id, link_index });
            }
            other => {
                engine_warn!("Effect rejected but no handler for {:?}: {}", other, reason);
            }
        }
    }
}

fn map_llm_event(event: LlmEvent) -> Msg {
    match event {
        LlmEvent::Completed { request_id, result } => {
            let (result_kind, metadata) = match result {
                Ok(outcome) => {
                    let kind = LlmResultKind::Success {
                        output_json: outcome.output_json,
                        input_tokens: outcome.metadata.input_tokens,
                        output_tokens: outcome.metadata.output_tokens,
                        prompt_version: outcome.metadata.prompt_version,
                        model_id: outcome.metadata.resolved_model.clone(),
                    };
                    (kind, Some(outcome.metadata))
                }
                Err(LlmCompletionError::ValidationFailed {
                    reason,
                    raw_response,
                    failure_metadata,
                }) => (
                    LlmResultKind::ValidationFailed {
                        reason,
                        raw_response,
                    },
                    failure_metadata.map(LlmRunMetadata::from),
                ),
                Err(LlmCompletionError::QuotaExhausted {
                    description,
                    failure_metadata,
                }) => (
                    LlmResultKind::QuotaExhausted {
                        reason: description,
                    },
                    failure_metadata.map(LlmRunMetadata::from),
                ),
                Err(LlmCompletionError::PersistenceFailed {
                    detail,
                    failure_metadata,
                }) => (
                    LlmResultKind::Failed {
                        reason: format!("replay persistence failed: {}", detail),
                    },
                    failure_metadata.map(LlmRunMetadata::from),
                ),
                Err(error) => (
                    LlmResultKind::Failed {
                        reason: llm_error_reason(error),
                    },
                    None,
                ),
            };
            Msg::LlmCompleted {
                request_id,
                result: result_kind,
                metadata,
            }
        }
    }
}

struct PollGuard {
    msg_tx: mpsc::Sender<Msg>,
}

impl PollGuard {
    fn new(msg_tx: mpsc::Sender<Msg>) -> Self {
        Self { msg_tx }
    }
}

impl Drop for PollGuard {
    fn drop(&mut self) {
        let _ = self.msg_tx.send(Msg::AllSourcesPollEnded);
    }
}

#[derive(Debug)]
struct RedirectPolicyError(String);

impl fmt::Display for RedirectPolicyError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl StdError for RedirectPolicyError {}

fn fetch_feed(
    feed_url: &str,
    url_policy: &UrlPolicy,
    fetch_settings: &FetchSettings,
) -> Result<Vec<u8>, String> {
    let parsed =
        Url::parse(feed_url).map_err(|err| format!("invalid feed url {}: {}", feed_url, err))?;
    if let Err(violation) = url_policy.check(&parsed) {
        return Err(format!(
            "feed url {} violates policy: {}",
            feed_url, violation
        ));
    }

    let redirect_limit = fetch_settings.redirect_limit;
    let policy = Policy::custom({
        let url_policy = url_policy.clone();
        move |attempt| {
            let count = attempt.previous().len();
            if count >= redirect_limit {
                attempt.error("redirect limit exceeded")
            } else {
                let target_url = attempt.url().clone();
                if let Err(violation) = url_policy.check(&target_url) {
                    attempt.error(RedirectPolicyError(format!(
                        "redirect target {} violated policy: {}",
                        target_url, violation
                    )))
                } else {
                    attempt.follow()
                }
            }
        }
    });

    let client = Client::builder()
        .connect_timeout(fetch_settings.connect_timeout)
        .timeout(fetch_settings.request_timeout)
        .redirect(policy)
        .user_agent(fetch_settings.user_agent.clone())
        .build()
        .map_err(|err| err.to_string())?;

    let mut response = client
        .get(parsed.clone())
        .header(ACCEPT, FEED_ACCEPT_HEADER)
        .send()
        .map_err(|err| err.to_string())?;

    if !response.status().is_success() {
        return Err(format!("HTTP error {} for {}", response.status(), feed_url));
    }

    if let Some(len) = response.content_length() {
        if len > MAX_FEED_RESPONSE_BYTES as u64 {
            return Err(format!(
                "feed {} too large: {} bytes",
                feed_url, MAX_FEED_RESPONSE_BYTES
            ));
        }
    }

    let mut buffer = Vec::new();
    let mut chunk = [0u8; 8192];
    let mut total = 0;
    loop {
        let read = response.read(&mut chunk).map_err(|err| err.to_string())?;
        if read == 0 {
            break;
        }
        total += read;
        if total > MAX_FEED_RESPONSE_BYTES {
            return Err(format!(
                "feed {} exceeded {} bytes",
                feed_url, MAX_FEED_RESPONSE_BYTES
            ));
        }
        buffer.extend_from_slice(&chunk[..read]);
    }

    Ok(buffer)
}

fn llm_error_reason(error: LlmCompletionError) -> String {
    match error {
        LlmCompletionError::ProviderError(err) => err.to_string(),
        LlmCompletionError::QuotaExhausted { description, .. } => description,
        LlmCompletionError::PromptNotFound { prompt_id } => {
            format!("prompt {:?} not found", prompt_id)
        }
        LlmCompletionError::PersistenceFailed { detail, .. } => {
            format!("replay persistence failed: {}", detail)
        }
        LlmCompletionError::InputTooLarge { size, limit } => {
            format!("input too large ({} > {})", size, limit)
        }
        LlmCompletionError::TemplateRenderFailed { detail } => {
            format!("template rendering failed: {}", detail)
        }
        LlmCompletionError::UnsupportedModel { model, reason } => {
            format!(
                "unsupported model {:?}/{}: {}",
                model.provider(),
                model.model_name(),
                reason
            )
        }
        LlmCompletionError::ValidationFailed { .. } => unreachable!(),
    }
}

fn download_link_page(
    url: &str,
    output_dir: &Path,
    url_policy: &UrlPolicy,
    fetch_settings: &FetchSettings,
) -> Result<PathBuf, String> {
    let linked_dir = output_dir.join("linked");
    ensure_output_dir(&linked_dir).map_err(|err| format!("linked output dir error: {}", err))?;

    let parsed = reqwest::Url::parse(url)
        .map_err(|err| format!("url parsing failed for {}: {}", url, err))?;
    if let Err(violation) = url_policy.check(&parsed) {
        return Err(format!(
            "url policy violation for linked page: {}",
            violation
        ));
    }

    let redirect_limit = fetch_settings.redirect_limit;
    let redirect_counter = Arc::new(AtomicUsize::new(0));
    let policy = Policy::custom({
        let counter = redirect_counter.clone();
        move |attempt| {
            let count = attempt.previous().len();
            counter.store(count, Ordering::Relaxed);
            if count >= redirect_limit {
                attempt.error("redirect limit exceeded")
            } else {
                attempt.follow()
            }
        }
    });

    let client = Client::builder()
        .connect_timeout(fetch_settings.connect_timeout)
        .timeout(fetch_settings.request_timeout)
        .redirect(policy)
        .user_agent(fetch_settings.user_agent.clone())
        .build()
        .map_err(|err| err.to_string())?;
    let mut response = client
        .get(parsed.clone())
        .send()
        .map_err(|err| err.to_string())?;
    if !response.status().is_success() {
        return Err(format!(
            "HTTP error {} for linked page {}",
            response.status(),
            url
        ));
    }

    let content_type = response
        .headers()
        .get(CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .map(|value| value.to_string());
    if let Some(ref content_type) = content_type {
        let ct = content_type
            .split(';')
            .next()
            .unwrap_or(content_type)
            .trim();
        if !fetch_settings
            .allowed_content_types
            .iter()
            .any(|allowed| allowed.eq_ignore_ascii_case(ct))
        {
            return Err(format!("unsupported content type '{}'", ct));
        }
    }

    let final_url = response.url().to_string();
    let mut bytes = Vec::new();
    let mut buffer = [0u8; 8192];
    loop {
        let read = response.read(&mut buffer).map_err(|err| err.to_string())?;
        if read == 0 {
            break;
        }
        let next_len = bytes.len() + read;
        if next_len as u64 > fetch_settings.max_bytes {
            return Err(format!(
                "response too large for linked page {} (limit {} bytes)",
                url, fetch_settings.max_bytes
            ));
        }
        bytes.extend_from_slice(&buffer[..read]);
    }

    let decoded =
        decode_html(&bytes, content_type.as_deref()).map_err(|err: DecodeError| err.to_string())?;

    let extractor = ReadabilityLikeExtractor;
    let extracted = extractor.extract(&decoded.html);
    let converter = LinkExtractingConverter::new();
    let conversion = converter.to_markdown(&extracted.content_html, Some(final_url.as_str()));
    let token_counter = WhitespaceTokenCounter;
    let fetched_utc = Utc::now().to_rfc3339();
    let (_tokens, doc) = build_markdown_document(
        &final_url,
        extracted.title.as_deref(),
        &decoded.encoding_label,
        &fetched_utc,
        &conversion.markdown,
        &token_counter,
    );

    let filename = deterministic_filename(extracted.title.as_deref(), &final_url);
    let writer = AtomicFileWriter::new(linked_dir);
    writer.write(&filename, &doc).map_err(|err| err.to_string())
}

fn map_stage(stage: harvester_engine::Stage) -> Stage {
    match stage {
        harvester_engine::Stage::Queued => Stage::Queued,
        harvester_engine::Stage::Downloading => Stage::Downloading,
        harvester_engine::Stage::Sanitizing => Stage::Sanitizing,
        harvester_engine::Stage::Converting => Stage::Converting,
        harvester_engine::Stage::Tokenizing => Stage::Tokenizing,
        harvester_engine::Stage::Writing => Stage::Writing,
        harvester_engine::Stage::Done => Stage::Done,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use harvester_engine::llm::load_context_file;
    use std::fs;
    use std::sync::mpsc;
    use std::time::Duration;
    use tempfile::tempdir;

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

    fn runner_with_receiver() -> (EffectRunner, mpsc::Receiver<Msg>) {
        let (tx, rx) = mpsc::channel();
        (EffectRunner::new(tx), rx)
    }

    fn write_markdown(dir: &Path, filename: &str, url: &str) {
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

    fn with_temp_working_dir<T>(callback: impl FnOnce(&Path) -> T) -> T {
        let original = std::env::current_dir().expect("current dir");
        let temp = tempdir().expect("tempdir");
        std::env::set_current_dir(temp.path()).expect("set cwd");
        let result = callback(temp.path());
        std::env::set_current_dir(original).expect("restore cwd");
        result
    }

    #[test]
    fn enqueue_url_effect_is_rejected_by_url_policy() {
        let (runner, rx) = runner_with_receiver();
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
        let (runner, rx) = runner_with_receiver();
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
        let (runner, rx) = runner_with_receiver();
        let job_id = 11;
        let link_index = 3;
        runner.enqueue(vec![Effect::DeleteLinkedPage {
            job_id,
            link_index,
            path: PathBuf::from("../outside.md"),
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
        with_temp_working_dir(|_| {
            let (runner, rx) = runner_with_receiver();
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
                    let saved = load_context_file(&PathBuf::from(&path)).expect("load saved");
                    assert_eq!(saved.variables.get("foo").map(String::as_str), Some("bar"));
                }
                other => panic!("unexpected message: {:?}", other),
            }
        });
    }

    #[test]
    fn save_prompt_context_file_reports_failure_when_existing_file_invalid() {
        with_temp_working_dir(|_| {
            fs::create_dir_all("contexts").expect("create contexts dir");
            fs::write("contexts/article_triage.toml", "bad toml").expect("write invalid file");

            let (runner, rx) = runner_with_receiver();
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
                    assert!(reason.contains("failed to parse"));
                }
                other => panic!("unexpected message: {:?}", other),
            }
        });
    }

    #[test]
    fn save_prompt_template_file_writes_file_and_dispatches_saved_msg() {
        with_temp_working_dir(|_| {
            let (runner, rx) = runner_with_receiver();
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
                    let file =
                        fs::read_to_string(PathBuf::from(&path)).expect("read saved template file");
                    assert!(file.contains("system {{context}}"));
                    assert!(file.contains("user {{context}}"));
                }
                other => panic!("unexpected message: {:?}", other),
            }
        });
    }

    #[test]
    fn load_articles_for_briefing_prereq_dispatches_loaded_message() {
        let temp = tempdir().expect("tempdir");
        write_markdown(temp.path(), "a.md", "https://example.com/a");
        let (mut runner, rx) = runner_with_receiver();
        runner.output_dir = temp.path().to_path_buf();
        runner.enqueue(vec![Effect::LoadArticlesForBriefingPrereq {
            ordered_urls: vec!["https://example.com/a".to_string()],
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
    fn load_articles_for_briefing_with_empty_ordered_urls_dispatches_empty_articles_loaded() {
        let temp = tempdir().expect("tempdir");
        write_markdown(temp.path(), "a.md", "https://example.com/a");
        let (mut runner, rx) = runner_with_receiver();
        runner.output_dir = temp.path().to_path_buf();
        runner.enqueue(vec![Effect::LoadArticlesForBriefing {
            ordered_urls: Vec::new(),
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

    // --- Step 3: map_llm_event failure metadata propagation tests ---

    fn make_failure_metadata() -> harvester_engine::llm::LlmFailureMetadata {
        use harvester_engine::llm::prompt::PromptId;
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
        let (mut runner, rx) = runner_with_receiver();
        runner.output_dir = temp.path().to_path_buf();
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
        let (mut runner, rx) = runner_with_receiver();
        runner.output_dir = temp.path().to_path_buf();
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
}
