use engine_logging::{engine_info, engine_warn};

use crate::state::TriageCacheLookupResult;
use crate::{
    briefing::{
        ArticleSummaryResult, BriefingPhase, BriefingResult, BriefingSession, BriefingThemeResult,
        CorpusFingerprint,
    },
    calc_left_width, context_hash,
    prompt_lab::PromptLabStage,
    triage::{ArticleTriageResult, TriagePhase, TriageSession},
    AppState, Effect, LlmRequestState, LlmResultKind, Msg, SessionState, StopPolicy,
    SummaryCacheKey, SummaryCacheKeyError, INPUT_PANEL_FIXED_WIDTH, MIN_JOBS_PANEL_WIDTH,
};
use harvester_engine::llm::prompt::{PromptId, PromptVersion};
use harvester_engine::llm::{validate_briefing, validate_summary, validate_triage};

// Left side is split into a fixed-width input panel plus a resizable jobs panel.
// Minimum width for the left region (PANEL_INPUT + PANEL_JOBS).
const MIN_LEFT_WIDTH: i32 = INPUT_PANEL_FIXED_WIDTH + MIN_JOBS_PANEL_WIDTH;
// Minimum width for the preview panel
const MIN_PREVIEW_WIDTH: i32 = 200;
// Total width occupied by splitter (width + margins)
const SPLITTER_TOTAL_WIDTH: i32 = 16; // 4px bar + 6px margin each side

/// Pure update function: applies a message to state and returns any effects.
#[allow(clippy::too_many_lines, clippy::cognitive_complexity, clippy::excessive_nesting)]
pub fn update(mut state: AppState, msg: Msg) -> (AppState, Vec<Effect>) {
    let effects = match msg {
        Msg::InputChanged(text) => {
            state.set_input_buffer(text);
            Vec::new()
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
            Vec::new()
        }
        Msg::RestoreCompletedJobs(entries) => {
            state.restore_completed_jobs(entries);
            Vec::new()
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
            input_content,
            context,
        } => {
            let request_id = state.allocate_next_llm_request_id();
            state.record_pending_llm_request(request_id, prompt_id);
            vec![Effect::RequestLlmCompletion {
                request_id,
                prompt_id,
                prompt_version,
                input_content,
                context,
            }]
        }
        Msg::LlmCompleted { request_id, result, metadata } => {
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
                        LlmResultKind::ValidationFailed { reason, raw_response } => {
                            format!("validation failed: {reason}; response: {raw_response}")
                        }
                        LlmResultKind::QuotaExhausted { reason } => {
                            format!("quota exhausted: {reason}")
                        }
                        LlmResultKind::Failed { reason } => reason.clone(),
                        LlmResultKind::Success { .. } => String::new(),
                    }
                };
                match &result {
                    LlmResultKind::Success { output_json, input_tokens, output_tokens, .. } => {
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
                            state.fail_prompt_lab_run(run_id, "metadata missing".to_string());
                        }
                    }
                    _ => {
                        let reason = reason_from_result(&result);
                        engine_warn!(
                            "[prompt-lab] run failed run_id={} request_id={} reason={}",
                            run_id.0, request_id, reason
                        );
                        state.fail_prompt_lab_run(run_id, reason);
                    }
                }
                state.consume_prompt_lab_ownership(request_id);
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
            vec![
                Effect::LoadPromptContexts,
                Effect::LoadLlmMetadata,
                Effect::LoadArticlesForBriefingPrereq,
            ]
        }
        Msg::BriefingPrereqArticlesLoaded { articles } => {
            engine_info!("[briefing-triage] prereq loaded count={}", articles.len());
            if articles.is_empty() {
                state
                    .briefing_mut()
                    .fail("No articles available".to_string());
                state.clear_briefing_orchestration();
                state.mark_dirty();
                return (state, Vec::new());
            }
            let prereq_fingerprint = CorpusFingerprint::from_articles(&articles);
            state.store_briefing_prereq_articles(articles.clone());
            let triage_reusable = matches!(state.triage().phase(), TriagePhase::Complete)
                && CorpusFingerprint::from_triage_results(state.triage()) == prereq_fingerprint;
            let mut effects = Vec::new();
            if triage_reusable {
                engine_info!("[briefing-triage] triage reused");
                on_triage_settled_for_briefing(&mut state, &mut effects);
            } else {
                engine_info!("[briefing-triage] triage rerun");
                state.triage_mut().reset_with_articles(articles);
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
            state.set_triage(TriageSession::new_loading(None));
            engine_info!("[triage] triage requested");
            vec![
                Effect::LoadPromptContexts,
                Effect::LoadLlmMetadata,
                Effect::LoadArticlesForTriage,
            ]
        }
        Msg::TriageArticlesLoaded { articles } => {
            if articles.is_empty() {
                state
                    .triage_mut()
                    .fail("no completed articles found".to_string());
                state.mark_dirty();
                return (state, Vec::new());
            }
            state.triage_mut().set_articles(articles);
            state.triage_mut().transition_to_triaging();
            state.mark_dirty();
            state.start_triage_cache_run();
            state.mark_triage_metadata_ready();
            let mut effects = Vec::new();
            dispatch_next_triage_step(&mut state, &mut effects);
            effects
        }
        Msg::TriageArticlesLoadFailed { reason } => {
            state.triage_mut().fail(reason);
            state.mark_dirty();
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
            // Continue with empty contexts (degraded but functional)
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
        Msg::PromptLabInputChanged { text } => {
            state.set_prompt_lab_input(text);
            Vec::new()
        }
        Msg::PromptLabRunRequested => {
            // Guard: one run at a time
            if state.prompt_lab().has_in_flight_run() {
                return (state, Vec::new());
            }
            // Guard: input must not be empty
            if state.prompt_lab().input().is_empty() {
                return (state, Vec::new());
            }
            let stage = state.prompt_lab().selected_stage();
            let prompt_id = prompt_id_for_stage(stage);
            let input = state.prompt_lab().input().to_string();
            let request_id = state.allocate_next_llm_request_id();
            let run_id = state.allocate_next_prompt_lab_run_id();
            let prompt_version = state.active_version_for(prompt_id);
            let context = state.context_for(prompt_id).to_vec();
            state.record_pending_llm_request(request_id, prompt_id);
            state.add_prompt_lab_pending_run(run_id, stage, prompt_id, input.clone(), request_id);
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
                input_content: input,
                context,
            }]
        }
        Msg::PromptLabHistoryCleared => {
            state.clear_prompt_lab_history();
            Vec::new()
        }
        Msg::Tick | Msg::NoOp => Vec::new(),
    };

    (state, effects)
}

fn prompt_id_for_stage(stage: PromptLabStage) -> PromptId {
    match stage {
        PromptLabStage::Triage => PromptId::ArticleTriage,
        PromptLabStage::Summary => PromptId::ArticleSummary,
        PromptLabStage::Briefing => PromptId::AggregateBriefing,
    }
}

fn parse_urls(raw: &str) -> Vec<String> {
    raw.lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .map(ToOwned::to_owned)
        .collect()
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
                state.mark_dirty();
                continue;
            }
            TriageCacheLookupResult::Miss => {
                state.record_triage_cache_miss();
                engine_info!("[triage-cache] miss content_hash={}", content_hash_short);
            }
            TriageCacheLookupResult::KeyUnavailable => {
                state.record_triage_cache_key_unavailable();
                engine_info!(
                    "[triage-cache] key-unavailable content_hash={}",
                    content_hash_short
                );
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
            input_content: prepared_text,
            context,
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
                    input_content: prepared_text,
                    context,
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
                    input_content: prepared_text,
                    context,
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
        input_content: collection_text,
        context,
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
        let articles = vec![
            LoadedArticle {
                url: "https://example.com/a".to_string(),
                source_title: Some("Article A".to_string()),
                prepared_text: "Article A text".to_string(),
                content_hash: "hash-a".to_string(),
            },
            LoadedArticle {
                url: "https://example.com/b".to_string(),
                source_title: Some("Article B".to_string()),
                prepared_text: "Article B text".to_string(),
                content_hash: "hash-b".to_string(),
            },
        ];
        (articles, "Collection text".to_string())
    }

    fn loaded_single_article() -> (Vec<LoadedArticle>, String) {
        let articles = vec![LoadedArticle {
            url: "https://example.com/a".to_string(),
            source_title: Some("Article A".to_string()),
            prepared_text: "Article A text".to_string(),
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
                Effect::LoadLlmMetadata,
                Effect::LoadArticlesForBriefingPrereq
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

        assert_eq!(
            effects,
            vec![Effect::RequestLlmCompletion {
                request_id: 3,
                prompt_id: PromptId::ArticleSummary,
                prompt_version: None,
                input_content: "Article A text".to_string(),
                context: Vec::new(),
            }]
        );
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

        assert_eq!(
            effects,
            vec![Effect::RequestLlmCompletion {
                request_id: 4,
                prompt_id: PromptId::ArticleSummary,
                prompt_version: None,
                input_content: "Article B text".to_string(),
                context: Vec::new(),
            }]
        );

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
                input_content: "Collection text".to_string(),
                context: Vec::new(),
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
                input_content: "Collection text".to_string(),
                context: Vec::new(),
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
                Effect::LoadLlmMetadata,
                Effect::LoadArticlesForBriefingPrereq
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
                input_content: "Collection text".to_string(),
                context: Vec::new(),
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
        assert_eq!(state.left_panel_width(), 360);
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
                prepared_text: format!("Article {i} text"),
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

    #[test]
    fn triage_clicked_emits_load_effects() {
        init_logging();
        let state = AppState::new();
        let (_state, effects) = update(state, Msg::TriageClicked);
        assert!(effects.contains(&Effect::LoadArticlesForTriage));
        assert!(effects.contains(&Effect::LoadLlmMetadata));
        assert!(effects.contains(&Effect::LoadPromptContexts));
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
        let (_, _) = update(state.clone(), Msg::TriageClicked);

        // Simulate loading 3 articles with limit=2: expect 2 effects emitted.
        state.set_triage(crate::triage::TriageSession::new_loading(None));
        let (state, effects) = update(
            state,
            Msg::TriageArticlesLoaded {
                articles: loaded_triage_articles(3),
            },
        );
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
        state.set_triage(crate::triage::TriageSession::new_loading(None));

        // Load 3 articles → 2 in-flight (ids 1, 2)
        let (state, _) = update(
            state,
            Msg::TriageArticlesLoaded {
                articles: loaded_triage_articles(3),
            },
        );
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
        state.set_triage(crate::triage::TriageSession::new_loading(None));

        // Load 3 articles → all 3 in-flight (request_ids 1, 2, 3)
        let (state, _) = update(
            state,
            Msg::TriageArticlesLoaded {
                articles: loaded_triage_articles(3),
            },
        );
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
        state.set_triage(crate::triage::TriageSession::new_loading(None));

        let (state, _) = update(
            state,
            Msg::TriageArticlesLoaded {
                articles: loaded_triage_articles(3),
            },
        );
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
        state.set_triage(crate::triage::TriageSession::new_loading(None));

        let (state, _) = update(
            state,
            Msg::TriageArticlesLoaded {
                articles: loaded_triage_articles(3),
            },
        );

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
        let (state, _) = update(state, Msg::PromptLabStageSelected { stage: crate::prompt_lab::PromptLabStage::Summary });
        assert_eq!(state.prompt_lab().selected_stage(), crate::prompt_lab::PromptLabStage::Summary);
    }

    #[test]
    fn prompt_lab_input_changed_updates_input() {
        init_logging();
        let state = AppState::new();
        let (state, _) = update(state, Msg::PromptLabInputChanged { text: "hello world".to_string() });
        assert_eq!(state.prompt_lab().input(), "hello world");
    }

    #[test]
    fn prompt_lab_run_requested_with_nonempty_input_emits_effect_and_creates_pending_run() {
        init_logging();
        let mut state = AppState::new();
        state.set_prompt_lab_input("some article text".to_string());
        let (state, effects) = update(state, Msg::PromptLabRunRequested);
        assert_eq!(effects.len(), 1);
        assert!(matches!(effects[0], Effect::RequestLlmCompletion { .. }));
        assert_eq!(state.prompt_lab().run_count(), 1);
        assert!(state.prompt_lab().has_in_flight_run());
        // latest_run should exist and be Pending
        use crate::prompt_lab::PromptLabRunStatus;
        assert!(matches!(state.prompt_lab().latest_run().unwrap().status, PromptLabRunStatus::Pending { .. }));
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
        state.set_prompt_lab_input("text".to_string());
        // Dispatch first run
        let (mut state, _) = update(state, Msg::PromptLabRunRequested);
        assert!(state.prompt_lab().has_in_flight_run());
        // Change input and try again — should be blocked
        state.set_prompt_lab_input("different text".to_string());
        let (state, effects) = update(state, Msg::PromptLabRunRequested);
        assert!(effects.is_empty());
        assert_eq!(state.prompt_lab().run_count(), 1);
    }

    // ------------------------------------------------------------------
    // Substep D: LlmCompleted → Prompt Lab routing tests
    // ------------------------------------------------------------------

    fn dispatch_lab_run(state: AppState) -> (AppState, u64) {
        let mut state = state;
        state.set_prompt_lab_input("article content".to_string());
        let (state, effects) = update(state, Msg::PromptLabRunRequested);
        // Extract the request_id from the emitted effect
        let request_id = effects.iter().find_map(|e| {
            if let Effect::RequestLlmCompletion { request_id, .. } = e {
                Some(*request_id)
            } else {
                None
            }
        }).expect("expected RequestLlmCompletion effect");
        (state, request_id)
    }

    #[test]
    fn llm_completed_success_routes_to_lab_run() {
        init_logging();
        let state = AppState::new();
        let (state, request_id) = dispatch_lab_run(state);
        let (state, effects) = update(state, Msg::LlmCompleted {
            request_id,
            result: LlmResultKind::Success {
                output_json: r#"{"priority":3}"#.to_string(),
                input_tokens: 10,
                output_tokens: 20,
                prompt_version: 1,
                model_id: "model-x".to_string(),
            },
            metadata: Some(LlmRunMetadata::stub()),
        });
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
        let (state, _) = update(state, Msg::LlmCompleted {
            request_id,
            result: LlmResultKind::ValidationFailed {
                reason: "bad json".to_string(),
                raw_response: "garbage".to_string(),
            },
            metadata: None,
        });
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
        let (state, _) = update(state, Msg::LlmCompleted {
            request_id,
            result: LlmResultKind::QuotaExhausted { reason: "over limit".to_string() },
            metadata: None,
        });
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
        state.add_prompt_lab_pending_run(run, PromptLabStage::Triage, PromptId::ArticleTriage, "x".to_string(), rid);
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
        let (state, _) = update(state, Msg::PromptLabStageSelected { stage: crate::prompt_lab::PromptLabStage::Summary });
        let (state, _) = update(state, Msg::PromptLabInputChanged { text: "article text".to_string() });
        let (state, effects) = {
            let (s, e) = update(state, Msg::PromptLabRunRequested);
            (s, e)
        };
        let request_id = effects.iter().find_map(|e| {
            if let Effect::RequestLlmCompletion { request_id, .. } = e { Some(*request_id) } else { None }
        }).unwrap();

        // Complete the run
        let (state, _) = update(state, Msg::LlmCompleted {
            request_id,
            result: LlmResultKind::Success {
                output_json: r#"{"priority":3,"category":"news","tags":[],"rationale":"ok"}"#.to_string(),
                input_tokens: 5,
                output_tokens: 10,
                prompt_version: 1,
                model_id: "m".to_string(),
            },
            metadata: None,
        });

        assert_eq!(state.briefing().clone(), briefing_before, "briefing must be unchanged");
    }

    #[test]
    fn prompt_lab_lifecycle_leaves_triage_default() {
        init_logging();
        let state = AppState::new();
        let triage_before = state.triage().clone();

        let (mut state, _) = update(state, Msg::PromptLabOpenRequested);
        state.set_prompt_lab_input("article text".to_string());
        let (state, effects) = update(state, Msg::PromptLabRunRequested);
        let request_id = effects.iter().find_map(|e| {
            if let Effect::RequestLlmCompletion { request_id, .. } = e { Some(*request_id) } else { None }
        }).unwrap();

        let (state, _) = update(state, Msg::LlmCompleted {
            request_id,
            result: LlmResultKind::Failed { reason: "timeout".to_string() },
            metadata: None,
        });

        assert_eq!(state.triage().clone(), triage_before, "triage must be unchanged");
    }

    /// Triage and Prompt Lab both have active request_ids. Completing the triage request
    /// must not touch the lab run, and vice versa.
    #[test]
    fn triage_and_lab_coexistence_no_bleed() {
        init_logging();
        let mut state = AppState::new();
        state.set_triage_max_in_flight(1);
        state.set_triage(crate::triage::TriageSession::new_loading(None));

        // Dispatch triage for 1 article → triage request_id = 1
        let (mut state, triage_effects) = update(
            state,
            Msg::TriageArticlesLoaded { articles: loaded_triage_articles(1) },
        );
        let triage_req_id = triage_effects.iter().find_map(|e| {
            if let Effect::RequestLlmCompletion { request_id, .. } = e { Some(*request_id) } else { None }
        }).expect("triage request");

        // Dispatch lab run → lab request_id = 2
        state.set_prompt_lab_input("article text".to_string());
        let (state, lab_effects) = update(state, Msg::PromptLabRunRequested);
        let lab_req_id = lab_effects.iter().find_map(|e| {
            if let Effect::RequestLlmCompletion { request_id, .. } = e { Some(*request_id) } else { None }
        }).expect("lab request");

        assert_ne!(triage_req_id, lab_req_id, "request IDs must be distinct");

        // Complete the triage request — lab run must still be Pending
        let (state, _) = update(state, triage_success(triage_req_id));
        use crate::prompt_lab::PromptLabRunStatus;
        assert!(matches!(
            state.prompt_lab().latest_run().unwrap().status,
            PromptLabRunStatus::Pending { .. }
        ), "lab run must remain Pending after triage completes");

        // Complete the lab request — triage must not gain extra completed articles
        let triage_completed_before = state.triage().completed_count();
        let (state, _) = update(state, Msg::LlmCompleted {
            request_id: lab_req_id,
            result: LlmResultKind::Success {
                output_json: r#"{"priority":3,"category":"news","tags":[],"rationale":"ok"}"#.to_string(),
                input_tokens: 5,
                output_tokens: 10,
                prompt_version: 1,
                model_id: "m".to_string(),
            },
            metadata: Some(LlmRunMetadata::stub()),
        });
        assert_eq!(state.triage().completed_count(), triage_completed_before, "triage completed count must not change");
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
        state.set_triage(crate::triage::TriageSession::new_loading(None));

        // 3 triage articles → 3 triage request_ids
        let (mut state, triage_effects) = update(
            state,
            Msg::TriageArticlesLoaded { articles: loaded_triage_articles(3) },
        );
        let triage_ids: Vec<u64> = triage_effects.iter().filter_map(|e| {
            if let Effect::RequestLlmCompletion { request_id, .. } = e { Some(*request_id) } else { None }
        }).collect();
        assert_eq!(triage_ids.len(), 3);

        // 2 lab runs
        state.set_prompt_lab_input("text1".to_string());
        let (state, e1) = update(state, Msg::PromptLabRunRequested);
        let lab_id1 = e1.iter().find_map(|e| if let Effect::RequestLlmCompletion { request_id, .. } = e { Some(*request_id) } else { None }).unwrap();

        // Complete first lab run to allow second
        let (mut state, _) = update(state, Msg::LlmCompleted {
            request_id: lab_id1,
            result: LlmResultKind::Failed { reason: "done".to_string() },
            metadata: None,
        });
        state.set_prompt_lab_input("text2".to_string());
        let (_, e2) = update(state, Msg::PromptLabRunRequested);
        let lab_id2 = e2.iter().find_map(|e| if let Effect::RequestLlmCompletion { request_id, .. } = e { Some(*request_id) } else { None }).unwrap();

        let all_ids = [triage_ids.as_slice(), &[lab_id1, lab_id2]].concat();
        let unique: std::collections::HashSet<u64> = all_ids.iter().copied().collect();
        assert_eq!(unique.len(), all_ids.len(), "all request_ids must be distinct");
    }
}
