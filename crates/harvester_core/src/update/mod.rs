use engine_logging::{engine_error, engine_info, engine_warn};
use std::borrow::ToOwned;

use crate::tabs::{AppTab, JobListScope, LeftTab};
use crate::{
    briefing::{BriefingPhase, BriefingSession, CorpusFingerprint},
    calc_left_width, context_hash,
    prompt_lab::{PromptLabCompareBatchStatus, PromptLabRunStatus, PromptLabStage},
    AppState, Effect, Msg, SessionState, StopPolicy, SummaryCacheKey, SummaryCacheKeyError,
    INPUT_PANEL_FIXED_WIDTH, MIN_JOBS_PANEL_WIDTH,
};
use harvester_engine::llm::prompt::{PromptId, PromptVersion};
use harvester_engine::llm::types::ModelId;

mod archive;
mod briefing;
mod import;
mod llm_completed;
mod polling;
mod prompt_lab;
mod triage;

// Left side is split into a fixed-width input panel plus a resizable jobs panel.
// Minimum width for the left region (PANEL_INPUT + PANEL_JOBS).
const MIN_LEFT_WIDTH: i32 = INPUT_PANEL_FIXED_WIDTH + MIN_JOBS_PANEL_WIDTH;
// Minimum width for the preview panel
const MIN_PREVIEW_WIDTH: i32 = 200;
// Total width occupied by splitter (width + margins)
const SPLITTER_TOTAL_WIDTH: i32 = 16; // 4px bar + 6px margin each side

/// Pure update function: applies a message to state and returns any effects.
pub fn update(mut state: AppState, msg: Msg) -> (AppState, Vec<Effect>) {
    let effects = match msg {
        Msg::InputChanged(text) => {
            state.set_input_buffer(text);
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
            let urls = parse_urls(&raw);
            if urls.is_empty() {
                return (state, Vec::new());
            }
            if matches!(
                state.session(),
                SessionState::Finishing | SessionState::Finished
            ) {
                return (state, Vec::new());
            }

            let ingest = state.ingest_urls(urls);
            state.set_last_paste_stats(ingest.enqueued, ingest.skipped);
            if ingest.enqueued > 0 {
                state.clear_input_buffer();
            }
            ingest.effects
        }
        Msg::StopFinishClicked => {
            if state.session() == SessionState::Running {
                state.finish_session();
                vec![Effect::StopFinish {
                    policy: StopPolicy::Finish,
                }]
            } else {
                Vec::new()
            }
        }
        Msg::ArchiveClicked => archive::handle_archive_clicked(&mut state),
        Msg::ToggleInputPanel => {
            let opening = !state.input_panel_visible();
            let desired_left_width_px = if opening {
                state.left_panel_width() + INPUT_PANEL_FIXED_WIDTH
            } else {
                state.left_panel_width() - INPUT_PANEL_FIXED_WIDTH
            };
            state.set_input_panel_visible(opening);
            let min_left = if opening {
                MIN_LEFT_WIDTH
            } else {
                MIN_JOBS_PANEL_WIDTH
            };
            let clamped = calc_left_width(
                desired_left_width_px,
                state.window_width(),
                min_left,
                MIN_PREVIEW_WIDTH,
                SPLITTER_TOTAL_WIDTH,
            );
            state.set_left_panel_width(clamped);
            state.mark_dirty();
            Vec::new()
        }
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
            let selected_tab = if state.selected_job_has_summary() {
                crate::tabs::AppTab::Summary
            } else {
                crate::tabs::AppTab::Triage
            };
            state.select_tab(selected_tab);
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
        Msg::SplitterMoved {
            desired_left_width_px,
        } => {
            let clamped = calc_left_width(
                desired_left_width_px,
                state.window_width(),
                MIN_LEFT_WIDTH,
                MIN_PREVIEW_WIDTH,
                SPLITTER_TOTAL_WIDTH,
            );
            state.set_left_panel_width(clamped);
            state.mark_dirty();
            Vec::new()
        }
        Msg::WindowResized { window_width } => {
            state.set_window_width(window_width);
            // Re-clamp the left panel width based on new window width
            let clamped = calc_left_width(
                state.left_panel_width(),
                window_width,
                MIN_LEFT_WIDTH,
                MIN_PREVIEW_WIDTH,
                SPLITTER_TOTAL_WIDTH,
            );
            state.set_left_panel_width(clamped);
            state.mark_dirty();
            Vec::new()
        }
        Msg::WindowResizeCompleted {
            outer_width,
            outer_height,
        } => {
            vec![Effect::PersistWindowSize {
                width: outer_width,
                height: outer_height,
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
        Msg::GenerateBriefingClicked => briefing::handle_generate_clicked(&mut state),
        Msg::PrepareSummariesClicked => briefing::handle_prepare_summaries_clicked(&mut state),
        Msg::BriefingPrereqArticlesLoaded { articles } => {
            briefing::handle_prereq_articles_loaded(&mut state, articles)
        }
        Msg::BriefingPrereqLoadFailed { reason } => {
            briefing::handle_prereq_load_failed(&mut state, reason)
        }
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
        } => archive::handle_dialog_ready(
            &mut state,
            request_id,
            article_count,
            since_utc,
            default_basename,
            default_file_exists,
            export_dir,
            pending_pre_triage_count,
        ),
        Msg::ArchiveDialogSubmitted {
            request_id,
            basename,
            set_checkpoint,
            submitted_at,
        } => archive::handle_dialog_submitted(
            &mut state,
            request_id,
            basename,
            set_checkpoint,
            submitted_at,
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
            Vec::new()
        }
        Msg::PromptContextsLoadFailed { reason } => {
            engine_warn!("[PromptContext] Failed to load contexts: {}", reason);
            // Continue with empty contexts (degraded but functional)
            state.mark_triage_metadata_ready();
            state.mark_dirty();
            Vec::new()
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
        Msg::PollSourcesClicked => polling::handle_poll_sources_clicked(&mut state),
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
        Msg::TabSelected { tab } => {
            state.select_tab(tab);
            if tab == AppTab::Trends {
                // Stub in Slice 1; full entity index loading in Slice 3.
                vec![Effect::LoadEntityIndex]
            } else {
                Vec::new()
            }
        }
        Msg::LeftTabSelected { tab } => {
            engine_info!("[jobs-ui] left tab selected: {:?}", tab);
            if tab == LeftTab::PromptLab {
                state.open_prompt_lab();
            } else {
                state.close_prompt_lab_internals();
                state.set_left_tab(tab);
            }
            Vec::new()
        }
        Msg::TrendCategorySelected { category } => {
            state.set_active_trend_category(category);
            Vec::new()
        }
        Msg::PromptLabOpenRequested => prompt_lab::handle_open_requested(&mut state),
        Msg::PromptLabCloseRequested => prompt_lab::handle_close_requested(&mut state),
        Msg::JobListScopeSet { scope } => {
            engine_info!(
                "[jobs-ui] scope set: {}",
                match scope {
                    JobListScope::All => "all",
                    JobListScope::SinceCheckpoint => "since-checkpoint",
                }
            );
            state.set_job_list_scope(scope);
            Vec::new()
        }
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

        Msg::Tick => {
            state.advance_tick();
            let tick = state.current_tick();
            let has_in_flight_jobs = state.batch_observation().jobs_in_flight > 0;
            triage::dispatch_pre_triage_if_due(&mut state, tick, has_in_flight_jobs)
        }
        Msg::NoOp => Vec::new(),
    };

    (state, effects)
}

fn parse_urls(raw: &str) -> Vec<String> {
    raw.lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .map(ToOwned::to_owned)
        .collect()
}

fn log_summary_cache_warmup_if_needed(state: &mut AppState) {
    if state.summary_cache_warmup_logged() {
        return;
    }
    let (version_display, model_display, reason_label) = match state.summary_cache_metadata() {
        Some((version, model)) => (version.to_string(), model.to_string(), "metadata-loaded"),
        None => (
            "<none>".to_string(),
            "<none>".to_string(),
            "missing-configured-model",
        ),
    };
    engine_info!(
        "[summary-cache] run warmup decision=run-start reason={} prompt_version={} model_id={}",
        reason_label,
        version_display,
        model_display
    );
    state.mark_summary_cache_warmup_logged();
}

fn log_summary_cache_run_summary(state: &mut AppState) {
    let metrics = state.summary_cache_metrics();
    engine_info!(
        "[summary-cache] run summary hits={} misses={} key_unavailable={} total={}",
        metrics.hits(),
        metrics.misses(),
        metrics.key_unavailable(),
        metrics.total()
    );
    state.finalize_summary_cache_run();
}

fn summary_cache_key_error_reason(error: &SummaryCacheKeyError) -> &'static str {
    match error {
        SummaryCacheKeyError::MissingPromptVersion => "missing prompt_version metadata",
        SummaryCacheKeyError::MissingModelId => "missing model_id metadata",
        SummaryCacheKeyError::EmptyContentHash => "empty content_hash",
    }
}

fn build_summary_cache_key(
    content_hash: &str,
    prompt_id: PromptId,
    prompt_version: Option<PromptVersion>,
    model_id: Option<&str>,
    context: &[(String, String)],
) -> Result<SummaryCacheKey, SummaryCacheKeyError> {
    SummaryCacheKey::try_new(content_hash, prompt_id, prompt_version, model_id, context)
}

fn short_hash(hash: &str) -> &str {
    let end = hash.len().min(8);
    &hash[..end]
}

fn log_summary_cache_lookup_mismatch(
    article_idx: usize,
    lookup: &SummaryCacheKey,
    store_key: &SummaryCacheKey,
) {
    if lookup == store_key {
        return;
    }
    engine_warn!(
        "[summary-cache] metadata mismatch article={} lookup=(version={},model={},context={}) store=(version={},model={},context={})",
        article_idx,
        lookup.prompt_version,
        lookup.model_id,
        lookup.context_hash,
        store_key.prompt_version,
        store_key.model_id,
        store_key.context_hash,
    );
}

fn log_summary_cache_completion_metadata(
    article_idx: usize,
    store_key: &SummaryCacheKey,
    completion_key: &SummaryCacheKey,
) {
    if completion_key == store_key {
        return;
    }
    if summary_cache_model_ids_compatible(&store_key.model_id, &completion_key.model_id) {
        engine_info!(
            "[summary-cache] completion metadata differs by model variant article={} cache_model={} completion_model={}",
            article_idx,
            store_key.model_id,
            completion_key.model_id,
        );
    } else {
        engine_warn!(
            "[summary-cache] completion metadata mismatch article={} cache=(version={},model={},context={}) completion=(version={},model={},context={})",
            article_idx,
            store_key.prompt_version,
            store_key.model_id,
            store_key.context_hash,
            completion_key.prompt_version,
            completion_key.model_id,
            completion_key.context_hash,
        );
    }
}

fn summary_cache_model_ids_compatible(store_model_id: &str, completion_model_id: &str) -> bool {
    if store_model_id == completion_model_id {
        return true;
    }
    completion_model_id.starts_with(store_model_id)
        && completion_model_id
            .as_bytes()
            .get(store_model_id.len())
            .is_some_and(|b| *b == b'-')
}

#[cfg(test)]
mod tests;
