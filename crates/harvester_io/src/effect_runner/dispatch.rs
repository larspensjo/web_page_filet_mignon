use std::collections::HashMap;
use std::fs;
use std::sync::Arc;
use std::thread;
use std::time::Instant;

use chrono::Utc;
use engine_logging::{engine_error, engine_info, engine_warn};
use harvester_core::{Effect, LlmResultKind, LoadedArticle, Msg, StopPolicy};
use harvester_engine::llm::load_context_file;
use harvester_engine::llm::prompt::{PromptId, PromptTemplateOwned, PROMPT_VERSION_DRAFT};
use harvester_engine::llm::prompt_context::{ContextMeta, PromptContextFile};
use harvester_engine::llm::LlmCommand;
use harvester_engine::{
    build_triage_archive, import_saved_webpages, is_confined_to,
    load_and_prepare_articles_filtered, scan_archive_article_metadata, ExportOptions,
    ImportOptions,
};

use crate::effect_helpers::{
    build_local_model_catalog, download_link_page, prompt_context_filename,
};

use super::worker::{run_triage_refresh_load, EntityIndexWorkerMsg};
use super::{truncate_url_for_log, EffectRunner};

impl EffectRunner {
    pub(super) fn execute_effect(&self, effect: Effect) {
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
            Effect::OpenArchiveDialog {
                request_id,
                article_count,
                since_utc,
                default_basename,
                pending_pre_triage_count,
                token_estimates,
            } => {
                let msg_tx = self.msg_tx.clone();
                let output_dir = self.paths.output_dir.clone();
                thread::spawn(move || {
                    let default_file_exists = output_dir.join(&default_basename).exists();
                    engine_info!(
                        "[archive-dialog] open requested request_id={} article_count={} default_basename={} default_file_exists={}",
                        request_id,
                        article_count,
                        default_basename,
                        default_file_exists
                    );
                    let _ = msg_tx.send(Msg::ArchiveDialogReady {
                        request_id,
                        article_count,
                        since_utc,
                        default_basename,
                        default_file_exists,
                        export_dir: output_dir,
                        pending_pre_triage_count,
                        token_estimates,
                    });
                });
            }
            Effect::ArchiveRequested {
                request_id,
                basename,
                ordered_urls,
                since_utc,
                requested_checkpoint,
                use_summaries,
                summaries,
            } => {
                let msg_tx = self.msg_tx.clone();
                let output_dir = self.paths.output_dir.clone();
                thread::spawn(move || {
                    let options = ExportOptions {
                        output_filename: basename.clone(),
                        manifest_filename: None,
                        ..ExportOptions::default()
                    };
                    match build_triage_archive(
                        &output_dir,
                        &basename,
                        &ordered_urls,
                        since_utc,
                        options,
                        use_summaries,
                        &summaries,
                    ) {
                        Ok(summary) => {
                            engine_info!(
                                "[archive-dialog] export completed request_id={} docs={} path={}",
                                request_id,
                                summary.doc_count,
                                summary.output_path.display()
                            );
                            let _ = msg_tx.send(Msg::ArchiveExportCompleted {
                                request_id,
                                path: summary.output_path,
                                doc_count: summary.doc_count,
                                requested_checkpoint,
                            });
                        }
                        Err(err) => {
                            engine_warn!(
                                "[archive-dialog] export failed request_id={} basename={} reason={}",
                                request_id,
                                basename,
                                err
                            );
                            let _ = msg_tx.send(Msg::ArchiveExportFailed {
                                request_id,
                                basename,
                                reason: err.to_string(),
                            });
                        }
                    }
                });
            }
            Effect::ShowArchiveDialog { .. } => {
                engine_warn!(
                    "[archive-dialog] ShowArchiveDialog reached effect runner unexpectedly"
                );
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
                        let reason = format!(
                            "required prompt contexts directory not found at {:?}",
                            contexts_dir
                        );
                        engine_warn!("[PromptContext] {}", reason);
                        let _ = msg_tx.send(Msg::PromptContextsLoadFailed { reason });
                        return;
                    }

                    let mut contexts = HashMap::new();
                    let mut required_context_failure = None;
                    let prompt_ids = [
                        PromptId::ArticleTriage,
                        PromptId::ArticleSummary,
                        PromptId::ArticleSignalCandidate,
                        PromptId::AggregateBriefing,
                    ];

                    for prompt_id in prompt_ids {
                        let filename = prompt_context_filename(prompt_id);
                        let path = contexts_dir.join(filename);

                        if !path.exists() {
                            if prompt_id == PromptId::ArticleTriage {
                                required_context_failure = Some(format!(
                                    "required ArticleTriage context file missing at {:?}",
                                    path
                                ));
                            }
                            continue;
                        }

                        match load_context_file(&path) {
                            Ok(ctx_file) => {
                                let vec: Vec<(String, String)> =
                                    ctx_file.variables.into_iter().collect();
                                contexts.insert(prompt_id, vec);
                            }
                            Err(e) => {
                                if prompt_id == PromptId::ArticleTriage {
                                    required_context_failure = Some(format!(
                                        "required ArticleTriage context failed to load from {:?}: {}",
                                        path, e
                                    ));
                                    continue;
                                }
                                engine_warn!("[PromptContext] Failed to load {:?}: {}", path, e);
                            }
                        }
                    }

                    if let Some(reason) = required_context_failure {
                        if !contexts.is_empty() {
                            let _ = msg_tx.send(Msg::PromptContextsLoaded { contexts });
                        }
                        engine_warn!("[PromptContext] {}", reason);
                        let _ = msg_tx.send(Msg::PromptContextsLoadFailed { reason });
                    } else {
                        let _ = msg_tx.send(Msg::PromptContextsLoaded { contexts });
                    }
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
                            PromptId::ArticleSignalCandidate,
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
            Effect::PersistSignalCandidateCache { cache } => {
                let msg_tx = self.msg_tx.clone();
                let path = self.paths.output_dir.join(".signal_candidate_cache.ron");
                thread::spawn(move || {
                    match crate::signal_candidate_cache_store::save(&path, &cache) {
                        Ok(_) => {
                            engine_info!("[signal-cache] Persisted cache to {:?}", path);
                        }
                        Err(err) => {
                            engine_warn!(
                                "[signal-cache] Failed to persist cache to {:?}: {}",
                                path,
                                err
                            );
                        }
                    }
                    let _ = msg_tx;
                });
            }
            Effect::PersistSignalCandidateOverrides { overrides } => {
                let msg_tx = self.msg_tx.clone();
                let path = self
                    .paths
                    .output_dir
                    .join(".signal_candidate_overrides.ron");
                thread::spawn(move || {
                    match crate::signal_candidate_overrides_store::save(&path, &overrides) {
                        Ok(_) => {
                            engine_info!("[signal-overrides] Persisted overrides to {:?}", path);
                        }
                        Err(err) => {
                            engine_warn!(
                                "[signal-overrides] Failed to persist overrides to {:?}: {}",
                                path,
                                err
                            );
                        }
                    }
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
            Effect::SaveBriefingCheckpoint { save_id, since_utc } => {
                let msg_tx = self.msg_tx.clone();
                let path = self.paths.briefing_checkpoint_path.clone();
                thread::spawn(move || {
                    let s = since_utc.map(|dt| dt.to_rfc3339());
                    match crate::save_briefing_checkpoint(&path, s.as_deref()) {
                        Ok(()) => {
                            let _ = msg_tx.send(Msg::BriefingCheckpointSaveSucceeded { save_id });
                        }
                        Err(e) => {
                            engine_error!(
                                "[briefing-checkpoint] save failed save_id={}: {}",
                                save_id,
                                e
                            );
                            let _ = msg_tx
                                .send(Msg::BriefingCheckpointSaveFailed { save_id, reason: e });
                        }
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

            // --- Window size persistence ---
            Effect::PersistWindowSize { width, height } => {
                let path = self.paths.state_path.clone();
                thread::spawn(move || {
                    crate::persist_window_size(&path, width, height);
                    engine_info!("[window-size] Persisted {}x{} to {:?}", width, height, path);
                });
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
}
