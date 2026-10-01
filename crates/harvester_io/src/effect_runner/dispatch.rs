#[cfg(test)]
use std::collections::HashMap;
use std::sync::atomic::Ordering;
use std::sync::Arc;
use std::thread;
use std::time::Instant;

use engine_logging::{engine_error, engine_info, engine_warn};
use harvester_core::{Effect, LlmResultKind, Msg, StopPolicy};
#[cfg(test)]
use harvester_engine::llm::prompt::PromptId;
#[cfg(test)]
use harvester_engine::llm::prompt_context::ContextMeta;
use harvester_engine::llm::prompt_context::PromptContextFile;
use harvester_engine::llm::LlmCommand;
use harvester_engine::{build_triage_archive, import_saved_webpages, ImportOptions};

use super::worker::run_triage_refresh_load;
use super::{truncate_url_for_log, EffectRunner};

pub(crate) fn ordered_context_pairs(ctx_file: &PromptContextFile) -> Vec<(String, String)> {
    let mut pairs: Vec<(String, String)> = ctx_file
        .variables
        .iter()
        .map(|(key, value)| (key.clone(), value.clone()))
        .collect();
    pairs.sort_by(|(left, _), (right, _)| left.cmp(right));
    pairs
}

impl EffectRunner {
    pub(super) fn execute_effect(&self, effect: Effect) {
        match effect {
            Effect::SaveResults { records } => self.result_sink.enqueue(records),
            Effect::FlushResults => {
                if let Err(error) = self.flush_results() {
                    engine_error!("[results] flush failed: {}", error);
                }
            }
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
                self.engine.resume();
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
                signal_candidate_default,
                signal_candidate_count,
                signal_candidate_scoring_done,
                signal_candidate_scoring_total,
                signal_candidate_token_estimates,
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
                        signal_candidate_default,
                        signal_candidate_count,
                        signal_candidate_scoring_done,
                        signal_candidate_scoring_total,
                        signal_candidate_token_estimates,
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
                annotations,
                priority_snapshot,
            } => {
                let msg_tx = self.msg_tx.clone();
                let output_dir = self.paths.output_dir.clone();
                thread::spawn(move || {
                    match build_triage_archive(
                        &output_dir,
                        &basename,
                        &ordered_urls,
                        since_utc,
                        use_summaries,
                        &summaries,
                        &annotations,
                        &priority_snapshot,
                    ) {
                        Ok(summary) => {
                            if let (Some(window_count), Some(counts)) =
                                (summary.window_count, summary.unexported_by_priority)
                            {
                                let [priority_5, priority_4, priority_3, priority_2, priority_1, unavailable] =
                                    counts;
                                engine_info!(
                                    "[archive-dialog] export completed request_id={} docs={} window_count={} unexported_by_priority={{\"5\":{},\"4\":{},\"3\":{},\"2\":{},\"1\":{},\"unavailable\":{}}} path={}",
                                    request_id,
                                    summary.doc_count,
                                    window_count,
                                    priority_5,
                                    priority_4,
                                    priority_3,
                                    priority_2,
                                    priority_1,
                                    unavailable,
                                    summary.output_path.display()
                                );
                            } else {
                                engine_info!(
                                    "[archive-dialog] export completed request_id={} docs={} path={}",
                                    request_id,
                                    summary.doc_count,
                                    summary.output_path.display()
                                );
                            }
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
            Effect::RequestLlmCompletion {
                request_id,
                prompt_id,
                prompt_version,
                input_content,
                context,
                extra_template_vars,
            } => {
                if let Some(handle) = &self.llm_handle {
                    let cmd = LlmCommand::Complete(Box::new(
                        harvester_engine::llm::LlmCompletionCommand {
                            request_id,
                            prompt_id,
                            prompt_version,
                            input_content,
                            context,
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

            Effect::LoadProcessingConfiguration {
                request_id,
                require_triage_context,
            } => {
                let msg_tx = self.msg_tx.clone();
                let paths = self.paths.clone();
                let registry = self.prompt_registry.clone();
                let effective_models = self.llm_metadata_models.clone();
                let max_input_bytes = self.llm_max_input_bytes.unwrap_or(100_000);
                thread::spawn(move || {
                    engine_info!(
                        "[processing-configuration] request_id={} start require_triage_context={}",
                        request_id,
                        require_triage_context
                    );
                    let result = super::configuration::load(
                        &paths,
                        &registry,
                        max_input_bytes,
                        require_triage_context,
                    );
                    let msg = match result {
                        Ok((contexts, active_versions, preparation_budget)) => {
                            Msg::ProcessingConfigurationLoaded {
                                request_id,
                                contexts,
                                active_versions,
                                effective_models,
                                preparation_budget,
                            }
                        }
                        Err(reason) => {
                            engine_warn!(
                                "[processing-configuration] request_id={} failed: {}",
                                request_id,
                                reason
                            );
                            Msg::ProcessingConfigurationFailed { request_id, reason }
                        }
                    };
                    let _ = msg_tx.send(msg);
                });
            }
            Effect::ResetCorpusScanIndex => {
                self.corpus_scan_reset_requested
                    .store(true, Ordering::Release);
                engine_info!("[corpus-index] reset requested after imported-corpus clear");
            }
            Effect::LoadArticlesForTriage {
                held,
                request_id,
                ordered_urls,
                since_utc,
            } => {
                let msg_tx = self.msg_tx.clone();
                let output_dir = self.paths.output_dir.clone();
                let registry = Arc::clone(&self.prompt_registry);
                let max_input_bytes = self.llm_max_input_bytes.unwrap_or(100_000);
                let index = self.corpus_scan_index.clone();
                let reset_requested = self.corpus_scan_reset_requested.clone();
                thread::spawn(move || {
                    run_triage_refresh_load(
                        index,
                        reset_requested,
                        held,
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
                    let (contexts, failure) =
                        super::configuration::load_contexts(&contexts_dir, true);
                    if let Some(reason) = failure {
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
                let msg_tx = self.msg_tx.clone();
                let prompts_dir = self.paths.prompts_dir.clone();
                let registry = self.prompt_registry.clone();
                thread::spawn(move || {
                    super::configuration::load_overlays(&prompts_dir, &registry);
                    let _ = msg_tx.send(Msg::PromptTemplateFilesLoaded);
                });
            }
            Effect::LoadLlmMetadata => {
                let msg_tx = self.msg_tx.clone();
                let registry = self.prompt_registry.clone();
                let models = self.llm_metadata_models.clone();
                thread::spawn(move || {
                    let active_versions = {
                        let guard = registry.read().unwrap();
                        guard.active_versions_map()
                    };
                    let effective_models = models;

                    engine_info!(
                        "[llm-metadata] metadata prepared (versions={}, models={})",
                        active_versions.len(),
                        effective_models.len(),
                    );

                    let _ = msg_tx.send(Msg::LlmMetadataLoaded {
                        active_versions,
                        effective_models,
                    });
                });
            }

            Effect::PersistSignalCandidateOverrides { overrides } => {
                let msg_tx = self.msg_tx.clone();
                let path = self
                    .paths
                    .output_dir
                    .join(".signal_candidate_overrides.ron");
                let observer = self.file_write_observer.clone();
                thread::spawn(move || {
                    let started = Instant::now();
                    match crate::signal_candidate_overrides_store::save(&path, &overrides) {
                        Ok(_) => {
                            super::observe_file_write(&observer, &path, started.elapsed());
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

            Effect::PollAllSources => {
                self.execute_poll_all_sources();
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
            Effect::PersistDesktopWindowSize { width, height } => {
                let path = self.paths.state_path.clone();
                thread::spawn(move || {
                    crate::persist_desktop_window_size(&path, width, height);
                    engine_info!(
                        "[desktop-window-size] Persisted logical inner size {}x{} to {:?}",
                        width,
                        height,
                        path
                    );
                });
            }
            Effect::PersistRuntimeState { snapshot } => {
                self.persistence_sink.enqueue(snapshot);
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

#[cfg(test)]
mod context_order_tests {
    use super::*;

    #[test]
    fn loaded_context_pairs_are_sorted_by_key() {
        let mut variables = HashMap::new();
        variables.insert("zeta".to_string(), "z".to_string());
        variables.insert("alpha".to_string(), "a".to_string());
        variables.insert("mid".to_string(), "m".to_string());
        let pairs = ordered_context_pairs(&PromptContextFile {
            meta: ContextMeta {
                prompt_id: PromptId::ArticleSummary.to_string(),
                schema_version: 1,
                version: 1,
                updated: "2026-06-15".to_string(),
                description: None,
                changelog: None,
            },
            variables,
        });
        let keys: Vec<&str> = pairs.iter().map(|(key, _)| key.as_str()).collect();
        assert_eq!(keys, ["alpha", "mid", "zeta"]);
    }
}
