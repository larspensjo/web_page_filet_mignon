use engine_logging::{engine_error, engine_info, engine_warn};
use std::borrow::ToOwned;
use std::path::PathBuf;

use crate::state::TriageCacheLookupResult;
use crate::{
    briefing::{
        ArticleSummaryResult, BriefingPhase, BriefingResult, BriefingSession, BriefingThemeResult,
        CorpusFingerprint,
    },
    calc_left_width, context_hash,
    pre_triage_filter::{PreTriagePhase, PreTriagePolicy, PreTriageSession},
    prompt_lab::{
        prompt_id_for_stage, PromptLabCompareBatchStatus, PromptLabRunStatus, PromptLabStage,
    },
    triage::{ArticleTriageResult, TriagePhase, TriageSession},
    AppState, Effect, LlmRequestState, LlmResultKind, Msg, SessionState, StopPolicy,
    SummaryCacheKey, SummaryCacheKeyError, INPUT_PANEL_FIXED_WIDTH, MIN_JOBS_PANEL_WIDTH,
};
use harvester_engine::llm::prompt::{PromptId, PromptVersion};
use harvester_engine::llm::types::ModelId;
use harvester_engine::llm::{
    validate_briefing, validate_summary, validate_template, validate_triage,
};

// Left side is split into a fixed-width input panel plus a resizable jobs panel.
// Minimum width for the left region (PANEL_INPUT + PANEL_JOBS).
const MIN_LEFT_WIDTH: i32 = INPUT_PANEL_FIXED_WIDTH + MIN_JOBS_PANEL_WIDTH;
// Minimum width for the preview panel
const MIN_PREVIEW_WIDTH: i32 = 200;
// Total width occupied by splitter (width + margins)
const SPLITTER_TOTAL_WIDTH: i32 = 16; // 4px bar + 6px margin each side

/// Pure update function: applies a message to state and returns any effects.
#[allow(
    clippy::too_many_lines,
    clippy::cognitive_complexity,
    clippy::excessive_nesting
)]
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
        Msg::ArchiveClicked => vec![Effect::ArchiveRequested],
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
        } => {
            state.apply_done(job_id, result, content_preview, extracted_links);
            refresh_pre_triage_if_needed(&mut state)
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
        Msg::RestoreCompletedJobs(entries) => {
            state.restore_completed_jobs(entries);
            refresh_pre_triage_if_needed(&mut state)
        }
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
            }]
        }
        Msg::LlmCompleted {
            request_id,
            result,
            metadata,
        } => {
            let new_state = match &result {
                LlmResultKind::Success {
                    output_json,
                    input_tokens,
                    output_tokens,
                    ..
                } => LlmRequestState::Completed {
                    output_json: output_json.clone(),
                    input_tokens: *input_tokens,
                    output_tokens: *output_tokens,
                },
                LlmResultKind::ValidationFailed {
                    reason,
                    raw_response,
                } => LlmRequestState::Failed {
                    reason: format!("validation failed: {reason}; response: {raw_response}"),
                },
                LlmResultKind::QuotaExhausted { reason } => LlmRequestState::Failed {
                    reason: reason.clone(),
                },
                LlmResultKind::Failed { reason } => LlmRequestState::Failed {
                    reason: reason.clone(),
                },
            };
            if state.llm_request_state(request_id).is_some() {
                state.record_llm_result(request_id, new_state);
            } else {
                engine_warn!("LLM completion for unknown request_id {request_id}");
            }
            if let Some(m) = metadata.as_ref() {
                state.record_llm_usage_from_metadata(m);
            }
            let mut effects = Vec::new();
            if let Some(article_idx) = state.briefing().find_article_by_request_id(request_id) {
                match &result {
                    LlmResultKind::Success {
                        output_json,
                        input_tokens,
                        output_tokens,
                        prompt_version,
                        model_id,
                    } => match validate_summary(output_json) {
                        Ok(summary) => {
                            let summary_result = ArticleSummaryResult {
                                title: summary.title,
                                summary: summary.summary,
                                key_points: summary.key_points,
                                input_tokens: *input_tokens,
                                output_tokens: *output_tokens,
                            };

                            // Clone data needed for cache key before mutable operations
                            let content_hash = state.briefing().articles()[article_idx]
                                .content_hash
                                .clone();
                            let context = state.context_for(PromptId::ArticleSummary).to_vec();
                            let lookup_key =
                                state.briefing().article_cache_key(article_idx).cloned();
                            let run_metadata = state
                                .summary_cache_metadata()
                                .map(|(version, model)| (version, model.to_string()));

                            // Complete the article
                            state
                                .briefing_mut()
                                .complete_article(article_idx, summary_result.clone());

                            // Refresh preview if this article is currently selected
                            state.refresh_selected_preview();

                            let cache_key_result = match lookup_key.clone() {
                                Some(key) => Ok(key),
                                None => build_summary_cache_key(
                                    &content_hash,
                                    PromptId::ArticleSummary,
                                    run_metadata.as_ref().map(|(version, _)| *version),
                                    run_metadata.as_ref().map(|(_, model)| model.as_str()),
                                    &context,
                                ),
                            };
                            match cache_key_result {
                                Ok(store_key) => {
                                    if let Some(lookup) = lookup_key.as_ref() {
                                        log_summary_cache_lookup_mismatch(
                                            article_idx,
                                            lookup,
                                            &store_key,
                                        );
                                    }

                                    let completion_key = build_summary_cache_key(
                                        &content_hash,
                                        PromptId::ArticleSummary,
                                        Some(*prompt_version),
                                        Some(model_id.as_str()),
                                        &context,
                                    );
                                    if let Ok(completion_key) = completion_key {
                                        log_summary_cache_completion_metadata(
                                            article_idx,
                                            &store_key,
                                            &completion_key,
                                        );
                                    }

                                    state.store_summary_result(
                                        store_key.clone(),
                                        summary_result,
                                        chrono::Utc::now().to_rfc3339(),
                                    );
                                    let lookup_label = if lookup_key.is_some() {
                                        "metadata-snapshot"
                                    } else {
                                        "none"
                                    };
                                    engine_info!(
                                        "[summary-cache] article={} decision=store metadata_source=run-frozen lookup_metadata={} prompt_version={} model_id={} context_hash={} content_hash_short={}",
                                        article_idx,
                                        lookup_label,
                                        store_key.prompt_version,
                                        store_key.model_id,
                                        store_key.context_hash,
                                        short_hash(&content_hash),
                                    );
                                }
                                Err(err) => {
                                    engine_warn!(
                                        "[summary-cache] article={} skip storing result: {}",
                                        article_idx,
                                        summary_cache_key_error_reason(&err)
                                    );
                                }
                            }
                            state
                                .briefing_mut()
                                .set_article_cache_key(article_idx, None);
                        }
                        Err(err) => {
                            state
                                .briefing_mut()
                                .fail_article(article_idx, format!("validation failed: {err}"));
                        }
                    },
                    LlmResultKind::QuotaExhausted { reason } => {
                        engine_info!("[briefing] quota exhausted during summaries: {reason}");
                        state
                            .briefing_mut()
                            .fail_article(article_idx, reason.clone());
                        state.briefing_mut().fail_all_pending("quota exhausted");
                    }
                    LlmResultKind::ValidationFailed { reason, .. }
                    | LlmResultKind::Failed { reason } => {
                        state
                            .briefing_mut()
                            .fail_article(article_idx, reason.clone());
                    }
                }
                dispatch_next_briefing_step(&mut state, &mut effects);
            } else if let Some(article_idx) = state.triage().find_article_by_request_id(request_id)
            {
                match &result {
                    LlmResultKind::Success {
                        output_json,
                        input_tokens,
                        output_tokens,
                        ..
                    } => match validate_triage(output_json) {
                        Ok(triage) => {
                            let content_hash =
                                state.triage().articles()[article_idx].content_hash.clone();
                            let result = ArticleTriageResult {
                                category: triage.category,
                                priority: triage.priority.value(),
                                tags: triage.tags,
                                rationale: triage.rationale,
                                input_tokens: *input_tokens,
                                output_tokens: *output_tokens,
                            };
                            state
                                .triage_mut()
                                .complete_article(article_idx, result.clone());
                            state.store_triage_result(&content_hash, result);

                            // Refresh preview if this article is currently selected
                            state.refresh_selected_preview();
                        }
                        Err(err) => {
                            state
                                .triage_mut()
                                .fail_article(article_idx, format!("validation: {err}"));
                        }
                    },
                    LlmResultKind::QuotaExhausted { reason } => {
                        state.triage_mut().fail_article(article_idx, reason.clone());
                        state.triage_mut().fail_all_pending("quota exhausted");
                    }
                    LlmResultKind::ValidationFailed { reason, .. }
                    | LlmResultKind::Failed { reason } => {
                        state.triage_mut().fail_article(article_idx, reason.clone());
                    }
                }
                dispatch_next_triage_step(&mut state, &mut effects);
            } else if state.briefing().is_briefing_request(request_id) {
                match &result {
                    LlmResultKind::Success {
                        output_json,
                        input_tokens,
                        output_tokens,
                        ..
                    } => match validate_briefing(output_json) {
                        Ok(briefing) => {
                            let themes = briefing
                                .themes
                                .into_iter()
                                .map(|theme| BriefingThemeResult {
                                    name: theme.name,
                                    description: theme.description,
                                })
                                .collect();
                            state.briefing_mut().complete_briefing(BriefingResult {
                                executive_summary: briefing.executive_summary,
                                themes,
                                article_count: briefing.article_count,
                                input_tokens: *input_tokens,
                                output_tokens: *output_tokens,
                            });
                            state.revert_preview_to_briefing();
                            effects.push(Effect::PersistSummaryCache {
                                cache: state.summary_cache().clone(),
                            });
                        }
                        Err(err) => {
                            engine_warn!("[briefing] briefing validation failed: {err}");
                            state.briefing_mut().complete_without_briefing();
                            state.revert_preview_to_briefing();
                            effects.push(Effect::PersistSummaryCache {
                                cache: state.summary_cache().clone(),
                            });
                        }
                    },
                    _ => {
                        state.briefing_mut().complete_without_briefing();
                        state.revert_preview_to_briefing();
                        effects.push(Effect::PersistSummaryCache {
                            cache: state.summary_cache().clone(),
                        });
                    }
                }
                log_summary_cache_run_summary(&mut state);
                state.mark_dirty();
            } else if let Some(run_id) = state.prompt_lab().ownership_for(request_id) {
                let reason_from_result = |r: &LlmResultKind| -> String {
                    match r {
                        LlmResultKind::ValidationFailed {
                            reason,
                            raw_response,
                        } => {
                            format!("validation failed: {reason}; response: {raw_response}")
                        }
                        LlmResultKind::QuotaExhausted { reason } => {
                            format!("quota exhausted: {reason}")
                        }
                        LlmResultKind::Failed { reason } => reason.clone(),
                        LlmResultKind::Success { .. } => String::new(),
                    }
                };
                let metadata_for_failure = metadata.clone();
                match &result {
                    LlmResultKind::Success {
                        output_json,
                        input_tokens,
                        output_tokens,
                        ..
                    } => {
                        engine_info!(
                            "[prompt-lab] run completed run_id={} request_id={} tokens_in={} tokens_out={}",
                            run_id.0, request_id, input_tokens, output_tokens
                        );
                        // metadata is always Some for a Success result.
                        if let Some(run_metadata) = metadata {
                            state.complete_prompt_lab_run(
                                run_id,
                                output_json.clone(),
                                run_metadata,
                            );
                        } else {
                            engine_warn!(
                                "[prompt-lab] run completed but metadata missing run_id={} request_id={}",
                                run_id.0, request_id
                            );
                            state.fail_prompt_lab_run(run_id, "metadata missing".to_string(), None);
                        }
                    }
                    _ => {
                        let reason = reason_from_result(&result);
                        engine_warn!(
                            "[prompt-lab] run failed run_id={} request_id={} reason={}",
                            run_id.0,
                            request_id,
                            reason
                        );
                        state.fail_prompt_lab_run(run_id, reason, metadata_for_failure);
                    }
                }
                state.consume_prompt_lab_ownership(request_id);
                let compare_batch_id = state
                    .prompt_lab()
                    .run_by_id(run_id)
                    .and_then(|run| run.compare_batch_id);
                if let Some(batch_id) = compare_batch_id {
                    let Some(batch) = state
                        .prompt_lab()
                        .batches()
                        .iter()
                        .find(|batch| batch.batch_id == batch_id)
                        .cloned()
                    else {
                        state.mark_dirty();
                        return (state, effects);
                    };
                    let all_dispatched = batch.pending_candidate_count() == 0;
                    let all_terminal = batch
                        .candidate_run_ids
                        .iter()
                        .filter_map(|(_, maybe_run)| *maybe_run)
                        .all(|candidate_run_id| {
                            state
                                .prompt_lab()
                                .run_by_id(candidate_run_id)
                                .map(|run| {
                                    !matches!(run.status, PromptLabRunStatus::Pending { .. })
                                })
                                .unwrap_or(false)
                        });
                    if all_dispatched && all_terminal {
                        let has_failed = batch
                            .candidate_run_ids
                            .iter()
                            .filter_map(|(_, maybe_run)| *maybe_run)
                            .any(|candidate_run_id| {
                                state
                                    .prompt_lab()
                                    .run_by_id(candidate_run_id)
                                    .map(|run| {
                                        matches!(run.status, PromptLabRunStatus::Failed { .. })
                                    })
                                    .unwrap_or(false)
                            });
                        let final_status = if has_failed {
                            PromptLabCompareBatchStatus::PartialFailure
                        } else {
                            PromptLabCompareBatchStatus::AllComplete
                        };
                        state
                            .prompt_lab_mut()
                            .set_batch_status(batch_id, final_status);
                        state
                            .prompt_lab_mut()
                            .recompute_auto_select_for_batch(batch_id);
                        state.prompt_lab_mut().clear_active_batch_if(batch_id);
                    } else {
                        effects.extend(dispatch_next_compare_candidate(&mut state, batch_id));
                    }
                }
                state.mark_dirty();
            }
            effects
        }
        Msg::GenerateBriefingClicked => {
            if !state.briefing().can_start() {
                return (state, Vec::new());
            }
            if state.triage().is_active() {
                engine_info!("[briefing-triage] interleave blocked: triage in progress");
                return (state, Vec::new());
            }
            state.request_briefing_orchestration();
            state.set_briefing(BriefingSession::new_waiting_for_triage(None));
            state.revert_preview_to_briefing();
            engine_info!("[briefing-triage] generate requested");
            let ordered_urls = state.ordered_completed_job_urls();
            vec![
                Effect::LoadPromptContexts,
                Effect::LoadPromptTemplateFiles,
                Effect::LoadLlmMetadata,
                Effect::LoadArticlesForBriefingPrereq { ordered_urls },
            ]
        }
        Msg::PrepareSummariesClicked => {
            if !state.briefing().can_start() {
                return (state, Vec::new());
            }
            if state.triage().is_active() {
                engine_info!("[briefing-triage] summary-prep blocked: triage in progress");
                return (state, Vec::new());
            }
            state.request_summary_preparation();
            state.set_briefing(BriefingSession::new_waiting_for_triage(None));
            state.revert_preview_to_briefing();
            engine_info!("[briefing-triage] summary-prep requested");
            let ordered_urls = state.ordered_completed_job_urls();
            vec![
                Effect::LoadPromptContexts,
                Effect::LoadPromptTemplateFiles,
                Effect::LoadLlmMetadata,
                Effect::LoadArticlesForBriefingPrereq { ordered_urls },
            ]
        }
        Msg::BriefingPrereqArticlesLoaded { articles } => {
            engine_info!("[briefing-triage] prereq loaded count={}", articles.len());
            let policy = PreTriagePolicy::default();
            let pre_triage = PreTriageSession::load_articles(articles, &policy);
            let filtered_articles = pre_triage.resolved_included_articles();
            if filtered_articles.is_empty() {
                state
                    .briefing_mut()
                    .fail("No articles available after pre-triage filters".to_string());
                state.clear_briefing_orchestration();
                state.mark_dirty();
                return (state, Vec::new());
            }
            let prereq_fingerprint = CorpusFingerprint::from_articles(&filtered_articles);
            state.store_briefing_prereq_articles(filtered_articles.clone());
            let triage_reusable = matches!(state.triage().phase(), TriagePhase::Complete)
                && CorpusFingerprint::from_triage_results(state.triage()) == prereq_fingerprint;
            let mut effects = Vec::new();
            if triage_reusable {
                engine_info!("[briefing-triage] triage reused");
                on_triage_settled_for_briefing(&mut state, &mut effects);
            } else {
                engine_info!("[briefing-triage] triage rerun");
                state.triage_mut().reset_with_articles(filtered_articles);
                state.triage_mut().transition_to_triaging();
                state.start_triage_cache_run();
                state.mark_triage_metadata_ready();
                dispatch_next_triage_step(&mut state, &mut effects);
            }
            effects
        }
        Msg::BriefingPrereqLoadFailed { reason } => {
            engine_warn!("[briefing-triage] prereq load failed reason={}", reason);
            state.briefing_mut().fail(reason);
            state.clear_briefing_orchestration();
            state.mark_dirty();
            Vec::new()
        }
        Msg::ArticlesLoaded {
            articles,
            collection_text,
        } => {
            if articles.is_empty() {
                state
                    .briefing_mut()
                    .fail("no completed articles found".to_string());
                log_summary_cache_run_summary(&mut state);
                state.mark_dirty();
                let cache = state.summary_cache().clone();
                return (state, vec![Effect::PersistSummaryCache { cache }]);
            }
            state.briefing_mut().set_articles(articles, collection_text);
            state.briefing_mut().transition_to_summarizing();
            state.mark_dirty();
            let mut effects = Vec::new();
            try_start_briefing_with_metadata(&mut state, &mut effects);
            effects
        }
        Msg::ArticlesLoadFailed { reason } => {
            state.briefing_mut().fail(reason);
            log_summary_cache_run_summary(&mut state);
            state.mark_dirty();
            let cache = state.summary_cache().clone();
            vec![Effect::PersistSummaryCache { cache }]
        }
        Msg::TriageClicked => {
            if state.briefing_orchestration_requested() {
                engine_info!("[briefing-triage] interleave blocked: briefing owns triage");
                return (state, Vec::new());
            }
            if !state.triage().can_start() {
                return (state, Vec::new());
            }
            if !matches!(state.pre_triage().phase(), PreTriagePhase::ReadyToTriage) {
                return (state, Vec::new());
            }
            if !state.triage_metadata_ready() {
                state.mark_triage_metadata_pending();
                engine_warn!("[triage-cache] metadata not ready; loading metadata before dispatch");
                return (
                    state,
                    vec![Effect::LoadPromptContexts, Effect::LoadLlmMetadata],
                );
            }
            engine_info!("[triage] triage requested");
            start_triage_from_pretriage(&mut state)
        }
        Msg::TriageArticlesLoaded { articles } => {
            let policy = PreTriagePolicy::default();
            let mut pre_triage = PreTriageSession::load_articles(articles, &policy);
            let job_url_pairs = state
                .view()
                .jobs
                .iter()
                .map(|job| (job.job_id, job.url.clone()))
                .collect::<Vec<_>>();
            pre_triage.bind_job_ids(&job_url_pairs);
            pre_triage.apply_manual_overrides(state.pre_triage_manual_overrides());
            state.set_pre_triage(pre_triage);
            state.refresh_selected_preview();
            state.mark_dirty();
            Vec::new()
        }
        Msg::TriageArticlesLoadFailed { reason } => {
            state.set_pre_triage(PreTriageSession::default());
            state.clear_pre_triage_manual_overrides();
            state.triage_mut().fail(reason);
            state.mark_dirty();
            Vec::new()
        }
        Msg::PreTriageDecisionSet { key, decision } => {
            if state.set_pre_triage_manual_decision(key, decision) {
                state.mark_dirty();
            }
            Vec::new()
        }
        Msg::PreTriageApplyClicked => Vec::new(),
        Msg::PreTriageResetClicked => {
            state.clear_pre_triage_manual_overrides();
            state.mark_dirty();
            Vec::new()
        }
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
            state.mark_briefing_metadata_ready();
            state.mark_triage_metadata_ready();
            state.mark_dirty();
            let mut effects = Vec::new();
            try_start_briefing_with_metadata(&mut state, &mut effects);
            effects
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
        Msg::PollSourcesClicked => {
            if matches!(
                state.session(),
                SessionState::Finishing | SessionState::Finished
            ) || state.is_poll_in_progress()
            {
                Vec::new()
            } else if state.start_poll() {
                engine_info!("[source-poll] polling requested");
                vec![Effect::PollAllSources]
            } else {
                Vec::new()
            }
        }
        Msg::SourcePollCompleted { source_id, urls } => {
            engine_info!("[source-poll] {} returned {} urls", source_id, urls.len());
            state.record_source_poll(&source_id, urls.len());
            let ingest = state.ingest_urls(urls);
            ingest.effects
        }
        Msg::SourcePollFailed { source_id, error } => {
            engine_warn!("[source-poll] {} failed: {}", source_id, error);
            state.record_source_error(&source_id, error);
            Vec::new()
        }
        Msg::AllSourcesPollEnded => {
            state.end_poll();
            Vec::new()
        }
        Msg::PromptLabOpenRequested => {
            state.open_prompt_lab();
            Vec::new()
        }
        Msg::PromptLabCloseRequested => {
            state.close_prompt_lab();
            Vec::new()
        }
        Msg::PromptLabStageSelected { stage } => {
            state.select_prompt_lab_stage(stage);
            Vec::new()
        }
        Msg::PromptLabInputSourceSelected { source } => {
            state.prompt_lab_mut().select_input_source(source);
            state.mark_dirty();
            Vec::new()
        }
        Msg::PromptLabInputChanged { text } => {
            state.set_prompt_lab_input(text);
            state.mark_dirty();
            Vec::new()
        }
        Msg::PromptLabAdvancedModeSet { enabled } => {
            state.prompt_lab_mut().set_advanced_mode(enabled);
            state.mark_dirty();
            Vec::new()
        }
        Msg::PromptLabModelCatalogLoaded { models, source } => {
            let sample = models
                .iter()
                .take(5)
                .map(|m| m.model_name().to_string())
                .collect::<Vec<_>>()
                .join(", ");
            engine_info!(
                "[prompt-lab-model] reducer received catalog source={:?} count={} sample=[{}]",
                source,
                models.len(),
                sample
            );
            state.prompt_lab_mut().set_model_catalog(models, source);
            state.mark_dirty();
            Vec::new()
        }
        Msg::PromptLabModelOverrideSet { model } => {
            state.prompt_lab_mut().set_model_override_checked(model);
            state.mark_dirty();
            Vec::new()
        }
        Msg::PromptLabCompareSectionToggled => {
            state.prompt_lab_mut().toggle_compare_section();
            state.mark_dirty();
            Vec::new()
        }
        Msg::PromptLabContextSectionToggled => {
            state.prompt_lab_mut().toggle_context_section();
            state.mark_dirty();
            Vec::new()
        }
        Msg::PromptLabTemplateSectionToggled => {
            state.prompt_lab_mut().toggle_template_section();
            state.mark_dirty();
            Vec::new()
        }
        Msg::PromptLabRunDetailsSectionToggled => {
            state.prompt_lab_mut().toggle_run_details_section();
            state.mark_dirty();
            Vec::new()
        }
        Msg::PromptLabUrlInputChanged { url } => {
            state.prompt_lab_mut().set_url_input(url);
            state.mark_dirty();
            Vec::new()
        }
        Msg::PromptLabResolveRequested => {
            let url = state.prompt_lab().url_input().to_owned();
            let has_pending = state.prompt_lab().pending_resolve_id().is_some();
            if url.is_empty() || has_pending {
                return (state, Vec::new());
            }
            let resolve_id = state.allocate_next_prompt_lab_resolve_id();
            state.prompt_lab_mut().begin_url_resolution(resolve_id);
            state.mark_dirty();
            vec![Effect::ResolvePromptLabInputFromUrl { resolve_id, url }]
        }
        Msg::PromptLabInputResolved { resolve_id, result } => {
            if state
                .prompt_lab_mut()
                .finish_url_resolution(resolve_id, result)
            {
                state.mark_dirty();
            }
            Vec::new()
        }
        Msg::PromptLabContextEditorOpened => {
            let stage = state.prompt_lab().selected_stage();
            let prompt_id = prompt_id_for_stage(stage);
            let base_snapshot = state.context_for(prompt_id).to_vec();
            state
                .prompt_lab_mut()
                .initialize_context_draft(prompt_id, &base_snapshot);
            engine_info!(
                "[prompt-lab-context] PromptLabContextEditorOpened prompt_id={:?}",
                prompt_id
            );
            Vec::new()
        }
        Msg::PromptLabContextDraftChanged { text } => {
            let stage = state.prompt_lab().selected_stage();
            let prompt_id = prompt_id_for_stage(stage);
            let base_snapshot = state.context_for(prompt_id).to_vec();
            state
                .prompt_lab_mut()
                .initialize_context_draft(prompt_id, &base_snapshot);
            state
                .prompt_lab_mut()
                .update_context_draft_text(prompt_id, text);
            state.mark_dirty();
            Vec::new()
        }
        Msg::PromptLabContextApplyRequested => {
            let stage = state.prompt_lab().selected_stage();
            let prompt_id = prompt_id_for_stage(stage);
            let base_snapshot = state.context_for(prompt_id).to_vec();
            state
                .prompt_lab_mut()
                .initialize_context_draft(prompt_id, &base_snapshot);
            if state.prompt_lab_mut().apply_context_draft(prompt_id) {
                let count = state
                    .prompt_lab()
                    .applied_context_pairs(prompt_id)
                    .map(|pairs: &[(String, String)]| pairs.len())
                    .unwrap_or(0);
                engine_info!(
                    "[prompt-lab-context] PromptLabContextApplied prompt_id={:?} pair_count={}",
                    prompt_id,
                    count
                );
                state.mark_dirty();
            } else {
                engine_warn!(
                    "[prompt-lab-context] PromptLabContextApplyRequested rejected for {:?}",
                    prompt_id
                );
            }
            Vec::new()
        }
        Msg::PromptLabContextApplyAndRerunRequested => {
            let stage = state.prompt_lab().selected_stage();
            let prompt_id = prompt_id_for_stage(stage);
            let base_snapshot = state.context_for(prompt_id).to_vec();
            state
                .prompt_lab_mut()
                .initialize_context_draft(prompt_id, &base_snapshot);
            if !state.prompt_lab_mut().apply_context_draft(prompt_id) {
                return (state, Vec::new());
            }
            if state.prompt_lab().has_in_flight_run() {
                return (state, Vec::new());
            }
            let snapshot = state
                .prompt_lab()
                .resolved_url_snapshot()
                .map(ToOwned::to_owned);
            let input = match snapshot {
                Some(text) => text,
                None => return (state, Vec::new()),
            };
            let prompt_version = state
                .prompt_lab()
                .selected_prompt_version()
                .or_else(|| state.active_version_for(prompt_id));
            let model_override = state.prompt_lab().selected_model_override().cloned();
            state.mark_dirty();
            let effects = dispatch_prompt_lab_run(
                &mut state,
                PromptLabDispatchRequest {
                    stage,
                    prompt_id,
                    input_snapshot: input,
                    prompt_version,
                    model_override,
                    compare_batch_id: None,
                    compare_candidate_id: None,
                },
            );
            return (state, effects);
        }
        Msg::PromptLabContextRevertRequested => {
            let stage = state.prompt_lab().selected_stage();
            let prompt_id = prompt_id_for_stage(stage);
            let base_snapshot = state.context_for(prompt_id).to_vec();
            state
                .prompt_lab_mut()
                .initialize_context_draft(prompt_id, &base_snapshot);
            if state.prompt_lab_mut().revert_context_draft(prompt_id) {
                engine_info!(
                    "[prompt-lab-context] PromptLabContextReverted prompt_id={:?}",
                    prompt_id
                );
                state.mark_dirty();
            }
            Vec::new()
        }
        Msg::PromptLabContextSaveRequested => {
            let stage = state.prompt_lab().selected_stage();
            let prompt_id = prompt_id_for_stage(stage);
            let base_snapshot = state.context_for(prompt_id).to_vec();
            state
                .prompt_lab_mut()
                .initialize_context_draft(prompt_id, &base_snapshot);
            if !state.prompt_lab().can_save_context(prompt_id) {
                engine_warn!(
                    "[prompt-lab-context] PromptLabContextSaveRequested without applied changes for {:?}",
                    prompt_id
                );
                return (state, Vec::new());
            }
            let context_pairs = match state.prompt_lab().applied_context_pairs(prompt_id) {
                Some(pairs) => pairs.to_vec(),
                None => {
                    engine_warn!(
                        "[prompt-lab-context] Save requested but no applied context for {:?}",
                        prompt_id
                    );
                    return (state, Vec::new());
                }
            };
            engine_info!(
                "[prompt-lab-context] PromptLabContextSaveRequested prompt_id={:?} pair_count={}",
                prompt_id,
                context_pairs.len()
            );
            return (
                state,
                vec![Effect::SavePromptContextFile {
                    prompt_id,
                    context_pairs,
                }],
            );
        }
        Msg::PromptLabContextReloadRequested => {
            let stage = state.prompt_lab().selected_stage();
            let prompt_id = prompt_id_for_stage(stage);
            state.prompt_lab_mut().drop_context_draft(prompt_id);
            engine_info!(
                "[prompt-lab-context] PromptLabContextReloadRequested prompt_id={:?}",
                prompt_id
            );
            return (state, vec![Effect::LoadPromptContexts]);
        }
        Msg::PromptLabContextSaved {
            prompt_id,
            path,
            version,
        } => {
            let message = Some(format!("Saved prompt context v{} to {}", version, path));
            state
                .prompt_lab_mut()
                .mark_context_saved(prompt_id, message.clone());
            engine_info!(
                "[prompt-lab-context] PromptLabContextSaved prompt_id={:?} path={} version={}",
                prompt_id,
                path,
                version
            );
            state.mark_dirty();
            Vec::new()
        }
        Msg::PromptLabContextSaveFailed { prompt_id, reason } => {
            let message = Some(format!("Save failed: {}", reason));
            state
                .prompt_lab_mut()
                .set_context_status_message(prompt_id, message.clone());
            engine_error!(
                "[prompt-lab-context] PromptLabContextSaveFailed prompt_id={:?} reason={}",
                prompt_id,
                reason
            );
            Vec::new()
        }
        Msg::PromptLabTemplateEditorOpened => {
            let stage = state.prompt_lab().selected_stage();
            let prompt_id = prompt_id_for_stage(stage);
            state.prompt_lab_mut().set_template_editor_open(true);
            ensure_prompt_lab_template_draft(&mut state, prompt_id);
            state.mark_dirty();
            Vec::new()
        }
        Msg::PromptLabTemplateSystemDraftChanged { text } => {
            let stage = state.prompt_lab().selected_stage();
            let prompt_id = prompt_id_for_stage(stage);
            ensure_prompt_lab_template_draft(&mut state, prompt_id);
            if state
                .prompt_lab_mut()
                .update_template_system(prompt_id, text)
            {
                state.mark_dirty();
            }
            Vec::new()
        }
        Msg::PromptLabTemplateUserDraftChanged { text } => {
            let stage = state.prompt_lab().selected_stage();
            let prompt_id = prompt_id_for_stage(stage);
            ensure_prompt_lab_template_draft(&mut state, prompt_id);
            if state.prompt_lab_mut().update_template_user(prompt_id, text) {
                state.mark_dirty();
            }
            Vec::new()
        }
        Msg::PromptLabTemplateApplyRequested => {
            let prompt_id = prompt_id_for_stage(state.prompt_lab().selected_stage());
            if apply_prompt_lab_template_draft(&mut state, prompt_id) {
                state.mark_dirty();
            }
            Vec::new()
        }
        Msg::PromptLabTemplateApplyAndRerunRequested => {
            let stage = state.prompt_lab().selected_stage();
            let prompt_id = prompt_id_for_stage(stage);
            if !apply_prompt_lab_template_draft(&mut state, prompt_id) {
                return (state, Vec::new());
            }
            if state.prompt_lab().has_in_flight_run() {
                return (state, Vec::new());
            }
            let snapshot = state
                .prompt_lab()
                .resolved_url_snapshot()
                .map(ToOwned::to_owned);
            let input = match snapshot {
                Some(text) => text,
                None => return (state, Vec::new()),
            };
            let prompt_version = state
                .prompt_lab()
                .selected_prompt_version()
                .or_else(|| state.active_version_for(prompt_id));
            let model_override = state.prompt_lab().selected_model_override().cloned();
            state.mark_dirty();
            let effects = dispatch_prompt_lab_run(
                &mut state,
                PromptLabDispatchRequest {
                    stage,
                    prompt_id,
                    input_snapshot: input,
                    prompt_version,
                    model_override,
                    compare_batch_id: None,
                    compare_candidate_id: None,
                },
            );
            return (state, effects);
        }
        Msg::PromptLabTemplateRevertRequested => {
            let stage = state.prompt_lab().selected_stage();
            let prompt_id = prompt_id_for_stage(stage);
            ensure_prompt_lab_template_draft(&mut state, prompt_id);
            if state.prompt_lab_mut().revert_template(prompt_id) {
                state.mark_dirty();
            }
            Vec::new()
        }
        Msg::PromptLabTemplateSaveRequested => {
            let stage = state.prompt_lab().selected_stage();
            let prompt_id = prompt_id_for_stage(stage);
            let effect = if let Some(draft) = state.prompt_lab().template_draft(prompt_id) {
                if draft.is_applied() && draft.validation_errors().is_empty() {
                    Some(Effect::SavePromptTemplateFile {
                        prompt_id,
                        system_template: draft.system_draft().to_string(),
                        user_template: draft.user_draft().to_string(),
                        description: draft.description().to_string(),
                        expected_format: draft.expected_format().to_string(),
                    })
                } else {
                    engine_warn!(
                        "[prompt-lab-template] Save requested without applied draft prompt_id={:?}",
                        prompt_id
                    );
                    None
                }
            } else {
                engine_warn!(
                    "[prompt-lab-template] Save requested but no draft open prompt_id={:?}",
                    prompt_id
                );
                None
            };
            if let Some(effect) = effect {
                return (state, vec![effect]);
            }
            Vec::new()
        }
        Msg::PromptLabTemplateSaved {
            prompt_id,
            version,
            path,
        } => {
            let path_buf = PathBuf::from(path.clone());
            state
                .prompt_lab_mut()
                .mark_template_saved(prompt_id, version, path_buf);
            engine_info!(
                "[prompt-lab-template] PromptLabTemplateSaved prompt_id={:?} path={} version={}",
                prompt_id,
                path,
                version
            );
            state.mark_dirty();
            Vec::new()
        }
        Msg::PromptLabTemplateSaveFailed { prompt_id, reason } => {
            engine_error!(
                "[prompt-lab-template] PromptLabTemplateSaveFailed prompt_id={:?} reason={}",
                prompt_id,
                reason
            );
            Vec::new()
        }
        Msg::PromptLabRunRequested => {
            if state.prompt_lab().has_in_flight_run() {
                return (state, Vec::new());
            }
            let snapshot = state
                .prompt_lab()
                .resolved_url_snapshot()
                .map(ToOwned::to_owned);
            let input = match snapshot {
                Some(text) => text,
                None => return (state, Vec::new()),
            };
            let stage = state.prompt_lab().selected_stage();
            let prompt_id = prompt_id_for_stage(stage);
            let prompt_version = state
                .prompt_lab()
                .selected_prompt_version()
                .or_else(|| state.active_version_for(prompt_id));
            let model_override = state.prompt_lab().selected_model_override().cloned();
            let effects = dispatch_prompt_lab_run(
                &mut state,
                PromptLabDispatchRequest {
                    stage,
                    prompt_id,
                    input_snapshot: input,
                    prompt_version,
                    model_override,
                    compare_batch_id: None,
                    compare_candidate_id: None,
                },
            );
            return (state, effects);
        }
        Msg::PromptLabRerunRequested => {
            if state.prompt_lab().has_in_flight_run() {
                return (state, Vec::new());
            }
            let latest = match state.prompt_lab().latest_run() {
                Some(run) => run,
                None => return (state, Vec::new()),
            };
            if matches!(latest.status, PromptLabRunStatus::Pending { .. }) {
                return (state, Vec::new());
            }
            let stage = latest.stage;
            let prompt_id = latest.prompt_id;
            let input_snapshot = latest.input_snapshot.clone();
            let prompt_version = latest.prompt_version_used;
            let model_override = latest.model_override.clone();
            let effects = dispatch_prompt_lab_run(
                &mut state,
                PromptLabDispatchRequest {
                    stage,
                    prompt_id,
                    input_snapshot,
                    prompt_version,
                    model_override,
                    compare_batch_id: None,
                    compare_candidate_id: None,
                },
            );
            return (state, effects);
        }
        Msg::PromptLabHistoryCleared => {
            state.clear_prompt_lab_history();
            Vec::new()
        }
        Msg::PromptLabCompareDraftReset => {
            state.prompt_lab_mut().clear_draft_candidates();
            state.mark_dirty();
            Vec::new()
        }
        Msg::PromptLabCompareCurrentSettingsCaptured => {
            if state
                .prompt_lab_mut()
                .add_draft_candidate_from_current(None)
                .is_ok()
            {
                state.mark_dirty();
            }
            Vec::new()
        }
        Msg::PromptLabCompareBaselineCaptured => {
            if state.prompt_lab_mut().add_baseline_candidate(None).is_ok() {
                state.mark_dirty();
            }
            Vec::new()
        }
        Msg::PromptLabCompareCandidateRemoved { candidate_id } => {
            if state.prompt_lab_mut().remove_draft_candidate(candidate_id) {
                state.mark_dirty();
            }
            Vec::new()
        }
        Msg::PromptLabCompareCandidateLabelChanged {
            candidate_id,
            label,
        } => {
            if state
                .prompt_lab_mut()
                .rename_draft_candidate(candidate_id, label)
            {
                state.mark_dirty();
            }
            Vec::new()
        }
        Msg::PromptLabCompareBatchStartRequested | Msg::PromptLabCompareBatchConfirmedStart => {
            if state.prompt_lab().has_in_flight_run() {
                return (state, Vec::new());
            }
            let snapshot = state
                .prompt_lab()
                .resolved_url_snapshot()
                .map(ToOwned::to_owned);
            let input = match snapshot {
                Some(text) => text,
                None => return (state, Vec::new()),
            };
            let batch_id = match state.prompt_lab_mut().freeze_batch(input.clone()) {
                Ok(batch_id) => batch_id,
                Err(_) => return (state, Vec::new()),
            };
            let effects = dispatch_next_compare_candidate(&mut state, batch_id);
            state.mark_dirty();
            effects
        }
        Msg::PromptLabCompareBatchCancelRequested => {
            if let Some(batch) = state.prompt_lab_mut().active_batch_mut() {
                batch.status = PromptLabCompareBatchStatus::Cancelled;
                let batch_id = batch.batch_id;
                state.prompt_lab_mut().clear_active_batch_if(batch_id);
                state.mark_dirty();
            }
            Vec::new()
        }
        Msg::PromptLabCompareWinnerSelected { run_id } => {
            if let Some(batch) = state.prompt_lab_mut().active_batch_mut() {
                batch.selected_run_id = Some(run_id);
                state.mark_dirty();
            }
            Vec::new()
        }
        Msg::PromptLabCompareWinnerCleared => {
            if let Some(batch) = state.prompt_lab_mut().active_batch_mut() {
                batch.selected_run_id = None;
                state.mark_dirty();
            }
            Vec::new()
        }
        Msg::PromptLabCompareRunRated { run_id, rating } => {
            if !(1..=5).contains(&rating) {
                return (state, Vec::new());
            }
            if let Some(run) = state.prompt_lab_mut().run_by_id_mut(run_id) {
                run.operator_rating = Some(rating);
                if let Some(batch_id) = run.compare_batch_id {
                    state
                        .prompt_lab_mut()
                        .recompute_auto_select_for_batch(batch_id);
                }
                state.mark_dirty();
            }
            Vec::new()
        }
        Msg::PromptLabComparePolicyUpdated {
            require_parse_ok,
            max_cost_microdollars,
            max_wall_ms,
            rating_beats_cost,
        } => {
            let lab = state.prompt_lab_mut();
            if let Some(value) = require_parse_ok {
                lab.compare_policy.require_parse_ok = value;
            }
            if let Some(value) = max_cost_microdollars {
                lab.compare_policy.max_cost_microdollars = value;
            }
            if let Some(value) = max_wall_ms {
                lab.compare_policy.max_wall_ms = value;
            }
            if let Some(value) = rating_beats_cost {
                lab.compare_policy.rating_beats_cost = value;
            }
            if let Some(batch_id) = lab.active_batch().map(|batch| batch.batch_id) {
                lab.recompute_auto_select_for_batch(batch_id);
            }
            state.mark_dirty();
            Vec::new()
        }
        Msg::PromptLabCompareAutoSelectRequested => {
            if let Some(batch_id) = state
                .prompt_lab()
                .active_batch()
                .map(|batch| batch.batch_id)
            {
                state
                    .prompt_lab_mut()
                    .recompute_auto_select_for_batch(batch_id);
                state.mark_dirty();
            }
            Vec::new()
        }
        Msg::PromptLabCompareBatchSetWarning { batch_id, warning } => {
            if let Some(batch) = state
                .prompt_lab_mut()
                .batches
                .iter_mut()
                .find(|batch| batch.batch_id == batch_id)
            {
                batch.warning = warning;
                state.mark_dirty();
            }
            Vec::new()
        }
        Msg::Tick | Msg::NoOp => Vec::new(),
    };

    (state, effects)
}

fn dispatch_prompt_lab_run(
    state: &mut AppState,
    dispatch: PromptLabDispatchRequest,
) -> Vec<Effect> {
    let PromptLabDispatchRequest {
        stage,
        prompt_id,
        input_snapshot,
        prompt_version,
        model_override,
        compare_batch_id,
        compare_candidate_id,
    } = dispatch;
    let request_id = state.allocate_next_llm_request_id();
    let run_id = state.allocate_next_prompt_lab_run_id();
    let context = state
        .prompt_lab()
        .applied_context_pairs(prompt_id)
        .map(|pairs| pairs.to_vec())
        .unwrap_or_else(|| state.context_for(prompt_id).to_vec());
    let pending_prompt_version = prompt_version;
    let pending_model_override = model_override.clone();
    state.record_pending_llm_request(request_id, prompt_id);
    state.add_prompt_lab_pending_run(crate::state::PromptLabPendingRunRegistration {
        run_id,
        stage,
        prompt_id,
        input_snapshot: input_snapshot.clone(),
        request_id,
        overrides: crate::prompt_lab::PromptLabRunOverrides {
            prompt_version_used: pending_prompt_version,
            model_override: pending_model_override,
        },
        compare_batch_id,
        compare_candidate_id,
    });
    state.mark_dirty();
    engine_info!(
        "[prompt-lab] run requested run_id={} request_id={} stage={:?}",
        run_id.0,
        request_id,
        stage
    );
    vec![Effect::RequestLlmCompletion {
        request_id,
        prompt_id,
        prompt_version,
        model_override,
        input_content: input_snapshot,
        context,
        template_override: state.prompt_lab().applied_template_override(prompt_id),
    }]
}

struct PromptLabDispatchRequest {
    stage: PromptLabStage,
    prompt_id: PromptId,
    input_snapshot: String,
    prompt_version: Option<PromptVersion>,
    model_override: Option<ModelId>,
    compare_batch_id: Option<crate::prompt_lab::PromptLabCompareBatchId>,
    compare_candidate_id: Option<u64>,
}

fn dispatch_next_compare_candidate(
    state: &mut AppState,
    batch_id: crate::prompt_lab::PromptLabCompareBatchId,
) -> Vec<Effect> {
    let Some(batch) = state
        .prompt_lab()
        .batches()
        .iter()
        .find(|batch| batch.batch_id == batch_id)
        .cloned()
    else {
        return Vec::new();
    };
    let Some(candidate) = batch.next_undispatched_candidate().cloned() else {
        return Vec::new();
    };
    let input_snapshot = batch.input_snapshot.clone();
    let effects = dispatch_prompt_lab_run(
        state,
        PromptLabDispatchRequest {
            stage: candidate.stage,
            prompt_id: candidate.prompt_id,
            input_snapshot,
            prompt_version: candidate.prompt_version,
            model_override: candidate.model_override.clone(),
            compare_batch_id: Some(batch_id),
            compare_candidate_id: Some(candidate.candidate_id),
        },
    );
    if let Some(run_id) = state.prompt_lab().latest_run().map(|run| run.run_id) {
        state
            .prompt_lab_mut()
            .record_compare_dispatch(batch_id, candidate.candidate_id, run_id);
    }
    effects
}

fn parse_urls(raw: &str) -> Vec<String> {
    raw.lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .map(ToOwned::to_owned)
        .collect()
}

fn refresh_pre_triage_if_needed(state: &mut AppState) -> Vec<Effect> {
    let ordered_urls = state.ordered_completed_job_urls();
    if ordered_urls.is_empty() {
        state.set_pre_triage(PreTriageSession::default());
        state.clear_pre_triage_manual_overrides();
        return Vec::new();
    }
    state.set_pre_triage(PreTriageSession::new_loading());
    state.mark_dirty();
    vec![Effect::LoadArticlesForTriage { ordered_urls }]
}

fn start_triage_from_pretriage(state: &mut AppState) -> Vec<Effect> {
    let included = state.pre_triage().resolved_included_articles();
    if included.is_empty() {
        state
            .triage_mut()
            .fail("no completed articles found".to_string());
        state.mark_dirty();
        return Vec::new();
    }
    state.set_triage(TriageSession::new_loading(None));
    state.triage_mut().set_articles(included);
    state.triage_mut().transition_to_triaging();
    state.mark_dirty();
    state.start_triage_cache_run();
    state.mark_triage_metadata_ready();
    let mut effects = Vec::new();
    dispatch_next_triage_step(state, &mut effects);
    effects
}

fn dispatch_next_triage_step(state: &mut AppState, effects: &mut Vec<Effect>) {
    log_triage_cache_run_start_if_needed(state);
    let limit = state.triage_max_in_flight();

    // Fill available in-flight slots.
    while state.triage().can_dispatch_more(limit) {
        let next_idx = state
            .triage()
            .next_pending_index()
            .expect("can_dispatch_more guarantees pending exists");

        let content_hash = state.triage().articles()[next_idx].content_hash.clone();
        let content_hash_short = short_hash(&content_hash);

        match state.try_reuse_triage(&content_hash) {
            TriageCacheLookupResult::Hit(cached) => {
                let result = cached.clone();
                state.record_triage_cache_hit();
                engine_info!("[triage-cache] hit content_hash={}", content_hash_short);
                state.triage_mut().complete_article(next_idx, result);
                state.refresh_selected_preview();
                state.mark_dirty();
                continue;
            }
            TriageCacheLookupResult::Miss => {
                state.record_triage_cache_miss();
                engine_info!("[triage-cache] miss content_hash={}", content_hash_short);
            }
            TriageCacheLookupResult::KeyUnavailable => {
                state.record_triage_cache_key_unavailable();
                if state.triage_metadata_ready() {
                    engine_warn!(
                        "[triage-cache] key-unavailable despite metadata-ready content_hash={}",
                        content_hash_short
                    );
                } else {
                    engine_info!(
                        "[triage-cache] key-unavailable metadata-pending content_hash={}",
                        content_hash_short
                    );
                }
            }
        }

        let prepared_text = state.triage().articles()[next_idx].prepared_text.clone();
        let request_id = state.allocate_next_llm_request_id();
        state.record_pending_llm_request(request_id, PromptId::ArticleTriage);
        state.triage_mut().start_article(next_idx, request_id);

        let context = state.context_for(PromptId::ArticleTriage).to_vec();

        engine_info!(
            "[llm-concurrency] triage dispatch request_id={} article={} in_flight={} limit={}",
            request_id,
            next_idx,
            state.triage().in_progress_count(),
            limit
        );

        effects.push(Effect::RequestLlmCompletion {
            request_id,
            prompt_id: PromptId::ArticleTriage,
            prompt_version: None,
            model_override: None,
            input_content: prepared_text,
            context,
            template_override: None,
        });
        state.mark_dirty();
    }

    // Check if all articles are settled (no pending, no in-progress).
    if state.triage().pending_count() == 0 && state.triage().in_progress_count() == 0 {
        if state.triage().completed_count() == 0 {
            state
                .triage_mut()
                .fail("all triage attempts failed".to_string());
        } else {
            state.triage_mut().complete();
            if state.briefing_orchestration_requested() {
                on_triage_settled_for_briefing(state, effects);
            }
        }
        log_triage_cache_run_summary(state);
        effects.push(Effect::PersistTriageCache {
            cache: state.triage_cache().clone(),
        });
        state.mark_dirty();
    }
}

fn ensure_prompt_lab_template_draft(state: &mut AppState, prompt_id: PromptId) {
    if state.prompt_lab().template_draft(prompt_id).is_some() {
        return;
    }
    if let Some(snapshot) = state.prompt_lab_template_snapshot(prompt_id).cloned() {
        let template = snapshot.template;
        state.prompt_lab_mut().open_template_draft(
            prompt_id,
            &template.system_template,
            &template.user_template,
            &template.description,
            &template.expected_format,
        );
    }
}

fn template_draft_texts(state: &mut AppState, prompt_id: PromptId) -> Option<(String, String)> {
    ensure_prompt_lab_template_draft(state, prompt_id);
    state.prompt_lab().template_draft(prompt_id).map(|draft| {
        (
            draft.system_draft().to_string(),
            draft.user_draft().to_string(),
        )
    })
}

fn apply_prompt_lab_template_draft(state: &mut AppState, prompt_id: PromptId) -> bool {
    let (system, user) = match template_draft_texts(state, prompt_id) {
        Some(pair) => pair,
        None => return false,
    };
    let errors = validate_template(prompt_id, &system, &user);
    if !errors.is_empty() {
        engine_warn!(
            "[prompt-lab-template] validation failed prompt_id={:?} error_count={}",
            prompt_id,
            errors.len()
        );
    }
    let applied = state.prompt_lab_mut().apply_template(prompt_id, errors);
    if applied {
        engine_info!(
            "[prompt-lab-template] PromptLabTemplateApplied prompt_id={:?}",
            prompt_id
        );
    }
    applied
}

fn on_triage_settled_for_briefing(state: &mut AppState, effects: &mut Vec<Effect>) {
    if !state.briefing_orchestration_requested() {
        return;
    }
    let policy = state.briefing_triage_policy();
    let ordered_urls = policy.eligible_urls(state.triage());
    engine_info!(
        "[briefing-triage] eligible count={} cutoff={}",
        ordered_urls.len(),
        policy.cutoff_exclusive
    );
    if ordered_urls.is_empty() {
        state
            .briefing_mut()
            .fail("No articles with sufficient priority".to_string());
        state.clear_briefing_orchestration();
        state.mark_dirty();
        return;
    }

    let _ = state.take_briefing_prereq_articles();
    state.start_summary_cache_run();
    state.mark_briefing_metadata_ready();
    state.set_briefing(BriefingSession::new_loading(None));
    state.clear_briefing_orchestration_request();
    effects.push(Effect::LoadArticlesForBriefing { ordered_urls });
}

fn dispatch_next_briefing_step(state: &mut AppState, effects: &mut Vec<Effect>) {
    log_summary_cache_warmup_if_needed(state);

    let limit = state.summary_max_in_flight();

    loop {
        // Stop filling if we've reached the concurrency limit.
        if state.briefing().in_progress_count() >= limit {
            break;
        }

        let Some(next_idx) = state.briefing().next_pending_index() else {
            break;
        };

        let article = &state.briefing().articles()[next_idx];
        let prepared_text = article.prepared_text.clone();
        let content_hash = article.content_hash.clone();
        let content_hash_short = short_hash(&content_hash);
        let context = state.context_for(PromptId::ArticleSummary).to_vec();
        let context_hash_value = context_hash(&context);
        let metadata = state.summary_cache_metadata();
        let version_display = metadata
            .map(|(version, _)| version.to_string())
            .unwrap_or_else(|| "<none>".to_string());
        let model_display = metadata
            .map(|(_, model)| model.to_string())
            .unwrap_or_else(|| "<none>".to_string());

        match build_summary_cache_key(
            &content_hash,
            PromptId::ArticleSummary,
            metadata.map(|(version, _)| version),
            metadata.map(|(_, model)| model),
            &context,
        ) {
            Ok(key) => {
                state
                    .briefing_mut()
                    .set_article_cache_key(next_idx, Some(key.clone()));
                if let Some(cached_result) = state.try_reuse_summary(&key) {
                    let result = cached_result.clone();
                    state.record_summary_cache_hit();
                    engine_info!(
                        "[summary-cache] article={} decision=hit reason=cache-hit prompt_version={} model_id={} context_hash={} content_hash_short={}",
                        next_idx,
                        version_display,
                        model_display,
                        &context_hash_value,
                        content_hash_short
                    );
                    state.briefing_mut().complete_article(next_idx, result);
                    state.briefing_mut().set_article_cache_key(next_idx, None);
                    state.refresh_selected_preview();
                    state.mark_dirty();
                    // Cache hit: slot not consumed, continue filling.
                    continue;
                }

                state.record_summary_cache_miss();
                engine_info!(
                    "[summary-cache] article={} decision=miss reason=cache-miss prompt_version={} model_id={} context_hash={} content_hash_short={}",
                    next_idx,
                    version_display,
                    model_display,
                    &context_hash_value,
                    content_hash_short
                );
                let request_id = state.allocate_next_llm_request_id();
                state.record_pending_llm_request(request_id, PromptId::ArticleSummary);
                state.briefing_mut().start_article(next_idx, request_id);
                engine_info!(
                    "[llm-concurrency] summary dispatch request_id={} article={} in_flight={} limit={}",
                    request_id,
                    next_idx,
                    state.briefing().in_progress_count(),
                    limit
                );
                effects.push(Effect::RequestLlmCompletion {
                    request_id,
                    prompt_id: PromptId::ArticleSummary,
                    prompt_version: None,
                    model_override: None,
                    input_content: prepared_text,
                    context,
                    template_override: None,
                });
                state.mark_dirty();
                // Live request: continue loop to fill remaining slots.
                continue;
            }
            Err(err) => {
                state.briefing_mut().set_article_cache_key(next_idx, None);
                state.record_summary_cache_key_unavailable();
                let reason = summary_cache_key_error_reason(&err);
                engine_info!(
                    "[summary-cache] article={} decision=key_unavailable reason={} prompt_version={} model_id={} context_hash={} content_hash_short={}",
                    next_idx,
                    reason,
                    version_display,
                    model_display,
                    &context_hash_value,
                    content_hash_short
                );
                let request_id = state.allocate_next_llm_request_id();
                state.record_pending_llm_request(request_id, PromptId::ArticleSummary);
                state.briefing_mut().start_article(next_idx, request_id);
                engine_info!(
                    "[llm-concurrency] summary dispatch (no-cache-key) request_id={} article={} in_flight={} limit={}",
                    request_id,
                    next_idx,
                    state.briefing().in_progress_count(),
                    limit
                );
                effects.push(Effect::RequestLlmCompletion {
                    request_id,
                    prompt_id: PromptId::ArticleSummary,
                    prompt_version: None,
                    model_override: None,
                    input_content: prepared_text,
                    context,
                    template_override: None,
                });
                state.mark_dirty();
                // Live request: continue loop to fill remaining slots.
                continue;
            }
        }
    }

    // Gate aggregate briefing on ALL articles settled (no pending, no in-progress).
    if state.briefing().pending_count() > 0 || state.briefing().in_progress_count() > 0 {
        return;
    }

    if state.briefing().completed_summary_count() == 0 {
        state
            .briefing_mut()
            .fail("all article summaries failed".to_string());
        state.mark_dirty();
        log_summary_cache_run_summary(state);
        effects.push(Effect::PersistSummaryCache {
            cache: state.summary_cache().clone(),
        });
        return;
    }

    if state.briefing_orchestration_skip_aggregate() {
        state.briefing_mut().complete_without_briefing();
        state.revert_preview_to_briefing();
        state.clear_briefing_orchestration();
        state.mark_dirty();
        log_summary_cache_run_summary(state);
        effects.push(Effect::PersistSummaryCache {
            cache: state.summary_cache().clone(),
        });
        return;
    }

    let collection_text = match state.briefing().collection_text() {
        Some(text) => text.to_string(),
        None => {
            state
                .briefing_mut()
                .fail("missing briefing collection".to_string());
            state.mark_dirty();
            log_summary_cache_run_summary(state);
            effects.push(Effect::PersistSummaryCache {
                cache: state.summary_cache().clone(),
            });
            return;
        }
    };
    let request_id = state.allocate_next_llm_request_id();
    state.record_pending_llm_request(request_id, PromptId::AggregateBriefing);
    state.briefing_mut().set_briefing_request_id(request_id);

    let context = state.context_for(PromptId::AggregateBriefing).to_vec();

    effects.push(Effect::RequestLlmCompletion {
        request_id,
        prompt_id: PromptId::AggregateBriefing,
        prompt_version: None,
        model_override: None,
        input_content: collection_text,
        context,
        template_override: None,
    });
    state.mark_dirty();
}

fn try_start_briefing_with_metadata(state: &mut AppState, effects: &mut Vec<Effect>) {
    if !state.is_briefing_metadata_ready() {
        return;
    }
    if matches!(state.briefing().phase(), BriefingPhase::Summarizing) {
        dispatch_next_briefing_step(state, effects);
    }
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

fn log_triage_cache_run_start_if_needed(state: &mut AppState) {
    if state.triage_cache_run_start_logged() {
        return;
    }
    let metadata = state.triage_cache_metadata();
    if let Some((version, model_id, _)) = metadata {
        engine_info!(
            "[triage-cache] run-start prompt_version={} model_id={}",
            version,
            model_id
        );
        state.mark_triage_cache_run_started();
    }
}

fn log_triage_cache_run_summary(state: &mut AppState) {
    let metrics = state.triage_cache_metrics();
    engine_info!(
        "[triage-cache] run summary hits={} misses={} key_unavailable={} total={}",
        metrics.hits(),
        metrics.misses(),
        metrics.key_unavailable(),
        metrics.total()
    );
    state.finalize_triage_cache_run();
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
mod tests {
    use std::collections::HashMap;
    use std::sync::Once;

    use super::*;
    use crate::briefing::{ArticleSummaryState, BriefingPhase, LoadedArticle};
    use harvester_engine::llm::run_metadata::LlmRunMetadata;

    fn init_logging() {
        static INIT: Once = Once::new();
        INIT.call_once(engine_logging::initialize_for_tests);
    }

    fn loaded_articles() -> (Vec<LoadedArticle>, String) {
        fn long_text(prefix: &str) -> String {
            format!(
                "{prefix} {}",
                std::iter::repeat_n("content", 220)
                    .collect::<Vec<_>>()
                    .join(" ")
            )
        }
        let articles = vec![
            LoadedArticle {
                url: "https://example.com/a".to_string(),
                source_title: Some("Article A".to_string()),
                prepared_text: long_text("Article A text"),
                content_hash: "hash-a".to_string(),
            },
            LoadedArticle {
                url: "https://example.com/b".to_string(),
                source_title: Some("Article B".to_string()),
                prepared_text: long_text("Article B text"),
                content_hash: "hash-b".to_string(),
            },
        ];
        (articles, "Collection text".to_string())
    }

    fn loaded_single_article() -> (Vec<LoadedArticle>, String) {
        let articles = vec![LoadedArticle {
            url: "https://example.com/a".to_string(),
            source_title: Some("Article A".to_string()),
            prepared_text: format!(
                "Article A text {}",
                std::iter::repeat_n("content", 220)
                    .collect::<Vec<_>>()
                    .join(" ")
            ),
            content_hash: "hash-a".to_string(),
        }];
        (articles, "Collection text".to_string())
    }

    fn with_summary_metadata(state: AppState) -> AppState {
        let mut active_versions = HashMap::new();
        active_versions.insert(PromptId::ArticleSummary, 1);
        let mut effective_models = HashMap::new();
        effective_models.insert(PromptId::ArticleSummary, "test-model".to_string());
        let (state, _) = update(
            state,
            Msg::LlmMetadataLoaded {
                active_versions,
                effective_models,
                templates: HashMap::new(),
            },
        );
        state
    }

    fn summary_json(title: &str) -> String {
        format!("{{\"title\":\"{title}\",\"summary\":\"Summary\",\"key_points\":[\"p1\"]}}")
    }

    fn briefing_json(article_count: u32) -> String {
        format!(
            "{{\"executive_summary\":\"Exec\",\"themes\":[{{\"name\":\"Theme\",\"description\":\"Desc\"}}],\"article_count\":{article_count}}}"
        )
    }

    fn request_id_for_prompt(effects: &[Effect], prompt_id: PromptId) -> Option<u64> {
        effects.iter().find_map(|effect| match effect {
            Effect::RequestLlmCompletion {
                request_id,
                prompt_id: pid,
                ..
            } if *pid == prompt_id => Some(*request_id),
            _ => None,
        })
    }

    fn start_briefing_after_triage(state: AppState, articles: Vec<LoadedArticle>) -> AppState {
        let (state, _) = update(state, Msg::GenerateBriefingClicked);
        let state = with_summary_metadata(state);
        let (mut state, effects) = update(
            state,
            Msg::BriefingPrereqArticlesLoaded {
                articles: articles.clone(),
            },
        );
        let mut triage_request_id =
            request_id_for_prompt(&effects, PromptId::ArticleTriage).expect("triage request");

        for idx in 0..articles.len() {
            let priority = (articles.len() - idx + 1) as u8;
            let (next_state, next_effects) = update(
                state,
                Msg::LlmCompleted {
                    request_id: triage_request_id,
                    result: LlmResultKind::Success {
                        output_json: format!(
                            "{{\"category\":\"security\",\"priority\":{},\"tags\":[\"tag\"],\"rationale\":\"reason\"}}",
                            priority
                        ),
                        input_tokens: 10,
                        output_tokens: 5,
                        prompt_version: 1,
                        model_id: "test-model".to_string(),
                    },
                    metadata: None,
                },
            );
            state = next_state;
            if idx + 1 < articles.len() {
                triage_request_id = request_id_for_prompt(&next_effects, PromptId::ArticleTriage)
                    .expect("next triage request");
            } else {
                assert!(
                    next_effects
                        .iter()
                        .any(|effect| matches!(effect, Effect::LoadArticlesForBriefing { .. })),
                    "final triage completion should trigger filtered briefing load"
                );
            }
        }
        state
    }

    #[test]
    fn generate_briefing_emits_load_effect() {
        init_logging();
        let state = AppState::new();
        let (state, effects) = update(state, Msg::GenerateBriefingClicked);

        assert_eq!(
            effects,
            vec![
                Effect::LoadPromptContexts,
                Effect::LoadPromptTemplateFiles,
                Effect::LoadLlmMetadata,
                Effect::LoadArticlesForBriefingPrereq {
                    ordered_urls: Vec::new(),
                }
            ]
        );
        assert_eq!(state.briefing().phase(), &BriefingPhase::WaitingForTriage);
    }

    #[test]
    fn articles_loaded_dispatches_first_summary() {
        init_logging();
        let state = AppState::new();
        let state = start_briefing_after_triage(state, loaded_articles().0.clone());
        let (articles, collection_text) = loaded_articles();

        let (state, effects) = update(
            state,
            Msg::ArticlesLoaded {
                articles,
                collection_text,
            },
        );

        assert_eq!(effects.len(), 1);
        assert!(matches!(
            &effects[0],
            Effect::RequestLlmCompletion {
                request_id: 3,
                prompt_id: PromptId::ArticleSummary,
                prompt_version: None,
                model_override: None,
                input_content,
                context,
                template_override: None,
            } if input_content.starts_with("Article A text") && context.is_empty()
        ));
        assert!(matches!(
            state.briefing().phase(),
            BriefingPhase::Summarizing
        ));
        assert!(matches!(
            state.briefing().articles()[0].summary_state,
            ArticleSummaryState::InProgress { request_id: 3 }
        ));
    }

    #[test]
    fn summary_completion_advances_and_generates_briefing() {
        init_logging();
        let state = AppState::new();
        let state = start_briefing_after_triage(state, loaded_articles().0.clone());
        let (articles, collection_text) = loaded_articles();
        let (state, _effects) = update(
            state,
            Msg::ArticlesLoaded {
                articles,
                collection_text,
            },
        );

        let (state, effects) = update(
            state,
            Msg::LlmCompleted {
                request_id: 3,
                result: LlmResultKind::Success {
                    output_json: summary_json("Article A"),
                    input_tokens: 10,
                    output_tokens: 5,
                    prompt_version: 1,
                    model_id: "test-model".to_string(),
                },
                metadata: None,
            },
        );

        assert_eq!(effects.len(), 1);
        assert!(matches!(
            &effects[0],
            Effect::RequestLlmCompletion {
                request_id: 4,
                prompt_id: PromptId::ArticleSummary,
                prompt_version: None,
                model_override: None,
                input_content,
                context,
                template_override: None,
            } if input_content.starts_with("Article B text") && context.is_empty()
        ));

        let (state, effects) = update(
            state,
            Msg::LlmCompleted {
                request_id: 4,
                result: LlmResultKind::Success {
                    output_json: summary_json("Article B"),
                    input_tokens: 10,
                    output_tokens: 5,
                    prompt_version: 1,
                    model_id: "test-model".to_string(),
                },
                metadata: None,
            },
        );

        assert_eq!(
            effects,
            vec![Effect::RequestLlmCompletion {
                request_id: 5,
                prompt_id: PromptId::AggregateBriefing,
                prompt_version: None,
                model_override: None,
                input_content: "Collection text".to_string(),
                context: Vec::new(),
                template_override: None,
            }]
        );

        let (state, effects) = update(
            state,
            Msg::LlmCompleted {
                request_id: 5,
                result: LlmResultKind::Success {
                    output_json: briefing_json(2),
                    input_tokens: 20,
                    output_tokens: 8,
                    prompt_version: 1,
                    model_id: "test-model".to_string(),
                },
                metadata: None,
            },
        );

        assert_eq!(effects.len(), 1);
        assert!(matches!(effects[0], Effect::PersistSummaryCache { .. }));
        assert_eq!(state.briefing().phase(), &BriefingPhase::Complete);
        assert!(state.briefing().briefing_result().is_some());
    }

    #[test]
    fn summary_store_uses_run_frozen_metadata_when_completion_model_differs() {
        init_logging();
        let state = AppState::new();
        let state = start_briefing_after_triage(state, loaded_single_article().0.clone());
        let (articles, collection_text) = loaded_single_article();
        let (state, _) = update(
            state,
            Msg::ArticlesLoaded {
                articles,
                collection_text,
            },
        );

        let (state, _) = update(
            state,
            Msg::LlmCompleted {
                request_id: 2,
                result: LlmResultKind::Success {
                    output_json: summary_json("Article A"),
                    input_tokens: 10,
                    output_tokens: 5,
                    prompt_version: 77,
                    model_id: "test-model-2024-07-18".to_string(),
                },
                metadata: None,
            },
        );

        let keys: Vec<_> = state
            .summary_cache()
            .iter()
            .map(|(key, _)| key.clone())
            .collect();
        assert_eq!(keys.len(), 1);
        assert_eq!(keys[0].prompt_version, 1);
        assert_eq!(keys[0].model_id, "test-model");
    }

    #[test]
    fn second_run_reuses_cached_summary_with_configured_model_key() {
        init_logging();
        let state = AppState::new();
        let state = start_briefing_after_triage(state, loaded_single_article().0.clone());
        let (articles, collection_text) = loaded_single_article();
        let (state, _) = update(
            state,
            Msg::ArticlesLoaded {
                articles,
                collection_text,
            },
        );
        let (state, effects) = update(
            state,
            Msg::LlmCompleted {
                request_id: 2,
                result: LlmResultKind::Success {
                    output_json: summary_json("Article A"),
                    input_tokens: 10,
                    output_tokens: 5,
                    prompt_version: 88,
                    model_id: "test-model-2024-07-18".to_string(),
                },
                metadata: None,
            },
        );
        assert_eq!(
            effects,
            vec![Effect::RequestLlmCompletion {
                request_id: 3,
                prompt_id: PromptId::AggregateBriefing,
                prompt_version: None,
                model_override: None,
                input_content: "Collection text".to_string(),
                context: Vec::new(),
                template_override: None,
            }]
        );
        let (state, _) = update(
            state,
            Msg::LlmCompleted {
                request_id: 3,
                result: LlmResultKind::Success {
                    output_json: briefing_json(1),
                    input_tokens: 10,
                    output_tokens: 4,
                    prompt_version: 1,
                    model_id: "test-model".to_string(),
                },
                metadata: None,
            },
        );

        let (state, effects) = update(state, Msg::GenerateBriefingClicked);
        assert_eq!(
            effects,
            vec![
                Effect::LoadPromptContexts,
                Effect::LoadPromptTemplateFiles,
                Effect::LoadLlmMetadata,
                Effect::LoadArticlesForBriefingPrereq {
                    ordered_urls: Vec::new(),
                }
            ]
        );
        let state = with_summary_metadata(state);
        let (state, effects) = update(
            state,
            Msg::BriefingPrereqArticlesLoaded {
                articles: loaded_single_article().0,
            },
        );
        assert!(effects
            .iter()
            .any(|effect| matches!(effect, Effect::LoadArticlesForBriefing { .. })));
        let (articles, collection_text) = loaded_single_article();
        let (_state, effects) = update(
            state,
            Msg::ArticlesLoaded {
                articles,
                collection_text,
            },
        );

        assert_eq!(
            effects,
            vec![Effect::RequestLlmCompletion {
                request_id: 4,
                prompt_id: PromptId::AggregateBriefing,
                prompt_version: None,
                model_override: None,
                input_content: "Collection text".to_string(),
                context: Vec::new(),
                template_override: None,
            }]
        );
    }

    #[test]
    fn splitter_move_preserves_minimum_jobs_width_with_fixed_input_panel() {
        init_logging();
        let state = AppState::new();

        let (state, effects) = update(
            state,
            Msg::SplitterMoved {
                desired_left_width_px: 300,
            },
        );

        assert!(effects.is_empty());
        assert_eq!(
            state.left_panel_width(),
            INPUT_PANEL_FIXED_WIDTH + MIN_JOBS_PANEL_WIDTH
        );
    }

    fn make_state_with_summarized_job_for_update() -> AppState {
        use crate::briefing::{ArticleSummaryResult, LoadedArticle};
        let mut state = AppState::new();
        let url = "https://open-browser.example/article".to_string();
        state.restore_completed_jobs(vec![crate::CompletedJobSnapshot {
            url: url.clone(),
            tokens: None,
            bytes: None,
            links: vec![],
        }]);
        // Set up briefing with completed summary for this URL
        let mut briefing = crate::briefing::BriefingSession::new_loading(None);
        briefing.set_articles(
            vec![LoadedArticle {
                url: url.clone(),
                source_title: None,
                prepared_text: "text".to_string(),
                content_hash: "hash".to_string(),
            }],
            "collection".to_string(),
        );
        briefing.transition_to_summarizing();
        briefing.start_article(0, 1);
        briefing.complete_article(
            0,
            ArticleSummaryResult {
                title: "Article".to_string(),
                summary: "Summary".to_string(),
                key_points: vec![],
                input_tokens: 10,
                output_tokens: 5,
            },
        );
        state.set_briefing(briefing);
        // Select the job
        let job_id = state.view().jobs.first().map(|j| j.job_id).unwrap_or(1);
        state.select_job(job_id);
        state
    }

    #[test]
    fn open_in_browser_with_summarized_job_selected_emits_effect() {
        init_logging();
        let state = make_state_with_summarized_job_for_update();
        let (_state, effects) = update(state, Msg::OpenInBrowserClicked);
        assert_eq!(effects.len(), 1);
        assert!(matches!(
            &effects[0],
            Effect::OpenUrlInBrowser { url } if url == "https://open-browser.example/article"
        ));
    }

    #[test]
    fn open_in_browser_with_unsummarized_job_selected_emits_nothing() {
        init_logging();
        let mut state = AppState::new();
        state.restore_completed_jobs(vec![crate::CompletedJobSnapshot {
            url: "https://no-summary.example".to_string(),
            tokens: None,
            bytes: None,
            links: vec![],
        }]);
        let job_id = state.view().jobs.first().map(|j| j.job_id).unwrap_or(1);
        state.select_job(job_id);
        let (_state, effects) = update(state, Msg::OpenInBrowserClicked);
        assert!(effects.is_empty());
    }

    #[test]
    fn open_in_browser_with_no_selection_emits_nothing() {
        init_logging();
        let state = AppState::new();
        let (_state, effects) = update(state, Msg::OpenInBrowserClicked);
        assert!(effects.is_empty());
    }

    // ── Triage reducer integration tests ─────────────────────────────────────

    fn loaded_triage_articles(count: usize) -> Vec<LoadedArticle> {
        (0..count)
            .map(|i| LoadedArticle {
                url: format!("https://example.com/{i}"),
                source_title: None,
                prepared_text: std::iter::repeat_n(format!("article-{i}-content"), 220)
                    .collect::<Vec<_>>()
                    .join(" "),
                content_hash: format!("hash-{i}"),
            })
            .collect()
    }

    fn triage_json() -> String {
        r#"{"category":"news","priority":3,"tags":["tag"],"rationale":"ok"}"#.to_string()
    }

    fn triage_success(request_id: u64) -> Msg {
        Msg::LlmCompleted {
            request_id,
            result: LlmResultKind::Success {
                output_json: triage_json(),
                input_tokens: 10,
                output_tokens: 5,
                prompt_version: 1,
                model_id: "test-model".to_string(),
            },
            metadata: None,
        }
    }

    fn start_triage_for_test(
        state: AppState,
        articles: Vec<LoadedArticle>,
    ) -> (AppState, Vec<Effect>) {
        let mut active_versions = std::collections::HashMap::new();
        active_versions.insert(PromptId::ArticleTriage, 1);
        let mut effective_models = std::collections::HashMap::new();
        effective_models.insert(PromptId::ArticleTriage, "test-model".to_string());
        let (state, _) = update(
            state,
            Msg::LlmMetadataLoaded {
                active_versions,
                effective_models,
                templates: std::collections::HashMap::new(),
            },
        );
        let (state, _) = update(
            state,
            Msg::PromptContextsLoaded {
                contexts: std::collections::HashMap::new(),
            },
        );
        let (state, _) = update(state, Msg::TriageArticlesLoaded { articles });
        update(state, Msg::TriageClicked)
    }

    #[test]
    fn triage_clicked_emits_load_effects() {
        init_logging();
        let state = AppState::new();
        let (_state, effects) = update(state, Msg::TriageClicked);
        assert!(effects.is_empty());
    }

    #[test]
    fn summary_cache_model_id_compatibility_accepts_resolved_suffix() {
        assert!(summary_cache_model_ids_compatible(
            "gpt-4o-mini",
            "gpt-4o-mini-2024-07-18",
        ));
        assert!(!summary_cache_model_ids_compatible("gpt-4o-mini", "gpt-4o",));
    }

    #[test]
    fn briefing_blocked_when_triage_in_progress() {
        init_logging();
        let mut state = AppState::new();
        state.set_triage(crate::triage::TriageSession::new_loading(None));
        let (next_state, effects) = update(state.clone(), Msg::GenerateBriefingClicked);
        assert!(effects.is_empty());
        assert_eq!(next_state.briefing().phase(), state.briefing().phase());
    }

    #[test]
    fn triage_click_blocked_when_briefing_owns_triage() {
        init_logging();
        let state = AppState::new();
        let (state, _) = update(state, Msg::GenerateBriefingClicked);
        let (_state, effects) = update(state, Msg::TriageClicked);
        assert!(effects.is_empty());
    }

    #[test]
    fn triage_articles_loaded_dispatches_up_to_limit_requests() {
        init_logging();
        let mut state = AppState::new();
        state.set_triage_max_in_flight(2);
        let (state, effects) = start_triage_for_test(state, loaded_triage_articles(3));
        let llm_effects: Vec<_> = effects
            .iter()
            .filter(|e| matches!(e, Effect::RequestLlmCompletion { .. }))
            .collect();
        assert_eq!(
            llm_effects.len(),
            2,
            "should dispatch 2 requests for limit=2"
        );
        assert_eq!(state.triage().in_progress_count(), 2);
        assert_eq!(state.triage().pending_count(), 1);
    }

    #[test]
    fn triage_completion_backfills_one_slot() {
        init_logging();
        let mut state = AppState::new();
        state.set_triage_max_in_flight(2);
        let (state, _) = start_triage_for_test(state, loaded_triage_articles(3));
        assert_eq!(state.triage().in_progress_count(), 2);

        // Complete request_id=1 → should backfill 1 more slot
        let (state, effects) = update(state, triage_success(1));
        let llm_effects: Vec<_> = effects
            .iter()
            .filter(|e| matches!(e, Effect::RequestLlmCompletion { .. }))
            .collect();
        assert_eq!(llm_effects.len(), 1, "backfill dispatches 1 new request");
        assert_eq!(state.triage().in_progress_count(), 2);
        assert_eq!(state.triage().completed_count(), 1);
    }

    #[test]
    fn triage_out_of_order_completion_routes_correctly() {
        init_logging();
        let mut state = AppState::new();
        state.set_triage_max_in_flight(3);
        let (state, _) = start_triage_for_test(state, loaded_triage_articles(3));
        assert_eq!(state.triage().in_progress_count(), 3);

        // Complete in reverse order: 3, then 1, then 2
        let (state, _) = update(state, triage_success(3));
        let (state, _) = update(state, triage_success(1));
        let (state, _) = update(state, triage_success(2));

        assert_eq!(state.triage().completed_count(), 3);
        assert_eq!(state.triage().failed_count(), 0);
        assert!(matches!(
            state.triage().phase(),
            crate::triage::TriagePhase::Complete
        ));
    }

    #[test]
    fn triage_progress_text_counts_settled_articles() {
        init_logging();
        let mut state = AppState::new();
        state.set_triage_max_in_flight(1);
        let (state, _) = start_triage_for_test(state, loaded_triage_articles(3));
        let text = state.triage().progress_text().unwrap();
        assert!(
            text.contains("0/3"),
            "initial progress shows 0 settled: got '{text}'"
        );

        let (state, _) = update(state, triage_success(1));
        let text = state.triage().progress_text().unwrap();
        assert!(
            text.contains("1/3"),
            "after 1 complete shows 1 settled: got '{text}'"
        );
    }

    #[test]
    fn triage_quota_exhausted_fails_all_pending() {
        init_logging();
        let mut state = AppState::new();
        state.set_triage_max_in_flight(1);
        let (state, _) = start_triage_for_test(state, loaded_triage_articles(3));

        // Quota exhausted on request_id=1 → all pending should fail
        let (state, _) = update(
            state,
            Msg::LlmCompleted {
                request_id: 1,
                result: LlmResultKind::QuotaExhausted {
                    reason: "too many calls".to_string(),
                },
                metadata: None,
            },
        );
        assert_eq!(state.triage().failed_count(), 3); // 1 from quota + 2 pending
    }

    #[test]
    fn briefing_aggregate_not_dispatched_until_all_articles_settled() {
        init_logging();
        let mut state = AppState::new();
        state.set_summary_max_in_flight(2);
        let state = start_briefing_after_triage(state, loaded_articles().0.clone());
        let (articles, collection_text) = loaded_articles();

        // Load 2 articles with limit=2 → both go in-flight
        let (state, _) = update(
            state,
            Msg::ArticlesLoaded {
                articles,
                collection_text,
            },
        );
        assert_eq!(state.briefing().in_progress_count(), 2);
        assert_eq!(state.briefing().pending_count(), 0);

        // Complete only first article → aggregate should NOT be dispatched yet
        let (state, effects) = update(
            state,
            Msg::LlmCompleted {
                request_id: 3,
                result: LlmResultKind::Success {
                    output_json: summary_json("Article A"),
                    input_tokens: 10,
                    output_tokens: 5,
                    prompt_version: 1,
                    model_id: "test-model".to_string(),
                },
                metadata: None,
            },
        );
        let has_aggregate = effects.iter().any(|e| {
            matches!(
                e,
                Effect::RequestLlmCompletion {
                    prompt_id: PromptId::AggregateBriefing,
                    ..
                }
            )
        });
        assert!(
            !has_aggregate,
            "aggregate must not dispatch while article 2 still in-flight"
        );
        assert_eq!(state.briefing().in_progress_count(), 1);

        // Complete second article → aggregate should now be dispatched
        let (_state, effects) = update(
            state,
            Msg::LlmCompleted {
                request_id: 4,
                result: LlmResultKind::Success {
                    output_json: summary_json("Article B"),
                    input_tokens: 10,
                    output_tokens: 5,
                    prompt_version: 1,
                    model_id: "test-model".to_string(),
                },
                metadata: None,
            },
        );
        let has_aggregate = effects.iter().any(|e| {
            matches!(
                e,
                Effect::RequestLlmCompletion {
                    prompt_id: PromptId::AggregateBriefing,
                    ..
                }
            )
        });
        assert!(
            has_aggregate,
            "aggregate should dispatch after all articles settled"
        );
    }

    // ------------------------------------------------------------------
    // Substep C: Prompt Lab reducer arm tests
    // ------------------------------------------------------------------

    #[test]
    fn prompt_lab_open_sets_visible_and_dirty() {
        init_logging();
        let state = AppState::new();
        let (state, effects) = update(state, Msg::PromptLabOpenRequested);
        assert!(state.prompt_lab().is_visible());
        assert!(state.view().dirty);
        assert!(effects.is_empty());
    }

    #[test]
    fn prompt_lab_close_clears_visible() {
        init_logging();
        let mut state = AppState::new();
        state.open_prompt_lab();
        let (state, effects) = update(state, Msg::PromptLabCloseRequested);
        assert!(!state.prompt_lab().is_visible());
        assert!(state.view().dirty);
        assert!(effects.is_empty());
    }

    #[test]
    fn prompt_lab_stage_selected_updates_stage() {
        init_logging();
        let state = AppState::new();
        let (state, _) = update(
            state,
            Msg::PromptLabStageSelected {
                stage: crate::prompt_lab::PromptLabStage::Summary,
            },
        );
        assert_eq!(
            state.prompt_lab().selected_stage(),
            crate::prompt_lab::PromptLabStage::Summary
        );
    }

    #[test]
    fn prompt_lab_input_changed_updates_input() {
        init_logging();
        let state = AppState::new();
        let (state, _) = update(
            state,
            Msg::PromptLabInputChanged {
                text: "hello world".to_string(),
            },
        );
        assert_eq!(state.prompt_lab().input(), "hello world");
    }

    #[test]
    fn prompt_lab_input_changed_sets_dirty() {
        init_logging();
        let state = AppState::new();
        let (state, _) = update(
            state,
            Msg::PromptLabInputChanged {
                text: "dirty text".to_string(),
            },
        );
        assert!(state.view().dirty);
    }

    #[test]
    fn prompt_lab_run_requested_with_nonempty_input_emits_effect_and_creates_pending_run() {
        init_logging();
        let mut state = AppState::new();
        prepare_type_url_snapshot(&mut state, "some article text");
        let (state, effects) = update(state, Msg::PromptLabRunRequested);
        assert_eq!(effects.len(), 1);
        assert!(matches!(effects[0], Effect::RequestLlmCompletion { .. }));
        assert_eq!(state.prompt_lab().run_count(), 1);
        assert!(state.prompt_lab().has_in_flight_run());
        // latest_run should exist and be Pending
        use crate::prompt_lab::PromptLabRunStatus;
        assert!(matches!(
            state.prompt_lab().latest_run().unwrap().status,
            PromptLabRunStatus::Pending { .. }
        ));
    }

    #[test]
    fn prompt_lab_run_requested_with_empty_input_emits_no_effects() {
        init_logging();
        let state = AppState::new(); // input is empty by default
        let (state, effects) = update(state, Msg::PromptLabRunRequested);
        assert!(effects.is_empty());
        assert_eq!(state.prompt_lab().run_count(), 0);
    }

    #[test]
    fn prompt_lab_run_requested_while_in_flight_emits_no_effects() {
        init_logging();
        let mut state = AppState::new();
        prepare_type_url_snapshot(&mut state, "text");
        // Dispatch first run
        let (mut state, _) = update(state, Msg::PromptLabRunRequested);
        assert!(state.prompt_lab().has_in_flight_run());
        // Change input and try again — should be blocked
        prepare_type_url_snapshot(&mut state, "different text");
        let (state, effects) = update(state, Msg::PromptLabRunRequested);
        assert!(effects.is_empty());
        assert_eq!(state.prompt_lab().run_count(), 1);
    }

    #[test]
    fn input_source_selection_updates_state_and_dirty() {
        init_logging();
        let state = AppState::new();
        let (state, _) = update(
            state,
            Msg::PromptLabInputSourceSelected {
                source: crate::prompt_lab::PromptLabInputSource::TypeUrl,
            },
        );
        assert_eq!(
            state.prompt_lab().selected_input_source(),
            crate::prompt_lab::PromptLabInputSource::TypeUrl
        );
        assert!(state.view().dirty);
    }

    #[test]
    fn url_input_change_marks_dirty() {
        init_logging();
        let (state, _) = update(
            AppState::new(),
            Msg::PromptLabUrlInputChanged {
                url: "https://example.com".to_string(),
            },
        );
        assert_eq!(state.prompt_lab().url_input(), "https://example.com");
        assert!(state.view().dirty);
    }

    #[test]
    fn resolve_requested_emits_effect() {
        init_logging();
        let mut state = AppState::new();
        state
            .prompt_lab_mut()
            .set_url_input("https://example.com".to_string());
        let (state, effects) = update(state, Msg::PromptLabResolveRequested);
        assert_eq!(effects.len(), 1);
        assert!(matches!(
            &effects[0],
            Effect::ResolvePromptLabInputFromUrl { .. }
        ));
        assert!(state.prompt_lab().pending_resolve_id().is_some());
    }

    #[test]
    fn resolve_requested_no_op_when_url_empty() {
        init_logging();
        let (state, effects) = update(AppState::new(), Msg::PromptLabResolveRequested);
        assert!(effects.is_empty());
        assert!(state.prompt_lab().pending_resolve_id().is_none());
    }

    #[test]
    fn input_resolved_stores_snapshot_and_marks_dirty() {
        init_logging();
        let mut state = AppState::new();
        state.prompt_lab_mut().begin_url_resolution(1);
        let (state, _) = update(
            state,
            Msg::PromptLabInputResolved {
                resolve_id: 1,
                result: Ok("snapshot".to_string()),
            },
        );
        assert_eq!(state.prompt_lab().resolved_url_snapshot(), Some("snapshot"));
        assert!(state.view().dirty);
    }

    #[test]
    fn stale_input_resolved_ignored() {
        init_logging();
        let mut state = AppState::new();
        state.prompt_lab_mut().begin_url_resolution(7);
        let (state, _) = update(
            state,
            Msg::PromptLabInputResolved {
                resolve_id: 8,
                result: Ok("ignored".to_string()),
            },
        );
        assert_eq!(state.prompt_lab().pending_resolve_id(), Some(7));
        assert!(state.prompt_lab().resolved_url_snapshot().is_none());
        assert!(!state.view().dirty);
    }

    #[test]
    fn run_requested_fromtriage_no_op_when_no_triage_articles() {
        init_logging();
        let (_state, effects) = update(AppState::new(), Msg::PromptLabRunRequested);
        assert!(effects.is_empty());
    }

    #[test]
    fn run_requested_uses_resolved_snapshot_after_job_selection() {
        init_logging();
        let (state, _) = update(
            AppState::new(),
            Msg::InputChanged("https://example.com/article/".to_string()),
        );
        let (mut state, _) = update(state, Msg::UrlsSubmitted);

        state.triage_mut().set_articles(vec![LoadedArticle {
            url: "https://example.com/article".to_string(),
            source_title: Some("Example".to_string()),
            prepared_text: "selected article prepared text".to_string(),
            content_hash: "hash-selected".to_string(),
        }]);
        state.triage_mut().transition_to_triaging();
        state.triage_mut().complete_article(
            0,
            crate::triage::ArticleTriageResult {
                category: "news".to_string(),
                priority: 3,
                tags: vec!["tag".to_string()],
                rationale: "ok".to_string(),
                input_tokens: 1,
                output_tokens: 1,
            },
        );

        let (state, effects) = update(state, Msg::JobSelected { job_id: 1 });
        let resolve_id = match effects.first() {
            Some(Effect::ResolvePromptLabInputFromUrl { resolve_id, .. }) => *resolve_id,
            other => panic!("expected resolve effect, got {other:?}"),
        };
        let (state, _) = update(
            state,
            Msg::PromptLabInputResolved {
                resolve_id,
                result: Ok("selected article prepared text".to_string()),
            },
        );
        let (_state, effects) = update(state, Msg::PromptLabRunRequested);
        assert_eq!(effects.len(), 1);
        match &effects[0] {
            Effect::RequestLlmCompletion { input_content, .. } => {
                assert_eq!(input_content, "selected article prepared text");
            }
            other => panic!("expected RequestLlmCompletion, got {other:?}"),
        }
    }

    #[test]
    fn job_selected_populates_prompt_lab_url_and_requests_resolve() {
        init_logging();
        let (state, _) = update(
            AppState::new(),
            Msg::InputChanged("https://example.com/article".to_string()),
        );
        let (state, _) = update(state, Msg::UrlsSubmitted);
        let (state, effects) = update(state, Msg::JobSelected { job_id: 1 });
        assert_eq!(
            state.prompt_lab().url_input(),
            "https://example.com/article"
        );
        assert!(effects
            .iter()
            .any(|effect| matches!(effect, Effect::ResolvePromptLabInputFromUrl { .. })));
    }

    #[test]
    fn run_requested_typeurl_no_op_when_snapshot_not_resolved() {
        init_logging();
        let (state, _effects) = update(
            AppState::new(),
            Msg::PromptLabInputSourceSelected {
                source: crate::prompt_lab::PromptLabInputSource::TypeUrl,
            },
        );
        let (state, _) = update(
            state,
            Msg::PromptLabUrlInputChanged {
                url: "https://example.com".to_string(),
            },
        );
        let (state, effects) = update(state, Msg::PromptLabRunRequested);
        assert!(effects.is_empty());
        assert!(state.prompt_lab().resolved_url_snapshot().is_none());
    }

    #[test]
    fn rerun_dispatches_same_parameters_as_original_run() {
        init_logging();
        let state = AppState::new();
        let (state, request_id) = dispatch_lab_run(state);
        let (state, _) = update(
            state,
            Msg::LlmCompleted {
                request_id,
                result: LlmResultKind::Success {
                    output_json: "{}".to_string(),
                    input_tokens: 5,
                    output_tokens: 5,
                    prompt_version: 1,
                    model_id: "model".to_string(),
                },
                metadata: Some(LlmRunMetadata::stub()),
            },
        );
        let (_state, effects) = update(state, Msg::PromptLabRerunRequested);
        assert_eq!(effects.len(), 1);
        if let Effect::RequestLlmCompletion { input_content, .. } = &effects[0] {
            assert_eq!(input_content, "article content");
        } else {
            panic!("expected RequestLlmCompletion");
        }
    }

    #[test]
    fn rerun_blocked_when_in_flight() {
        init_logging();
        let state = AppState::new();
        let (state, _) = dispatch_lab_run(state);
        let (_state, effects) = update(state, Msg::PromptLabRerunRequested);
        assert!(effects.is_empty());
    }

    #[test]
    fn prompt_lab_lifecycle_leaves_triage_session_unchanged() {
        init_logging();
        let mut state = AppState::new();
        let triage_phase = state.triage().phase().clone();
        prepare_type_url_snapshot(&mut state, "content");
        let (state, _) = update(state, Msg::PromptLabRunRequested);
        assert_eq!(state.triage().phase(), &triage_phase);
    }

    #[test]
    fn prompt_lab_lifecycle_leaves_briefing_session_unchanged() {
        init_logging();
        let mut state = AppState::new();
        let briefing_phase = state.briefing().phase().clone();
        prepare_type_url_snapshot(&mut state, "content");
        let (state, _) = update(state, Msg::PromptLabRunRequested);
        assert_eq!(state.briefing().phase(), &briefing_phase);
    }

    // ------------------------------------------------------------------
    // Substep D: LlmCompleted → Prompt Lab routing tests
    // ------------------------------------------------------------------

    fn prepare_type_url_snapshot(state: &mut AppState, snapshot: &str) {
        state
            .prompt_lab_mut()
            .select_input_source(crate::prompt_lab::PromptLabInputSource::TypeUrl);
        state
            .prompt_lab_mut()
            .set_url_input("https://example.com".to_string());
        let resolve_id = state.allocate_next_prompt_lab_resolve_id();
        state.prompt_lab_mut().begin_url_resolution(resolve_id);
        state
            .prompt_lab_mut()
            .finish_url_resolution(resolve_id, Ok(snapshot.to_string()));
    }

    fn dispatch_lab_run(state: AppState) -> (AppState, u64) {
        let mut state = state;
        prepare_type_url_snapshot(&mut state, "article content");
        let (state, effects) = update(state, Msg::PromptLabRunRequested);
        let request_id = effects
            .iter()
            .find_map(|e| {
                if let Effect::RequestLlmCompletion { request_id, .. } = e {
                    Some(*request_id)
                } else {
                    None
                }
            })
            .expect("expected RequestLlmCompletion effect");
        (state, request_id)
    }

    #[test]
    fn llm_completed_success_routes_to_lab_run() {
        init_logging();
        let state = AppState::new();
        let (state, request_id) = dispatch_lab_run(state);
        let (state, effects) = update(
            state,
            Msg::LlmCompleted {
                request_id,
                result: LlmResultKind::Success {
                    output_json: r#"{"priority":3}"#.to_string(),
                    input_tokens: 10,
                    output_tokens: 20,
                    prompt_version: 1,
                    model_id: "model-x".to_string(),
                },
                metadata: Some(LlmRunMetadata::stub()),
            },
        );
        assert!(effects.is_empty());
        use crate::prompt_lab::PromptLabRunStatus;
        assert!(matches!(
            state.prompt_lab().latest_run().unwrap().status,
            PromptLabRunStatus::Completed { .. }
        ));
        assert!(!state.prompt_lab().has_in_flight_run());
    }

    #[test]
    fn llm_completed_validation_failed_routes_to_lab_run_as_failed() {
        init_logging();
        let state = AppState::new();
        let (state, request_id) = dispatch_lab_run(state);
        let (state, _) = update(
            state,
            Msg::LlmCompleted {
                request_id,
                result: LlmResultKind::ValidationFailed {
                    reason: "bad json".to_string(),
                    raw_response: "garbage".to_string(),
                },
                metadata: None,
            },
        );
        use crate::prompt_lab::PromptLabRunStatus;
        assert!(matches!(
            state.prompt_lab().latest_run().unwrap().status,
            PromptLabRunStatus::Failed { .. }
        ));
        assert!(!state.prompt_lab().has_in_flight_run());
    }

    #[test]
    fn llm_completed_quota_exhausted_routes_to_lab_run_as_failed() {
        init_logging();
        let state = AppState::new();
        let (state, request_id) = dispatch_lab_run(state);
        let (state, _) = update(
            state,
            Msg::LlmCompleted {
                request_id,
                result: LlmResultKind::QuotaExhausted {
                    reason: "over limit".to_string(),
                },
                metadata: None,
            },
        );
        use crate::prompt_lab::PromptLabRunStatus;
        assert!(matches!(
            state.prompt_lab().latest_run().unwrap().status,
            PromptLabRunStatus::Failed { .. }
        ));
    }

    #[test]
    fn prompt_lab_history_cleared_removes_completed_and_failed() {
        init_logging();
        use crate::prompt_lab::PromptLabStage;
        let mut state = AppState::new();
        // Add a completed run manually
        let rid = state.allocate_next_llm_request_id();
        let run = state.allocate_next_prompt_lab_run_id();
        state.add_prompt_lab_pending_run(crate::state::PromptLabPendingRunRegistration {
            run_id: run,
            stage: PromptLabStage::Triage,
            prompt_id: PromptId::ArticleTriage,
            input_snapshot: "x".to_string(),
            request_id: rid,
            overrides: crate::prompt_lab::PromptLabRunOverrides::default(),
            compare_batch_id: None,
            compare_candidate_id: None,
        });
        state.complete_prompt_lab_run(run, "{}".to_string(), LlmRunMetadata::stub());
        state.consume_prompt_lab_ownership(rid);
        assert_eq!(state.prompt_lab().run_count(), 1);
        // Send clear message
        let (state, effects) = update(state, Msg::PromptLabHistoryCleared);
        assert_eq!(state.prompt_lab().run_count(), 0);
        assert!(effects.is_empty());
        // latest_run should be None after clearing all
        assert!(state.prompt_lab().latest_run().is_none());
    }

    // ------------------------------------------------------------------
    // Substep E: Isolation and non-regression tests
    // ------------------------------------------------------------------

    /// A full Prompt Lab lifecycle (open → stage → run → LlmCompleted) must not
    /// mutate briefing or triage state.
    #[test]
    fn prompt_lab_lifecycle_leaves_briefing_default() {
        init_logging();
        let state = AppState::new();
        let briefing_before = state.briefing().clone();

        // Open lab, change stage, dispatch a run
        let (state, _) = update(state, Msg::PromptLabOpenRequested);
        let (state, _) = update(
            state,
            Msg::PromptLabStageSelected {
                stage: crate::prompt_lab::PromptLabStage::Summary,
            },
        );
        let (mut state, _) = update(
            state,
            Msg::PromptLabInputChanged {
                text: "article text".to_string(),
            },
        );
        prepare_type_url_snapshot(&mut state, "article text");
        let (state, effects) = {
            let (s, e) = update(state, Msg::PromptLabRunRequested);
            (s, e)
        };
        let request_id = effects
            .iter()
            .find_map(|e| {
                if let Effect::RequestLlmCompletion { request_id, .. } = e {
                    Some(*request_id)
                } else {
                    None
                }
            })
            .unwrap();

        // Complete the run
        let (state, _) = update(
            state,
            Msg::LlmCompleted {
                request_id,
                result: LlmResultKind::Success {
                    output_json: r#"{"priority":3,"category":"news","tags":[],"rationale":"ok"}"#
                        .to_string(),
                    input_tokens: 5,
                    output_tokens: 10,
                    prompt_version: 1,
                    model_id: "m".to_string(),
                },
                metadata: None,
            },
        );

        assert_eq!(
            state.briefing().clone(),
            briefing_before,
            "briefing must be unchanged"
        );
    }

    #[test]
    fn prompt_lab_lifecycle_leaves_triage_default() {
        init_logging();
        let state = AppState::new();
        let triage_before = state.triage().clone();

        let (mut state, _) = update(state, Msg::PromptLabOpenRequested);
        prepare_type_url_snapshot(&mut state, "article text");
        let (state, effects) = update(state, Msg::PromptLabRunRequested);
        let request_id = effects
            .iter()
            .find_map(|e| {
                if let Effect::RequestLlmCompletion { request_id, .. } = e {
                    Some(*request_id)
                } else {
                    None
                }
            })
            .unwrap();

        let (state, _) = update(
            state,
            Msg::LlmCompleted {
                request_id,
                result: LlmResultKind::Failed {
                    reason: "timeout".to_string(),
                },
                metadata: None,
            },
        );

        assert_eq!(
            state.triage().clone(),
            triage_before,
            "triage must be unchanged"
        );
    }

    /// Triage and Prompt Lab both have active request_ids. Completing the triage request
    /// must not touch the lab run, and vice versa.
    #[test]
    fn triage_and_lab_coexistence_no_bleed() {
        init_logging();
        let mut state = AppState::new();
        state.set_triage_max_in_flight(1);
        let (mut state, triage_effects) = start_triage_for_test(state, loaded_triage_articles(1));
        let triage_req_id = triage_effects
            .iter()
            .find_map(|e| {
                if let Effect::RequestLlmCompletion { request_id, .. } = e {
                    Some(*request_id)
                } else {
                    None
                }
            })
            .expect("triage request");

        // Dispatch lab run → lab request_id = 2
        prepare_type_url_snapshot(&mut state, "article text");
        let (state, lab_effects) = update(state, Msg::PromptLabRunRequested);
        let lab_req_id = lab_effects
            .iter()
            .find_map(|e| {
                if let Effect::RequestLlmCompletion { request_id, .. } = e {
                    Some(*request_id)
                } else {
                    None
                }
            })
            .expect("lab request");

        assert_ne!(triage_req_id, lab_req_id, "request IDs must be distinct");

        // Complete the triage request — lab run must still be Pending
        let (state, _) = update(state, triage_success(triage_req_id));
        use crate::prompt_lab::PromptLabRunStatus;
        assert!(
            matches!(
                state.prompt_lab().latest_run().unwrap().status,
                PromptLabRunStatus::Pending { .. }
            ),
            "lab run must remain Pending after triage completes"
        );

        // Complete the lab request — triage must not gain extra completed articles
        let triage_completed_before = state.triage().completed_count();
        let (state, _) = update(
            state,
            Msg::LlmCompleted {
                request_id: lab_req_id,
                result: LlmResultKind::Success {
                    output_json: r#"{"priority":3,"category":"news","tags":[],"rationale":"ok"}"#
                        .to_string(),
                    input_tokens: 5,
                    output_tokens: 10,
                    prompt_version: 1,
                    model_id: "m".to_string(),
                },
                metadata: Some(LlmRunMetadata::stub()),
            },
        );
        assert_eq!(
            state.triage().completed_count(),
            triage_completed_before,
            "triage completed count must not change"
        );
        assert!(matches!(
            state.prompt_lab().latest_run().unwrap().status,
            PromptLabRunStatus::Completed { .. }
        ));
    }

    /// After N triage dispatches and M lab dispatches, all request_ids are distinct.
    #[test]
    fn id_namespace_all_request_ids_distinct() {
        init_logging();
        let mut state = AppState::new();
        state.set_triage_max_in_flight(3);
        let (mut state, triage_effects) = start_triage_for_test(state, loaded_triage_articles(3));
        let triage_ids: Vec<u64> = triage_effects
            .iter()
            .filter_map(|e| {
                if let Effect::RequestLlmCompletion { request_id, .. } = e {
                    Some(*request_id)
                } else {
                    None
                }
            })
            .collect();
        assert_eq!(triage_ids.len(), 3);

        // 2 lab runs
        prepare_type_url_snapshot(&mut state, "text1");
        let (state, e1) = update(state, Msg::PromptLabRunRequested);
        let lab_id1 = e1
            .iter()
            .find_map(|e| {
                if let Effect::RequestLlmCompletion { request_id, .. } = e {
                    Some(*request_id)
                } else {
                    None
                }
            })
            .unwrap();

        // Complete first lab run to allow second
        let (mut state, _) = update(
            state,
            Msg::LlmCompleted {
                request_id: lab_id1,
                result: LlmResultKind::Failed {
                    reason: "done".to_string(),
                },
                metadata: None,
            },
        );
        prepare_type_url_snapshot(&mut state, "text2");
        let (_, e2) = update(state, Msg::PromptLabRunRequested);
        let lab_id2 = e2
            .iter()
            .find_map(|e| {
                if let Effect::RequestLlmCompletion { request_id, .. } = e {
                    Some(*request_id)
                } else {
                    None
                }
            })
            .unwrap();

        let all_ids = [triage_ids.as_slice(), &[lab_id1, lab_id2]].concat();
        let unique: std::collections::HashSet<u64> = all_ids.iter().copied().collect();
        assert_eq!(
            unique.len(),
            all_ids.len(),
            "all request_ids must be distinct"
        );
    }

    // --- Step 3 per-run override tests ---

    #[test]
    fn prompt_lab_run_with_model_override_emits_effect_containing_it() {
        init_logging();
        use harvester_engine::llm::{ModelId, ProviderKind};
        let mut state = AppState::new();
        prepare_type_url_snapshot(&mut state, "some text");
        let override_model = ModelId::new(ProviderKind::OpenAi, "gpt-4o");
        state
            .prompt_lab_mut()
            .set_model_override(Some(override_model.clone()));
        let (state, effects) = update(state, Msg::PromptLabRunRequested);
        assert_eq!(effects.len(), 1);
        if let Effect::RequestLlmCompletion { model_override, .. } = &effects[0] {
            assert_eq!(model_override.as_ref(), Some(&override_model));
        } else {
            panic!("expected RequestLlmCompletion effect");
        }
        // Run record should store it too
        let run = state.prompt_lab().latest_run().unwrap();
        assert_eq!(run.model_override.as_ref(), Some(&override_model));
    }

    #[test]
    fn prompt_lab_run_with_prompt_version_override_emits_effect_containing_it() {
        init_logging();
        let mut state = AppState::new();
        prepare_type_url_snapshot(&mut state, "some text");
        state.prompt_lab_mut().set_prompt_version_override(Some(42));
        let (state, effects) = update(state, Msg::PromptLabRunRequested);
        assert_eq!(effects.len(), 1);
        if let Effect::RequestLlmCompletion { prompt_version, .. } = &effects[0] {
            assert_eq!(*prompt_version, Some(42));
        } else {
            panic!("expected RequestLlmCompletion effect");
        }
        let run = state.prompt_lab().latest_run().unwrap();
        assert_eq!(run.prompt_version_used, Some(42));
    }

    #[test]
    fn prompt_lab_run_record_stores_dispatched_override_values() {
        init_logging();
        use harvester_engine::llm::{ModelId, ProviderKind};
        let mut state = AppState::new();
        prepare_type_url_snapshot(&mut state, "some text");
        let override_model = ModelId::new(ProviderKind::OpenAi, "gpt-4o");
        state
            .prompt_lab_mut()
            .set_model_override(Some(override_model.clone()));
        state.prompt_lab_mut().set_prompt_version_override(Some(7));
        let (state, _) = update(state, Msg::PromptLabRunRequested);
        let run = state.prompt_lab().latest_run().unwrap();
        assert_eq!(run.model_override.as_ref(), Some(&override_model));
        assert_eq!(run.prompt_version_used, Some(7));
    }

    #[test]
    fn prompt_lab_run_none_overrides_behaves_as_before() {
        init_logging();
        let mut state = AppState::new();
        prepare_type_url_snapshot(&mut state, "some text");
        // No overrides set — defaults are None
        let (state, effects) = update(state, Msg::PromptLabRunRequested);
        assert_eq!(effects.len(), 1);
        if let Effect::RequestLlmCompletion { model_override, .. } = &effects[0] {
            assert!(
                model_override.is_none(),
                "model_override should be None when not set"
            );
        } else {
            panic!("expected RequestLlmCompletion effect");
        }
        let run = state.prompt_lab().latest_run().unwrap();
        assert!(run.model_override.is_none());
        assert!(run.prompt_version_used.is_none());
    }

    #[test]
    fn production_dispatch_paths_emit_none_model_override() {
        init_logging();
        // Triage path
        let (_, effects) = update(
            AppState::new(),
            Msg::TriageArticlesLoaded {
                articles: loaded_triage_articles(1),
            },
        );
        let llm_effect = effects
            .iter()
            .find(|e| matches!(e, Effect::RequestLlmCompletion { .. }));
        if let Some(Effect::RequestLlmCompletion { model_override, .. }) = llm_effect {
            assert!(
                model_override.is_none(),
                "triage path must emit None override"
            );
        }
    }

    #[test]
    fn dispatch_prompt_lab_run_uses_applied_context_overlay() {
        init_logging();
        let mut state = AppState::new();
        let prompt_id = PromptId::ArticleTriage;
        let mut contexts = HashMap::new();
        contexts.insert(prompt_id, vec![("prod".into(), "value".into())]);
        state.set_prompt_contexts(contexts);
        let context_snapshot = state.context_for(prompt_id).to_vec();
        state
            .prompt_lab_mut()
            .initialize_context_draft(prompt_id, &context_snapshot);
        state
            .prompt_lab_mut()
            .update_context_draft_text(prompt_id, "override=two".to_string());
        assert!(state.prompt_lab_mut().apply_context_draft(prompt_id));
        let effects = dispatch_prompt_lab_run(
            &mut state,
            PromptLabDispatchRequest {
                stage: PromptLabStage::Triage,
                prompt_id,
                input_snapshot: "input".to_string(),
                prompt_version: None,
                model_override: None,
                compare_batch_id: None,
                compare_candidate_id: None,
            },
        );
        assert_eq!(effects.len(), 1);
        if let Effect::RequestLlmCompletion { context, .. } = &effects[0] {
            assert_eq!(context, &vec![("override".into(), "two".into())]);
        } else {
            panic!("expected RequestLlmCompletion effect");
        }
    }

    #[test]
    fn dispatch_prompt_lab_run_uses_production_context_without_overlay() {
        init_logging();
        let mut state = AppState::new();
        let prompt_id = PromptId::ArticleTriage;
        let mut contexts = HashMap::new();
        contexts.insert(prompt_id, vec![("prod".into(), "value".into())]);
        state.set_prompt_contexts(contexts);
        // Do not apply any overlay.
        let effects = dispatch_prompt_lab_run(
            &mut state,
            PromptLabDispatchRequest {
                stage: PromptLabStage::Triage,
                prompt_id,
                input_snapshot: "input".to_string(),
                prompt_version: None,
                model_override: None,
                compare_batch_id: None,
                compare_candidate_id: None,
            },
        );
        assert_eq!(effects.len(), 1);
        if let Effect::RequestLlmCompletion { context, .. } = &effects[0] {
            assert_eq!(context, &vec![("prod".into(), "value".into())]);
        } else {
            panic!("expected RequestLlmCompletion effect");
        }
    }

    #[test]
    fn prompt_lab_save_requested_without_applied_context_emits_no_effect() {
        init_logging();
        let state = AppState::new();
        let (_state, effects) = update(state, Msg::PromptLabContextSaveRequested);
        assert!(effects.is_empty());
    }

    #[test]
    fn prompt_lab_save_requested_emits_save_effect() {
        init_logging();
        let mut state = AppState::new();
        let prompt_id = PromptId::ArticleTriage;
        let mut contexts = HashMap::new();
        contexts.insert(prompt_id, vec![("prod".into(), "value".into())]);
        state.set_prompt_contexts(contexts);
        let context_snapshot = state.context_for(prompt_id).to_vec();
        state
            .prompt_lab_mut()
            .initialize_context_draft(prompt_id, &context_snapshot);
        state
            .prompt_lab_mut()
            .update_context_draft_text(prompt_id, "override=two".to_string());
        assert!(state.prompt_lab_mut().apply_context_draft(prompt_id));
        let (_state, effects) = update(state, Msg::PromptLabContextSaveRequested);
        assert_eq!(effects.len(), 1);
        if let Effect::SavePromptContextFile { context_pairs, .. } = &effects[0] {
            assert_eq!(context_pairs, &vec![("override".into(), "two".into())]);
        } else {
            panic!("expected SavePromptContextFile effect");
        }
    }

    #[test]
    fn prompt_lab_compare_start_dispatches_first_candidate() {
        init_logging();
        let mut state = AppState::new();
        prepare_type_url_snapshot(&mut state, "compare input");
        let (state, _) = update(state, Msg::PromptLabCompareCurrentSettingsCaptured);
        let (state, _) = update(state, Msg::PromptLabCompareBaselineCaptured);
        let (_state, effects) = update(state, Msg::PromptLabCompareBatchStartRequested);
        assert_eq!(effects.len(), 1);
        assert!(matches!(effects[0], Effect::RequestLlmCompletion { .. }));
    }

    #[test]
    fn prompt_lab_compare_completion_advances_to_next_candidate() {
        init_logging();
        let mut state = AppState::new();
        prepare_type_url_snapshot(&mut state, "compare input");
        let (state, _) = update(state, Msg::PromptLabCompareCurrentSettingsCaptured);
        let (state, _) = update(state, Msg::PromptLabCompareBaselineCaptured);
        let (state, effects) = update(state, Msg::PromptLabCompareBatchStartRequested);
        let first_request_id = effects
            .iter()
            .find_map(|effect| {
                if let Effect::RequestLlmCompletion { request_id, .. } = effect {
                    Some(*request_id)
                } else {
                    None
                }
            })
            .expect("first request id");
        let (_state, effects) = update(
            state,
            Msg::LlmCompleted {
                request_id: first_request_id,
                result: LlmResultKind::Success {
                    output_json: triage_json(),
                    input_tokens: 5,
                    output_tokens: 3,
                    prompt_version: 1,
                    model_id: "m".to_string(),
                },
                metadata: Some(LlmRunMetadata::stub()),
            },
        );
        assert_eq!(effects.len(), 1);
        assert!(matches!(effects[0], Effect::RequestLlmCompletion { .. }));
    }
}
