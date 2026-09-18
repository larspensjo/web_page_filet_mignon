use engine_logging::{engine_info, engine_warn};

use crate::{AppState, Effect, Msg, SessionState};

mod archive;
mod batch_results;
mod briefing;
mod import;
mod llm_completed;
mod pipeline_run;
mod polling;
mod prompt_lab;
pub(crate) mod signal_candidate;
mod summary_cache_support;
mod triage;
mod url_input;

#[cfg(test)]
mod tests;

/// Pure update function: applies a message to state and returns any effects.
pub fn update(mut state: AppState, msg: Msg) -> (AppState, Vec<Effect>) {
    let progress_before = pipeline_run::progress_before(&state, &msg);
    let effects = match msg {
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
        Msg::StartupHydrationRequested => {
            state.mark_triage_metadata_pending();
            vec![
                Effect::LoadPromptContexts,
                Effect::LoadLlmMetadata,
                Effect::LoadPromptLabModelCatalog,
                Effect::LoadBriefingHistory,
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
            content_preview,
        } => {
            state.apply_progress(job_id, stage, tokens, bytes, content_preview);
            Vec::new()
        }
        Msg::JobDone {
            job_id,
            result,
            content_preview,
            extracted_links,
            fetched_utc,
        } => {
            state.apply_done(
                job_id,
                result,
                content_preview,
                extracted_links,
                fetched_utc,
            );
            state.request_pre_triage_refresh_evaluation(true);
            Vec::new()
        }
        Msg::LinkToggleRequested {
            job_id,
            link_index,
            checked,
        } => {
            let mut effects = Vec::new();
            if let Some((url, downloaded_path)) = state.link_metadata(job_id, link_index) {
                if checked && state.mark_link_download_requested(job_id, link_index) {
                    effects.push(Effect::DownloadLinkedPage {
                        job_id,
                        link_index,
                        url,
                    });
                } else if !checked && state.mark_link_deleted(job_id, link_index) {
                    if let Some(path) = downloaded_path {
                        effects.push(Effect::DeleteLinkedPage {
                            job_id,
                            link_index,
                            path,
                        });
                    }
                }
            }
            effects
        }
        Msg::LinkDownloadStarted { job_id, link_index } => {
            state.mark_link_download_requested(job_id, link_index);
            Vec::new()
        }
        Msg::LinkDownloadCompleted {
            job_id,
            link_index,
            path,
        } => {
            state.mark_link_download_completed(job_id, link_index, path);
            Vec::new()
        }
        Msg::LinkDownloadFailed {
            job_id,
            link_index,
            error,
        } => {
            state.mark_link_download_failed(job_id, link_index, error);
            Vec::new()
        }
        Msg::LinkDeleted { job_id, link_index } => {
            state.mark_link_deleted(job_id, link_index);
            Vec::new()
        }
        Msg::JobSelected { job_id } => {
            state.select_job(job_id);
            let Some(url) = state.selected_job_url() else {
                return (state, Vec::new());
            };
            let url_changed = state.prompt_lab().url_input() != url;
            if url_changed {
                state.prompt_lab_mut().set_url_input(url.clone());
                state.mark_dirty();
            }
            let should_resolve = state.prompt_lab().pending_resolve_id().is_none()
                && (url_changed || state.prompt_lab().resolved_url_snapshot().is_none());
            if should_resolve {
                let resolve_id = state.allocate_next_prompt_lab_resolve_id();
                state.prompt_lab_mut().begin_url_resolution(resolve_id);
                state.mark_dirty();
                vec![Effect::ResolvePromptLabInputFromUrl { resolve_id, url }]
            } else {
                Vec::new()
            }
        }
        Msg::WorkspaceViewSet { view } => {
            state.set_workspace_view(view);
            Vec::new()
        }
        Msg::JobListModeSet { mode } => {
            state.set_job_list_mode(mode);
            Vec::new()
        }
        Msg::JobsSearchRevealRequested => {
            state.set_workspace_view(crate::WorkspaceView::Review);
            Vec::new()
        }
        Msg::TrendsViewOpened => {
            state.set_workspace_view(crate::WorkspaceView::Trends);
            vec![Effect::LoadEntityIndex]
        }
        Msg::PipelineRunRequested => {
            pipeline_run::handle_pipeline_requested(&mut state);
            Vec::new()
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
            state.restore_completed_jobs(entries);
            state.request_pre_triage_refresh_evaluation(false);
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
        Msg::WindowResizeCompleted {
            outer_width,
            outer_height,
        } => {
            vec![Effect::PersistWindowSize {
                width: outer_width,
                height: outer_height,
            }]
        }
        Msg::DesktopWindowResizeCompleted {
            inner_width,
            inner_height,
        } => {
            vec![Effect::PersistDesktopWindowSize {
                width: inner_width,
                height: inner_height,
            }]
        }
        Msg::RequestLlmCompletion {
            prompt_id,
            prompt_version,
            model_override,
            input_content,
            context,
            template_override,
        } => {
            let request_id = state.allocate_next_llm_request_id();
            state.record_pending_llm_request(request_id, prompt_id);
            vec![Effect::RequestLlmCompletion {
                request_id,
                prompt_id,
                prompt_version,
                model_override,
                input_content,
                context,
                template_override,
                extra_template_vars: vec![],
            }]
        }
        Msg::LlmCompleted {
            request_id,
            result,
            metadata,
        } => llm_completed::handle(&mut state, request_id, result, metadata),
        Msg::RearmDeferredBatchStages => {
            let deferred_signal_urls = state.signal_candidate().deferred_urls();
            state.triage_mut().rearm_deferred();
            state.briefing_mut().rearm_deferred();
            state.signal_candidate_mut().rearm_deferred();
            let mut effects = Vec::new();
            if matches!(state.triage().phase(), crate::TriagePhase::Triaging) {
                triage::dispatch_next_triage_step(&mut state, &mut effects);
            }
            if matches!(state.briefing().phase(), crate::BriefingPhase::Summarizing) {
                briefing::dispatch_next_briefing_step(&mut state, &mut effects);
            }
            for url in deferred_signal_urls {
                signal_candidate::try_enqueue(&mut state, &url, &mut effects);
            }
            state.mark_dirty();
            effects
        }
        Msg::BatchResultsCollected { entries } => batch_results::handle(&mut state, entries),
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
        Msg::GenerateBriefingClicked => briefing::handle_generate_clicked(&mut state),
        Msg::NextBriefingItemClicked => briefing::handle_next_item_clicked(&mut state),
        Msg::PrepareSummariesClicked => briefing::handle_prepare_summaries_clicked(&mut state),
        Msg::BriefingHistoryLoaded { entries } => {
            briefing::handle_history_loaded(&mut state, entries)
        }
        Msg::BriefingCheckpointLoaded { since_utc } => {
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
        Msg::ArticlesLoaded {
            articles,
            collection_text,
        } => briefing::handle_articles_loaded(&mut state, articles, collection_text),
        Msg::ArticlesLoadFailed { reason } => {
            briefing::handle_articles_load_failed(&mut state, reason)
        }
        Msg::TriageClicked => triage::handle_triage_clicked(&mut state),
        Msg::TriageArticlesLoaded {
            request_id,
            articles,
        } => triage::handle_articles_loaded(&mut state, request_id, articles),
        Msg::TriageArticlesLoadProgress {
            request_id,
            files_scanned,
            files_total,
        } => triage::handle_articles_load_progress(
            &mut state,
            request_id,
            files_scanned,
            files_total,
        ),
        Msg::TriageArticlesLoadFailed { request_id, reason } => {
            triage::handle_articles_load_failed(&mut state, request_id, reason)
        }
        Msg::PreTriageDecisionSet { key, decision } => {
            triage::handle_pre_triage_decision_set(&mut state, key, decision)
        }
        Msg::PreTriageApplyClicked => triage::handle_pre_triage_apply_clicked(&mut state),
        Msg::PreTriageResetClicked => triage::handle_pre_triage_reset_clicked(&mut state),
        Msg::PromptContextsLoaded { contexts } => {
            engine_info!("[PromptContext] Loaded {} context(s)", contexts.len());
            state.prompt_lab_mut().clear_context_overlays();
            state.set_prompt_contexts(contexts);
            state.mark_triage_metadata_ready();
            state.mark_dirty();
            briefing::resume_deferred_exec_dispatch(&mut state)
        }
        Msg::PromptContextsLoadFailed { reason } => {
            engine_warn!("[PromptContext] Failed to load contexts: {}", reason);
            state.mark_prompt_contexts_load_failed();
            state.mark_triage_metadata_pending();
            state.mark_dirty();
            Vec::new()
        }
        Msg::PromptTemplateFilesLoaded => {
            engine_info!("[prompt-lab-template] Saved template overlays loaded");
            state.mark_prompt_template_files_loaded();
            state.mark_dirty();
            let mut effects = Vec::new();
            if state.briefing().exec_dispatch_deferred() && !state.llm_metadata_loaded() {
                effects.push(Effect::LoadLlmMetadata);
            }
            effects.extend(briefing::resume_deferred_exec_dispatch(&mut state));
            effects
        }
        Msg::LlmMetadataLoaded {
            active_versions,
            effective_models,
            templates,
        } => {
            engine_info!(
                "[LlmMetadata] Loaded {} active version(s)",
                active_versions.len()
            );
            state.set_llm_metadata(active_versions, effective_models, templates);
            state.reconcile_ai_availability_from_metadata();
            state.mark_briefing_metadata_ready();
            state.mark_triage_metadata_ready();
            state.mark_dirty();
            let mut effects = Vec::new();
            briefing::try_start_briefing_with_metadata(&mut state, &mut effects);
            effects.extend(briefing::resume_deferred_exec_dispatch(&mut state));
            signal_candidate::sweep_eligible_after_hydration(&mut state, &mut effects);
            effects
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
            let mut effects = Vec::new();
            signal_candidate::sweep_eligible_after_hydration(&mut state, &mut effects);
            effects
        }
        Msg::SignalCandidateCacheLoaded { cache } => {
            engine_info!(
                "[signal-cache] Hydrated {} entries from persistent store",
                cache.len()
            );
            let mut effects = Vec::new();
            signal_candidate::handle_cache_loaded(&mut state, cache, &mut effects);
            effects
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
            let mut effects = Vec::new();
            signal_candidate::sweep_eligible_after_hydration(&mut state, &mut effects);
            effects
        }
        Msg::PreTriageOverridesHydrated { overrides } => {
            state.set_pre_triage_manual_overrides(overrides);
            Vec::new()
        }
        Msg::OpenInBrowserClicked => match state.selected_article_url() {
            Some(url) => {
                engine_info!("[browser] Open in browser requested for URL: {}", url);
                vec![Effect::OpenUrlInBrowser { url }]
            }
            None => Vec::new(),
        },
        Msg::PollSourcesClicked => {
            let effects = polling::handle_poll_sources_clicked(&mut state);
            if !effects.is_empty() {
                pipeline_run::begin_run_if_needed(&mut state);
            }
            effects
        }
        Msg::PollIndirectLinks => polling::handle_poll_indirect_links(&mut state),
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
        Msg::TrendCategorySelected { category } => {
            state.set_active_trend_category(category);
            Vec::new()
        }
        Msg::PromptLabOpenRequested => prompt_lab::handle_open_requested(&mut state),
        Msg::PromptLabCloseRequested => prompt_lab::handle_close_requested(&mut state),
        Msg::PromptLabStageSelected { stage } => {
            prompt_lab::handle_stage_selected(&mut state, stage)
        }
        Msg::PromptLabInputSourceSelected { source } => {
            prompt_lab::handle_input_source_selected(&mut state, source)
        }
        Msg::PromptLabInputChanged { text } => prompt_lab::handle_input_changed(&mut state, text),
        Msg::PromptLabAdvancedModeSet { enabled } => {
            prompt_lab::handle_advanced_mode_set(&mut state, enabled)
        }
        Msg::PromptLabModelCatalogLoaded { models, source } => {
            prompt_lab::handle_model_catalog_loaded(&mut state, models, source)
        }
        Msg::PromptLabModelOverrideSet { model } => {
            prompt_lab::handle_model_override_set(&mut state, model)
        }
        Msg::PromptLabCompareSectionToggled => {
            prompt_lab::handle_compare_section_toggled(&mut state)
        }
        Msg::PromptLabContextSectionToggled => {
            prompt_lab::handle_context_section_toggled(&mut state)
        }
        Msg::PromptLabTemplateSectionToggled => {
            prompt_lab::handle_template_section_toggled(&mut state)
        }
        Msg::PromptLabRunDetailsSectionToggled => {
            prompt_lab::handle_run_details_section_toggled(&mut state)
        }
        Msg::PromptLabUrlInputChanged { url } => {
            prompt_lab::handle_url_input_changed(&mut state, url)
        }
        Msg::PromptLabResolveRequested => prompt_lab::handle_resolve_requested(&mut state),
        Msg::PromptLabInputResolved { resolve_id, result } => {
            prompt_lab::handle_input_resolved(&mut state, resolve_id, result)
        }
        Msg::PromptLabContextEditorOpened => prompt_lab::handle_context_editor_opened(&mut state),
        Msg::PromptLabContextDraftChanged { text } => {
            prompt_lab::handle_context_draft_changed(&mut state, text)
        }
        Msg::PromptLabContextApplyRequested => {
            prompt_lab::handle_context_apply_requested(&mut state)
        }
        Msg::PromptLabContextApplyAndRerunRequested => {
            prompt_lab::handle_context_apply_and_rerun_requested(&mut state)
        }
        Msg::PromptLabContextRevertRequested => {
            prompt_lab::handle_context_revert_requested(&mut state)
        }
        Msg::PromptLabContextSaveRequested => prompt_lab::handle_context_save_requested(&mut state),
        Msg::PromptLabContextReloadRequested => {
            prompt_lab::handle_context_reload_requested(&mut state)
        }
        Msg::PromptLabContextSaved {
            prompt_id,
            path,
            version,
        } => prompt_lab::handle_context_saved(&mut state, prompt_id, path, version),
        Msg::PromptLabContextSaveFailed { prompt_id, reason } => {
            prompt_lab::handle_context_save_failed(&mut state, prompt_id, reason)
        }
        Msg::PromptLabTemplateEditorToggled => {
            prompt_lab::handle_template_editor_toggled(&mut state)
        }
        Msg::PromptLabTemplateSystemDraftChanged { text } => {
            prompt_lab::handle_template_system_draft_changed(&mut state, text)
        }
        Msg::PromptLabTemplateUserDraftChanged { text } => {
            prompt_lab::handle_template_user_draft_changed(&mut state, text)
        }
        Msg::PromptLabTemplateApplyRequested => {
            prompt_lab::handle_template_apply_requested(&mut state)
        }
        Msg::PromptLabTemplateApplyAndRerunRequested => {
            prompt_lab::handle_template_apply_and_rerun_requested(&mut state)
        }
        Msg::PromptLabTemplateRevertRequested => {
            prompt_lab::handle_template_revert_requested(&mut state)
        }
        Msg::PromptLabTemplateSaveRequested => {
            prompt_lab::handle_template_save_requested(&mut state)
        }
        Msg::PromptLabTemplateSaved {
            prompt_id,
            version,
            path,
        } => prompt_lab::handle_template_saved(&mut state, prompt_id, version, path),
        Msg::PromptLabTemplateSaveFailed { prompt_id, reason } => {
            prompt_lab::handle_template_save_failed(&mut state, prompt_id, reason)
        }
        Msg::PromptLabRunRequested => prompt_lab::handle_run_requested(&mut state),
        Msg::PromptLabRerunRequested => prompt_lab::handle_rerun_requested(&mut state),
        Msg::PromptLabHistoryCleared => prompt_lab::handle_history_cleared(&mut state),
        Msg::PromptLabCompareDraftReset => prompt_lab::handle_compare_draft_reset(&mut state),
        Msg::PromptLabCompareCurrentSettingsCaptured => {
            prompt_lab::handle_compare_current_settings_captured(&mut state)
        }
        Msg::PromptLabCompareBaselineCaptured => {
            prompt_lab::handle_compare_baseline_captured(&mut state)
        }
        Msg::PromptLabCompareCandidateRemoved { candidate_id } => {
            prompt_lab::handle_compare_candidate_removed(&mut state, candidate_id)
        }
        Msg::PromptLabCompareCandidateLabelChanged {
            candidate_id,
            label,
        } => prompt_lab::handle_compare_candidate_label_changed(&mut state, candidate_id, label),
        Msg::PromptLabCompareBatchStartRequested | Msg::PromptLabCompareBatchConfirmedStart => {
            prompt_lab::handle_compare_batch_start_requested(&mut state)
        }
        Msg::PromptLabCompareBatchCancelRequested => {
            prompt_lab::handle_compare_batch_cancel_requested(&mut state)
        }
        Msg::PromptLabCompareWinnerSelected { run_id } => {
            prompt_lab::handle_compare_winner_selected(&mut state, run_id)
        }
        Msg::PromptLabCompareWinnerCleared => prompt_lab::handle_compare_winner_cleared(&mut state),
        Msg::PromptLabCompareRunRated { run_id, rating } => {
            prompt_lab::handle_compare_run_rated(&mut state, run_id, rating)
        }
        Msg::PromptLabComparePolicyUpdated {
            require_parse_ok,
            max_cost_microdollars,
            max_wall_ms,
            rating_beats_cost,
        } => prompt_lab::handle_compare_policy_updated(
            &mut state,
            require_parse_ok,
            max_cost_microdollars,
            max_wall_ms,
            rating_beats_cost,
        ),
        Msg::PromptLabCompareAutoSelectRequested => {
            prompt_lab::handle_compare_auto_select_requested(&mut state)
        }
        Msg::PromptLabCompareBatchSetWarning { batch_id, warning } => {
            prompt_lab::handle_compare_batch_set_warning(&mut state, batch_id, warning)
        }
        Msg::EntityIndexLoaded { index } => {
            engine_info!("[entity-index] loaded {} entries", index.entries.len());
            state.set_entity_index(index, 13, 10);
            Vec::new()
        }
        Msg::EntityIndexLoadFailed { reason } => {
            engine_warn!("[entity-index] load failed: {reason}; triggering rebuild");
            vec![Effect::RebuildEntityIndex]
        }
        Msg::EntityIndexRebuilt { index } => {
            engine_info!("[entity-index] rebuilt {} entries", index.entries.len());
            state.set_entity_index(index, 13, 10);
            Vec::new()
        }
        Msg::EntityIndexRebuildFailed { reason } => {
            engine_warn!("[entity-index] rebuild failed: {reason}");
            Vec::new()
        }

        // --- Import saved webpages ---
        Msg::ImportSavedWebpagesRequested { dir } => {
            import::handle_import_requested(&mut state, dir)
        }
        Msg::ImportSavedWebpagesCompleted { request_id, report } => {
            import::handle_import_completed(&mut state, request_id, report)
        }
        Msg::ImportSavedWebpagesFailed { request_id, reason } => {
            import::handle_import_failed(&mut state, request_id, reason)
        }
        Msg::ImportedCorpusCleared => import::handle_corpus_cleared(&mut state),

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
        Msg::NoOp => Vec::new(),
    };

    pipeline_run::record_progress_after(&mut state, progress_before);
    (state, effects)
}

#[cfg(test)]
mod desktop_contract_tests {
    use chrono::{DateTime, Utc};

    use super::update;
    use crate::{AppState, Effect, Msg, WorkspaceView};

    #[test]
    fn trends_view_opened_loads_the_entity_index() {
        let (state, effects) = update(AppState::default(), Msg::TrendsViewOpened);
        assert_eq!(state.workspace_view(), WorkspaceView::Trends);
        assert_eq!(effects, vec![Effect::LoadEntityIndex]);
    }

    #[test]
    fn jobs_search_reveal_changes_only_the_desktop_workspace() {
        let initial = AppState::default();
        let (state, effects) = update(initial, Msg::JobsSearchRevealRequested);
        assert_eq!(state.workspace_view(), WorkspaceView::Review);
        assert_eq!(state.job_list_mode(), crate::JobListMode::SinceCheckpoint);
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
                content_preview: None,
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
