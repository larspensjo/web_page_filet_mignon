#[cfg(test)]
use std::collections::HashMap;
use std::sync::atomic::Ordering;
use std::sync::Arc;
use std::thread;
use std::time::{Duration, Instant};

use engine_logging::{engine_error, engine_info, engine_warn};
use harvester_core::morning_archive::{ExternalArchiveTarget, PreviousArchiveDisposition};
use harvester_core::{Effect, LlmResultKind, Msg, StopPolicy};
#[cfg(test)]
use harvester_engine::llm::prompt::PromptId;
#[cfg(test)]
use harvester_engine::llm::prompt_context::ContextMeta;
use harvester_engine::llm::prompt_context::PromptContextFile;
use harvester_engine::llm::LlmCommand;
use harvester_engine::{
    build_triage_archive_with, import_saved_webpages, prepare_triage_archive_with,
    ArchiveReadProgress, ArchiveTarget, ImportOptions,
};

use super::archive_progress::PublicationJob;
use super::worker::run_triage_refresh_load;
use super::{truncate_url_for_log, EffectRunner};

fn publication_disposition(
    target: &ExternalArchiveTarget,
) -> crate::archive_publication::PreviousArchiveDisposition {
    use crate::archive_publication::{
        ArchiveTargetObservation, PreviousArchiveDisposition as IoDisposition,
    };
    match target.disposition {
        PreviousArchiveDisposition::None => IoDisposition::None,
        PreviousArchiveDisposition::KeepAside => IoDisposition::KeepAside,
        PreviousArchiveDisposition::Replace { observed } => IoDisposition::Replace {
            observed: ArchiveTargetObservation {
                len: observed.len,
                modified: observed.modified,
            },
        },
    }
}

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
            Effect::LoadArticleLinks { job_id, url } => {
                let output = self.paths.output_dir.clone();
                let tx = self.msg_tx.clone();
                thread::spawn(move || {
                    let links = crate::article_links::try_load_article_links(&output, &url);
                    let _ = tx.send(Msg::ArticleLinksLoaded { job_id, url, links });
                });
            }
            Effect::StoreArticleLinks { url, links } => {
                self.persistence_sink.store_article_links(
                    &self.paths.output_dir,
                    url,
                    links,
                    self.file_write_observer.clone(),
                );
            }
            Effect::SaveResults { records } => self.result_sink.enqueue(records),
            Effect::FlushResults => {
                if let Err(error) = self.result_sink.request_flush() {
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
            Effect::ResetRunQuotas => {
                self.engine.reset_run_quota();
                if let Some(handle) = &self.llm_handle {
                    handle.reset_usage();
                }
            }
            Effect::StopFinish { policy } => {
                let immediate = matches!(policy, StopPolicy::Immediate);
                self.engine.stop(immediate);
            }
            Effect::OpenArchiveDialog {
                request_id,
                article_count,
                since_utc,
                portfolio_record,
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
                        "[archive-dialog] open requested request_id={} article_count={} default_basename={} default_file_exists={} morning_start={}",
                        request_id,
                        article_count,
                        default_basename,
                        default_file_exists,
                        portfolio_record
                            .as_ref()
                            .and_then(|record| record.confirmed_until_utc)
                            .map_or_else(|| "none".to_string(), |start| start.to_rfc3339())
                    );
                    let _ = msg_tx.send(Msg::ArchiveDialogReady {
                        request_id,
                        article_count,
                        since_utc,
                        portfolio_record: portfolio_record.map(Box::new),
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
                external,
                ordered_urls,
                window,
                requested_checkpoint,
                use_summaries,
                summaries,
                annotations,
                priority_snapshot,
                folds,
            } => {
                let msg_tx = self.msg_tx.clone();
                let output_dir = self.paths.output_dir.clone();
                let corpus_scan_index = Arc::clone(&self.corpus_scan_index);
                // The reducer has acknowledged the intent before issuing an external export.
                let publication_job = external.as_ref().map(|_| {
                    PublicationJob::start(
                        Arc::clone(&self.archive_receipts),
                        request_id,
                        self.archive_publication_observer.clone(),
                    )
                });
                let path = external.as_ref().map_or_else(
                    || output_dir.join(&basename),
                    |external| external.path.clone(),
                );
                engine_info!(
                    "[archive-dialog] export queued request_id={} path={} urls={} use_summaries={} summaries={} fold_groups={} requested_checkpoint={:?}",
                    request_id, path.display(), ordered_urls.len(), use_summaries,
                    summaries.len(), folds.len(), requested_checkpoint
                );
                thread::spawn(move || {
                    let started = Instant::now();
                    engine_info!(
                        "[archive-dialog] worker started request_id={} path={}",
                        request_id,
                        path.display()
                    );
                    let hints_wait_started = Instant::now();
                    let scan_index_lock = corpus_scan_index.lock();
                    let hints_wait_ms = hints_wait_started.elapsed().as_millis();
                    let hints = match scan_index_lock {
                        Ok(index) => Some(index.archive_read_hints()),
                        Err(_) => {
                            engine_warn!(
                                "[archive-export] scan index lock poisoned request_id={} path={} falling back to full read",
                                request_id, path.display()
                            );
                            None
                        }
                    };
                    let mut files_total = 0;
                    let mut last_reported = 0;
                    let mut last_report = started;
                    let mut on_progress = |progress: ArchiveReadProgress| {
                        let checked = progress.files_checked;
                        files_total = progress.files_total;
                        if checked == 0
                            || checked == files_total
                            || checked - last_reported >= 250
                            || last_report.elapsed() >= Duration::from_millis(200)
                        {
                            last_reported = checked;
                            last_report = Instant::now();
                            let _ = msg_tx.send(Msg::ArchiveExportProgress {
                                request_id,
                                files_checked: checked,
                                files_total,
                            });
                        }
                    };
                    if let Some((external, publication_job)) = external.zip(publication_job) {
                        let prepared = prepare_triage_archive_with(
                            &output_dir,
                            ArchiveTarget::External { path: &path },
                            &ordered_urls,
                            window,
                            use_summaries,
                            &summaries,
                            &annotations,
                            &priority_snapshot,
                            Some(&folds),
                            hints.as_ref(),
                            &mut on_progress,
                        );
                        let result = match prepared {
                            Ok(prepared) => prepared
                                .publish(|target, markdown| {
                                    crate::archive_publication::publish_archive_observed(
                                        target,
                                        markdown.as_bytes(),
                                        publication_disposition(&external),
                                        |step, publication| {
                                            publication_job.observe(step, publication)
                                        },
                                    )
                                })
                                .map_err(|error| {
                                    (
                                        error.to_string(),
                                        error.previous_kept,
                                        error.previous_since_utc,
                                    )
                                }),
                            Err(error) => Err((error.to_string(), None, None)),
                        };
                        let supersedes = |previous_start| {
                            window
                                .since_utc()
                                .zip(previous_start)
                                .is_some_and(|(since, previous)| since <= previous)
                        };
                        let reply = match result {
                            Ok((summary, publication)) => Msg::BatchArchivePublished {
                                request_id,
                                doc_count: summary.doc_count,
                                previous_kept: publication.previous_kept,
                                previous_superseded: supersedes(publication.previous_since_utc),
                            },
                            Err((reason, previous_kept, previous_start)) => {
                                Msg::BatchArchivePublicationFailed {
                                    request_id,
                                    reason,
                                    previous_kept,
                                    previous_superseded: supersedes(previous_start),
                                }
                            }
                        };
                        // Dropping the job records that the worker can no longer replace the target.
                        drop(publication_job);
                        if msg_tx.send(reply).is_err() {
                            engine_warn!("[morning-archive] target={} publication acknowledgement lost request_id={request_id}", path.display());
                        }
                        return;
                    }
                    match build_triage_archive_with(
                        &output_dir,
                        ArchiveTarget::InOutput {
                            basename: &basename,
                        },
                        &ordered_urls,
                        window,
                        use_summaries,
                        &summaries,
                        &annotations,
                        &priority_snapshot,
                        Some(&folds),
                        hints.as_ref(),
                        &mut on_progress,
                    ) {
                        Ok(summary) => {
                            engine_info!(
                                "[archive-export] hints={} hints_wait_ms={} files_read={} files_skipped={} request_id={} path={}",
                                hints.as_ref().map_or(0, |hints| hints.len()), hints_wait_ms, summary.files_read,
                                files_total - summary.files_read, request_id, path.display()
                            );
                            if let (Some(window_count), Some(counts)) =
                                (summary.window_count, summary.unexported_by_priority)
                            {
                                let [priority_5, priority_4, priority_3, priority_2, priority_1, unavailable] =
                                    counts;
                                let [same_event, identical_content] =
                                    summary.unexported_folded.unwrap_or_default();
                                engine_info!(
                                    "[archive-dialog] export completed request_id={} docs={} window_count={} unexported_by_priority={{\"5\":{},\"4\":{},\"3\":{},\"2\":{},\"1\":{},\"unavailable\":{}}} unexported_folded={{\"same_event\":{},\"identical_content\":{}}} path={}",
                                    request_id,
                                    summary.doc_count,
                                    window_count,
                                    priority_5,
                                    priority_4,
                                    priority_3,
                                    priority_2,
                                    priority_1,
                                    unavailable,
                                    same_event,
                                    identical_content,
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
                            if msg_tx
                                .send(Msg::ArchiveExportCompleted {
                                    request_id,
                                    path: summary.output_path,
                                    doc_count: summary.doc_count,
                                    requested_checkpoint,
                                })
                                .is_err()
                            {
                                engine_warn!("[archive-dialog] completion delivery failed request_id={}; reducer receiver closed", request_id);
                            } else {
                                engine_info!("[archive-dialog] completion delivered request_id={} elapsed_ms={}", request_id, started.elapsed().as_millis());
                            }
                        }
                        Err(err) => {
                            engine_warn!(
                                "[archive-dialog] export failed request_id={} basename={} reason={}",
                                request_id,
                                basename,
                                err
                            );
                            if msg_tx
                                .send(Msg::ArchiveExportFailed {
                                    request_id,
                                    basename,
                                    reason: err.to_string(),
                                })
                                .is_err()
                            {
                                engine_warn!("[archive-dialog] failure delivery failed request_id={}; reducer receiver closed", request_id);
                            }
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
                        self.send_llm_message(Msg::LlmCompleted {
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
                    self.send_llm_message(Msg::LlmCompleted {
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
                let prompts_dir = self.paths.prompts_dir.clone();
                let msg_tx = self.msg_tx.clone();
                let registry = self.prompt_registry.clone();
                let models = self.llm_metadata_models.clone();
                thread::spawn(move || {
                    super::configuration::load_overlays(&prompts_dir, &registry);
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
                    let reply = match crate::persistence::load_checkpoint_record(&path) {
                        Ok(checkpoint) => Msg::BriefingCheckpointLoaded {
                            since_utc: checkpoint.since_utc,
                            portfolio: checkpoint.portfolio,
                        },
                        Err(reason) => {
                            engine_warn!("[briefing-checkpoint] {reason}");
                            Msg::BriefingCheckpointLoadFailed { reason }
                        }
                    };
                    let _ = msg_tx.send(reply);
                });
            }
            Effect::ObserveArchiveTarget { target } => {
                let msg_tx = self.msg_tx.clone();
                thread::spawn(move || {
                    let observation = crate::archive_observation::observe_archive_target(&target);
                    let _ = msg_tx.send(Msg::ArchiveTargetObserved {
                        target,
                        observation,
                        observed_at: chrono::Utc::now(),
                    });
                });
            }
            Effect::SavePortfolioRecord {
                save_id,
                target_key,
                record,
                checkpoint,
            } => {
                let msg_tx = self.msg_tx.clone();
                let path = self.paths.briefing_checkpoint_path.clone();
                let observer = self.file_write_observer.clone();
                let archive_receipts = Arc::clone(&self.archive_receipts);
                thread::spawn(move || {
                    let started = Instant::now();
                    let result = crate::persistence::save_portfolio_record(
                        &path,
                        &target_key,
                        &record,
                        checkpoint,
                    );
                    if result.is_ok() {
                        super::observe_file_write(&observer, &path, started.elapsed());
                    }
                    archive_receipts
                        .lock()
                        .unwrap_or_else(std::sync::PoisonError::into_inner)
                        .record_save = Some((save_id, result.clone()));
                    if msg_tx
                        .send(Msg::PortfolioRecordSaved { save_id, result })
                        .is_err()
                    {
                        engine_warn!("[morning-archive] target={target_key} record acknowledgement lost save_id={save_id}");
                    }
                });
            }
            Effect::LogMorningArchiveExcusals {
                target,
                articles,
                blocked,
            } => {
                crate::archive_observation::log_excused_articles(&target, &articles, blocked);
            }
            Effect::SaveBriefingCheckpoint { save_id, since_utc } => {
                let msg_tx = self.msg_tx.clone();
                let path = self.paths.briefing_checkpoint_path.clone();
                thread::spawn(move || {
                    let s = since_utc.map(|dt| dt.to_rfc3339());
                    engine_info!(
                        "[briefing-checkpoint] write started save_id={} path={} since_utc={:?}",
                        save_id,
                        path.display(),
                        s
                    );
                    match crate::save_briefing_checkpoint(&path, s.as_deref()) {
                        Ok(()) => {
                            engine_info!(
                                "[briefing-checkpoint] write completed save_id={} path={}",
                                save_id,
                                path.display()
                            );
                            if msg_tx
                                .send(Msg::BriefingCheckpointSaveSucceeded { save_id })
                                .is_err()
                            {
                                engine_warn!("[briefing-checkpoint] completion delivery failed save_id={}; reducer receiver closed", save_id);
                            }
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
                let tx = self.msg_tx.clone();
                thread::spawn(move || {
                    crate::persistence::persist_desktop_window_size_with_notices(
                        &path,
                        width,
                        height,
                        |message| {
                            let _ = tx.send(Msg::RuntimeStateNotice { message });
                        },
                    );
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
