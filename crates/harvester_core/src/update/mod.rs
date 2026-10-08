use engine_logging::{engine_info, engine_warn};

use crate::state::InitialArticleWindowOutcome as StartupWindow;
use crate::{AppState, Effect, Msg, SessionState};

mod archive;
mod briefing;
mod import;
mod llm_completed;
mod model_dispatch;
mod pipeline_run;
mod polling;
pub(crate) mod processing;
mod reuse;
pub(crate) mod signal_candidate;
mod summary_cache_support;
mod triage;
mod url_input;
mod waves;

#[cfg(test)]
pub(crate) mod test_support;
#[cfg(test)]
mod tests;

/// Pure update function: applies a message to state and returns any effects.
pub fn update(mut state: AppState, msg: Msg) -> (AppState, Vec<Effect>) {
    state.record_startup_reply(&msg);
    let view_change_requested =
        matches!(&msg, Msg::JobSelected { .. } | Msg::JobListModeSet { .. });
    let previous_view_selection = (state.job_list_mode(), state.selected_job_id());
    let run_was_active = state.run_progress_is_active();
    let stop_requested = matches!(&msg, Msg::StopFinishClicked)
        && state.stop_finish_button_state().policy().is_some();
    let pending_before_stop = stop_requested.then(|| state.pending_intake_urls().len());
    let progress_before = pipeline_run::progress_before(&state, &msg);
    let unfinished_revisions = state.unfinished_revisions();
    let completed_identity = match &msg {
        Msg::LlmCompleted { request_id, .. } => state
            .signal_candidate()
            .url_for_request(*request_id)
            .and_then(|url| {
                state
                    .pre_triage()
                    .article_content_hash(url)
                    .map(|hash| (url.to_string(), hash.to_string()))
            })
            .or_else(|| {
                state
                    .briefing()
                    .find_article_by_request_id(*request_id)
                    .and_then(|index| state.briefing().articles().get(index))
                    .map(|article| (article.url.clone(), article.content_hash.clone()))
            })
            .or_else(|| {
                state
                    .triage()
                    .find_article_by_request_id(*request_id)
                    .and_then(|index| state.triage().articles().get(index))
                    .map(|article| (article.url.clone(), article.content_hash.clone()))
            }),
        _ => None,
    };
    let persist_runtime_state = matches!(
        &msg,
        Msg::FetchTimeRecoveryCompleted
            | Msg::JobDone {
                result: crate::JobResultKind::Success,
                ..
            }
            | Msg::FetchOutcomeClassified {
                class: harvester_engine::FetchOutcomeClass::PermanentBlock
                    | harvester_engine::FetchOutcomeClass::Success,
                ..
            }
    ) || (matches!(&msg, Msg::SourcePollCompleted { .. })
        && !state.pipeline_intake_open());
    let mut effects = match msg {
        Msg::ValidatedResultReceived { record } => {
            match *record {
                crate::SavedResult::Summary(key, entry) => {
                    state.store_summary_result(key, entry.result, entry.created_at_utc)
                }
                crate::SavedResult::Triage(key, entry) => {
                    state.store_frozen_triage_result(key, entry.result, entry.created_at_utc)
                }
                crate::SavedResult::SignalCandidate(key, entry) => {
                    state.store_signal_candidate_result(key, entry.result, entry.created_at_utc)
                }
            }
            Vec::new()
        }
        Msg::ResultStoreUnavailable { reason } => {
            state.refuse_result_store(reason);
            Vec::new()
        }
        Msg::InputChanged(text) => {
            state.set_input_buffer(text);
            Vec::new()
        }
        Msg::JobsSearchQueryChanged(text) => {
            state.set_jobs_search_query(text);
            Vec::new()
        }
        Msg::JobsSearchCleared => {
            state.clear_jobs_search_query();
            Vec::new()
        }
        Msg::RestoreDesktopView {
            mode,
            selected_article_url,
            now,
        } => {
            if let Some(mode) = mode {
                state.set_job_list_mode(mode);
            }
            state.pending_selected_article_url = selected_article_url;
            state.observe_utc(now);
            Vec::new()
        }
        Msg::SavedArticlesLoaded {
            request_id,
            articles,
        } => {
            if state.triage_in_flight_request_id() == Some(request_id) {
                state.saved_articles_loaded(articles);
            }
            Vec::new()
        }
        Msg::StartupHydrationRequested => {
            state.mark_prompt_contexts_pending();
            state.mark_triage_metadata_pending();
            state.startup_inputs = Default::default();
            vec![
                Effect::LoadPromptContexts,
                Effect::LoadLlmMetadata,
                Effect::LoadBriefingCheckpoint,
            ]
        }
        Msg::UrlsSubmitted => {
            let raw = state.input_buffer().to_owned();
            let urls = url_input::parse_urls(&raw);
            if urls.is_empty() {
                return (state, Vec::new());
            }
            if matches!(
                state.session(),
                SessionState::Finishing | SessionState::Finished
            ) {
                return (state, Vec::new());
            }

            let ingest = state.ingest_urls(urls, chrono::Utc::now());
            state.set_last_paste_stats(ingest.enqueued, ingest.skipped);
            if ingest.enqueued > 0 {
                state.clear_input_buffer();
            }
            ingest.effects
        }
        Msg::StopFinishClicked => {
            if let Some(policy) = state.stop_finish_button_state().policy() {
                pipeline_run::handle_stop_for_pipeline(&mut state);
                state.finish_session();
                vec![Effect::StopFinish { policy }]
            } else {
                Vec::new()
            }
        }
        Msg::ArchiveClicked => archive::handle_archive_clicked(&mut state),
        Msg::JobProgress {
            job_id,
            stage,
            tokens,
            bytes,
        } => {
            state.apply_progress(job_id, stage, tokens, bytes);
            Vec::new()
        }
        Msg::JobDone {
            job_id,
            result,
            extracted_links,
            fetched_utc,
        } => {
            let successful = matches!(result, crate::JobResultKind::Success);
            let stopped_drain = state.pipeline_run_phase() == crate::PipelineRunPhase::Stopping
                || state.session() == SessionState::Finishing;
            let link_effect = (successful && !extracted_links.is_empty())
                .then(|| {
                    state.job_url(job_id).map(|url| Effect::StoreArticleLinks {
                        url: url.to_owned(),
                        links: extracted_links.clone(),
                    })
                })
                .flatten();
            state.apply_done(job_id, result, extracted_links, fetched_utc);
            if successful {
                if let Some(url) = state.job_url(job_id).map(str::to_owned) {
                    state.remove_pending_intake_url(&url);
                }
            }
            if !stopped_drain || successful {
                state.request_pre_triage_refresh_evaluation(true);
            }
            link_effect.into_iter().collect()
        }

        Msg::JobSelected { job_id } => {
            state.pending_selected_article_url = None;
            let previous = state.selected_job_id();
            state.select_job(job_id);
            if state.selected_job_id() == Some(job_id) && previous != Some(job_id) {
                state
                    .article_links_load_url(job_id)
                    .map(|url| Effect::LoadArticleLinks {
                        job_id,
                        url: url.to_owned(),
                    })
                    .into_iter()
                    .collect()
            } else {
                Vec::new()
            }
        }
        Msg::ArticleLinksLoaded { job_id, url, links } => {
            match links {
                Ok(links) => state.article_links_loaded(job_id, &url, links),
                Err(error) => engine_warn!(
                    "[article-links] load job_id={} url={} error={}",
                    job_id,
                    url,
                    error
                ),
            }
            Vec::new()
        }
        Msg::RuntimeStateNotice { message } => {
            state.set_runtime_state_notice(message);
            Vec::new()
        }
        Msg::FetchTimeRecoveryCompleted => {
            state.fetch_time_recovery_done = true;
            Vec::new()
        }
        Msg::JobListModeSet { mode } => {
            state.pending_selected_article_url = None;
            state.set_job_list_mode(mode);
            Vec::new()
        }
        Msg::PipelineRunRequested { scope } => {
            pipeline_run::handle_pipeline_requested(&mut state, scope)
        }
        Msg::PipelineRunAdvance => pipeline_run::handle_pipeline_advance(&mut state),
        Msg::RunFinishedNoticeDismissed => {
            pipeline_run::dismiss_run_notice(&mut state);
            Vec::new()
        }
        Msg::ExtractedLinkOpenRequested { job_id, link_index } => state
            .job_extracted_link_url(job_id, link_index)
            .map(|url| Effect::OpenUrlInBrowser { url })
            .into_iter()
            .collect(),
        Msg::RestoreCompletedJobs(entries) => {
            if entries.is_empty() {
                state.startup_inputs.initial_article_window = StartupWindow::Empty;
            } else {
                state.restore_completed_jobs(entries);
                state.startup_inputs.initial_article_window = StartupWindow::Pending;
                state.request_pre_triage_refresh_evaluation(false);
            }
            Vec::new()
        }
        Msg::RestorePendingIntake(urls) => {
            state.restore_pending_intake(urls);
            Vec::new()
        }
        Msg::EvaluatePreTriageRefresh {
            ordered_urls,
            triggered_by_job_done,
        } => triage::handle_evaluate_pre_triage_refresh(
            &mut state,
            ordered_urls,
            triggered_by_job_done,
        ),
        Msg::DesktopWindowResizeCompleted {
            inner_width,
            inner_height,
        } => {
            vec![Effect::PersistDesktopWindowSize {
                width: inner_width,
                height: inner_height,
            }]
        }

        Msg::LlmCompleted {
            request_id,
            result,
            metadata,
        } => llm_completed::handle(&mut state, request_id, result, metadata),

        Msg::LlmQuotaConfigured { limits } => {
            state.set_llm_quota_limits(limits);
            state.mark_dirty();
            Vec::new()
        }
        Msg::LlmQuotaUsageUpdated { usage } => {
            state.set_llm_quota_usage(usage);
            state.mark_dirty();
            Vec::new()
        }

        Msg::BriefingCheckpointLoaded { since_utc } => {
            state.restored_checkpoint_ready = true;
            briefing::handle_checkpoint_loaded(&mut state, since_utc)
        }
        Msg::BriefingCheckpointSaveSucceeded { save_id } => {
            briefing::handle_checkpoint_save_succeeded(&mut state, save_id)
        }
        Msg::BriefingCheckpointSaveFailed { save_id, reason } => {
            briefing::handle_checkpoint_save_failed(&mut state, save_id, reason)
        }
        Msg::BriefingCheckpointSet(since) => briefing::handle_checkpoint_set(&mut state, since),
        Msg::ArchiveDialogReady {
            request_id,
            article_count,
            since_utc,
            default_basename,
            default_file_exists,
            export_dir,
            pending_pre_triage_count,
            token_estimates,
            signal_candidate_default,
            signal_candidate_count,
            signal_candidate_scoring_done,
            signal_candidate_scoring_total,
            signal_candidate_token_estimates,
        } => archive::handle_dialog_ready(
            &mut state,
            request_id,
            article_count,
            since_utc,
            default_basename,
            default_file_exists,
            export_dir,
            pending_pre_triage_count,
            token_estimates,
            signal_candidate_default,
            signal_candidate_count,
            signal_candidate_scoring_done,
            signal_candidate_scoring_total,
            signal_candidate_token_estimates,
        ),
        Msg::ArchiveDialogSubmitted {
            request_id,
            basename,
            set_checkpoint,
            submitted_at,
            use_summaries,
            use_signal_candidates,
        } => archive::handle_dialog_submitted(
            &mut state,
            request_id,
            basename,
            set_checkpoint,
            submitted_at,
            use_summaries,
            use_signal_candidates,
        ),
        Msg::ArchiveExportCompleted {
            request_id,
            requested_checkpoint,
            ..
        } => archive::handle_export_completed(&mut state, request_id, requested_checkpoint),
        Msg::ArchiveExportFailed {
            request_id,
            basename,
            reason,
        } => archive::handle_export_failed(&mut state, request_id, basename, reason),
        Msg::ToggleSignalCandidateExclusion { signal_key } => {
            let mut effects = Vec::new();
            signal_candidate::handle_toggle_exclusion(&mut state, signal_key, &mut effects);
            effects
        }

        Msg::TriageArticlesLoaded { request_id, delta } => {
            triage::handle_articles_loaded(&mut state, request_id, delta)
        }
        Msg::TriageArticlesLoadProgress { .. } => Vec::new(),
        Msg::TriageArticlesLoadFailed { request_id, reason } => {
            triage::handle_articles_load_failed(&mut state, request_id, reason)
        }
        Msg::ProcessingConfigurationLoaded {
            request_id,
            contexts,
            active_versions,
            effective_models,
            preparation_budget,
        } => {
            if state
                .processing_start
                .as_ref()
                .is_some_and(|p| p.configuration_request == Some(request_id))
            {
                state.set_prompt_contexts(contexts);
                state.mark_prompt_template_files_loaded();
                state.set_llm_metadata(active_versions, effective_models);
                state.mark_triage_metadata_ready();
                state.mark_briefing_metadata_ready();
                state.processing_budget = Some(preparation_budget);
                if let Some(run) = state.pipeline_admission.as_mut() {
                    run.configured = true;
                }
                state
                    .processing_start
                    .as_mut()
                    .unwrap()
                    .configuration_request = None;
                processing::resume(&mut state)
            } else {
                Vec::new()
            }
        }
        Msg::ProcessingConfigurationFailed { request_id, reason } => {
            if state
                .processing_start
                .as_ref()
                .is_some_and(|p| p.configuration_request == Some(request_id))
            {
                processing::fail(&mut state, reason);
            }
            Vec::new()
        }
        Msg::PromptContextsLoaded { .. }
        | Msg::PromptContextsLoadFailed { .. }
        | Msg::PromptTemplateFilesLoaded
        | Msg::LlmMetadataLoaded { .. }
            if state.pipeline_ready() =>
        {
            Vec::new()
        }
        Msg::PromptContextsLoaded { contexts } => {
            engine_info!("[PromptContext] Loaded {} context(s)", contexts.len());
            state.set_prompt_contexts(contexts);
            state.mark_triage_metadata_ready();
            state.mark_dirty();
            Vec::new()
        }
        Msg::PromptContextsLoadFailed { reason } => {
            engine_warn!("[PromptContext] Failed to load contexts: {}", reason);
            state.mark_prompt_contexts_load_failed();
            state.mark_triage_metadata_pending();
            state.mark_dirty();
            Vec::new()
        }
        Msg::PromptTemplateFilesLoaded => {
            engine_info!("[prompt-template] Saved template overlays loaded");
            state.mark_prompt_template_files_loaded();
            state.mark_dirty();
            Vec::new()
        }
        Msg::LlmMetadataLoaded {
            active_versions,
            effective_models,
        } => {
            engine_info!(
                "[LlmMetadata] Loaded {} active version(s)",
                active_versions.len()
            );
            state.set_llm_metadata(active_versions, effective_models);
            state.reconcile_ai_availability_from_metadata();
            state.mark_briefing_metadata_ready();
            state.mark_triage_metadata_ready();
            state.mark_dirty();
            Vec::new()
        }
        Msg::AiAvailabilityDetected { availability } => {
            state.set_ai_availability(availability);
            state.mark_dirty();
            Vec::new()
        }
        Msg::SummaryCacheHydrated { cache } => {
            engine_info!(
                "[summary-cache] Hydrated {} entries from persistent store",
                cache.len()
            );
            state.set_summary_cache(cache);
            state.mark_dirty();
            Vec::new()
        }
        Msg::SignalCandidateCacheLoaded { cache } => {
            engine_info!(
                "[signal-cache] Hydrated {} entries from persistent store",
                cache.len()
            );
            signal_candidate::handle_cache_loaded(&mut state, cache);
            Vec::new()
        }
        Msg::SignalCandidateOverridesLoaded { overrides } => {
            engine_info!(
                "[signal-overrides] Hydrated {} override(s) from persistent store",
                overrides.len()
            );
            signal_candidate::handle_overrides_loaded(&mut state, overrides);
            Vec::new()
        }
        Msg::TriageCacheHydrated { cache } => {
            engine_info!(
                "[triage-cache] Hydrated {} entries from persistent store",
                cache.len()
            );
            state.set_triage_cache(cache);
            state.mark_dirty();
            Vec::new()
        }
        Msg::OpenInBrowserClicked => match state.selected_article_url() {
            Some(url) => {
                engine_info!("[browser] Open in browser requested for URL: {}", url);
                vec![Effect::OpenUrlInBrowser { url }]
            }
            None => Vec::new(),
        },

        Msg::PollStarted { total } => polling::handle_poll_started(&mut state, total),
        Msg::SourcePollCompleted {
            source_id,
            urls,
            kind,
            parsed,
            dedup_filtered,
        } => polling::handle_source_poll_completed(
            &mut state,
            source_id,
            urls,
            kind,
            parsed,
            dedup_filtered,
        ),
        Msg::SourcePollFailed { source_id, error } => {
            polling::handle_source_poll_failed(&mut state, source_id, error)
        }
        Msg::AllSourcesPollEnded => polling::handle_all_sources_poll_ended(&mut state),
        Msg::ImportSavedWebpagesRequested { dir } => {
            import::handle_import_requested(&mut state, dir)
        }
        Msg::ImportSavedWebpagesCompleted { request_id, report } => {
            import::handle_import_completed(&mut state, request_id, report)
        }
        Msg::ImportSavedWebpagesFailed { request_id, reason } => {
            import::handle_import_failed(&mut state, request_id, reason)
        }

        Msg::FetchOutcomeClassified {
            job_id,
            class,
            failure_label,
            recorded_at,
        } => {
            if let Some(url) = state.job_url_for(job_id).map(|s| s.to_string()) {
                let changed = state.blacklist.record_for_url(
                    &url,
                    class,
                    failure_label.as_deref(),
                    recorded_at,
                );
                if changed {
                    state.mark_dirty();
                }
            }
            Vec::new()
        }

        Msg::BlacklistHydrated { state: blacklist } => {
            state.set_blacklist(blacklist);
            Vec::new()
        }

        Msg::Tick { now } => {
            state.observe_utc(now);
            state.advance_tick();
            let tick = state.current_tick();
            let has_in_flight_jobs = state.batch_observation().jobs_in_flight > 0;
            triage::dispatch_pre_triage_if_due(&mut state, tick, has_in_flight_jobs)
        }
    };

    model_dispatch::dispatch_model_work(&mut state, &mut effects);
    let after_revisions = state.unfinished_revisions();
    if after_revisions.0 != unfinished_revisions.0 {
        if after_revisions.1 == unfinished_revisions.1 {
            if let Some((url, content_hash)) = completed_identity {
                state.refresh_unfinished_identity(&url, &content_hash);
            } else {
                state.recompute_unfinished_work();
            }
        } else {
            state.recompute_unfinished_work();
        }
    }
    state.rebuild_saved_results_if_changed();
    if let Some(effect) = state.restore_desktop_selection_if_ready() {
        effects.push(effect);
    }
    pipeline_run::record_progress_after(&mut state, progress_before);
    pipeline_run::finish_if_settled(&mut state);
    let records = std::mem::take(&mut state.pending_results);
    if !records.is_empty() {
        // Save before any flush emitted by settlement, so it includes this completion.
        effects.insert(0, Effect::SaveResults { records });
    }
    if stop_requested || (run_was_active && !state.run_progress_is_active()) {
        effects.push(Effect::FlushResults);
    }
    if persist_runtime_state
        || (view_change_requested
            && previous_view_selection != (state.job_list_mode(), state.selected_job_id()))
        || pending_before_stop.is_some_and(|before| state.pending_intake_urls().len() != before)
    {
        effects.push(Effect::PersistRuntimeState {
            snapshot: crate::PersistenceSnapshot::capture(&state),
        });
    }
    (state, effects)
}

#[cfg(test)]
mod desktop_contract_tests {
    use chrono::{DateTime, Utc};

    use super::update;
    use crate::{AppState, Effect, Msg};

    #[test]
    fn successful_job_completion_emits_a_runtime_persistence_snapshot() {
        let (state, _effects) = update(
            AppState::default(),
            Msg::InputChanged("https://example.com/article".to_string()),
        );
        let (state, effects) = update(state, Msg::UrlsSubmitted);
        let job_id = effects
            .iter()
            .find_map(|effect| match effect {
                Effect::EnqueueUrl { job_id, .. } => Some(*job_id),
                _ => None,
            })
            .expect("URL submission must enqueue a job");

        let (_state, effects) = update(
            state,
            Msg::JobDone {
                job_id,
                result: crate::JobResultKind::Success,
                extracted_links: Vec::new(),
                fetched_utc: None,
            },
        );

        assert!(matches!(
            effects.last(),
            Some(Effect::PersistRuntimeState { snapshot })
                if snapshot.completed.len() == 1 && snapshot.completed[0].url == "https://example.com/article"
        ));
        assert!(!effects
            .iter()
            .any(|effect| matches!(effect, Effect::StoreArticleLinks { .. })));
    }

    #[test]
    fn urls_submitted_clears_the_pasted_input_buffer() {
        let input = "https://a.example.com \n\n  https://b.example.com\n   \n";
        let (state, _) = update(AppState::new(), Msg::InputChanged(input.into()));
        assert_eq!(state.input_buffer(), input);

        let (state, effects) = update(state, Msg::UrlsSubmitted);
        assert!(state.input_buffer().is_empty());
        assert_eq!(state.view().job_count, 2);
        assert_eq!(
            effects
                .iter()
                .filter(|e| matches!(e, Effect::EnqueueUrl { .. }))
                .count(),
            2
        );

        let (state, effects) = update(state, Msg::UrlsSubmitted);
        assert!(state.input_buffer().is_empty());
        assert_eq!(state.view().job_count, 2);
        assert!(effects.is_empty());
    }

    #[test]
    fn tick_observes_time_and_makes_view_deterministic() {
        let now = DateTime::parse_from_rfc3339("2026-09-03T12:00:00Z")
            .unwrap()
            .with_timezone(&Utc);
        let (state, _) = update(AppState::default(), Msg::tick_at(now));
        assert_eq!(state.last_observed_utc(), Some(now));
        assert_eq!(state.view(), state.view());
    }

    #[test]
    fn extracted_link_opening_is_resolved_by_core_index() {
        let (state, _) = update(
            AppState::default(),
            Msg::InputChanged("https://article.example".into()),
        );
        let (state, _) = update(state, Msg::UrlsSubmitted);
        let (state, _) = update(
            state,
            Msg::JobDone {
                job_id: 1,
                result: crate::JobResultKind::Success,
                extracted_links: vec![harvester_engine::ExtractedLink {
                    url: "https://linked.example".into(),
                    text: None,
                    kind: harvester_engine::LinkKind::Hyperlink,
                }],
                fetched_utc: None,
            },
        );
        let (_, effects) = update(
            state.clone(),
            Msg::ExtractedLinkOpenRequested {
                job_id: 1,
                link_index: 0,
            },
        );
        assert_eq!(
            effects,
            vec![Effect::OpenUrlInBrowser {
                url: "https://linked.example/".into()
            }]
        );
        let (_, effects) = update(
            state,
            Msg::ExtractedLinkOpenRequested {
                job_id: 1,
                link_index: 1,
            },
        );
        assert!(effects.is_empty());
    }
}
