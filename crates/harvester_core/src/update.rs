use engine_logging::{engine_info, engine_warn};

use crate::{
    briefing::{ArticleSummaryResult, BriefingResult, BriefingSession, BriefingThemeResult},
    calc_left_width,
    triage::{ArticleTriageResult, TriageSession},
    AppState, Effect, LlmRequestState, LlmResultKind, Msg, SessionState, StopPolicy,
    INPUT_PANEL_FIXED_WIDTH, MIN_JOBS_PANEL_WIDTH,
};
use harvester_engine::llm::prompt::PromptId;
use harvester_engine::llm::{validate_briefing, validate_summary, validate_triage};

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
        Msg::LlmCompleted { request_id, result } => {
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
                        ..
                    } => match validate_summary(output_json) {
                        Ok(summary) => {
                            state.briefing_mut().complete_article(
                                article_idx,
                                ArticleSummaryResult {
                                    title: summary.title,
                                    summary: summary.summary,
                                    key_points: summary.key_points,
                                    input_tokens: *input_tokens,
                                    output_tokens: *output_tokens,
                                },
                            );
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
                            state.triage_mut().complete_article(
                                article_idx,
                                ArticleTriageResult {
                                    category: triage.category,
                                    priority: triage.priority.value(),
                                    tags: triage.tags,
                                    rationale: triage.rationale,
                                    input_tokens: *input_tokens,
                                    output_tokens: *output_tokens,
                                },
                            );
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
                        }
                        Err(err) => {
                            engine_warn!("[briefing] briefing validation failed: {err}");
                            state.briefing_mut().complete_without_briefing();
                        }
                    },
                    _ => {
                        state.briefing_mut().complete_without_briefing();
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
            state.set_briefing(BriefingSession::new_loading(None));
            engine_info!("[briefing] briefing requested");
            vec![
                Effect::LoadPromptContexts,
                Effect::LoadLlmMetadata,
                Effect::LoadArticlesForBriefing,
            ]
        }
        Msg::ArticlesLoaded {
            articles,
            collection_text,
        } => {
            if articles.is_empty() {
                state
                    .briefing_mut()
                    .fail("no completed articles found".to_string());
                state.mark_dirty();
                return (state, Vec::new());
            }
            state.briefing_mut().set_articles(articles, collection_text);
            state.briefing_mut().transition_to_summarizing();
            state.mark_dirty();
            let mut effects = Vec::new();
            dispatch_next_briefing_step(&mut state, &mut effects);
            effects
        }
        Msg::ArticlesLoadFailed { reason } => {
            state.briefing_mut().fail(reason);
            state.mark_dirty();
            Vec::new()
        }
        Msg::TriageClicked => {
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
            state.mark_dirty();
            Vec::new()
        }
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
        Msg::Tick | Msg::NoOp => Vec::new(),
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

fn dispatch_next_triage_step(state: &mut AppState, effects: &mut Vec<Effect>) {
    if let Some(next_idx) = state.triage().next_pending_index() {
        let prepared_text = state.triage().articles()[next_idx].prepared_text.clone();
        let request_id = state.allocate_next_llm_request_id();
        state.record_pending_llm_request(request_id, PromptId::ArticleTriage);
        state.triage_mut().start_article(next_idx, request_id);

        let context = state.context_for(PromptId::ArticleTriage).to_vec();

        effects.push(Effect::RequestLlmCompletion {
            request_id,
            prompt_id: PromptId::ArticleTriage,
            prompt_version: None,
            input_content: prepared_text,
            context,
        });
        state.mark_dirty();
        return;
    }
    if state.triage().completed_count() == 0 {
        state
            .triage_mut()
            .fail("all triage attempts failed".to_string());
    } else {
        state.triage_mut().complete();
    }
    state.mark_dirty();
}

fn dispatch_next_briefing_step(state: &mut AppState, effects: &mut Vec<Effect>) {
    if let Some(next_idx) = state.briefing().next_pending_index() {
        let prepared_text = state.briefing().articles()[next_idx].prepared_text.clone();
        let request_id = state.allocate_next_llm_request_id();
        state.record_pending_llm_request(request_id, PromptId::ArticleSummary);
        state.briefing_mut().start_article(next_idx, request_id);

        let context = state.context_for(PromptId::ArticleSummary).to_vec();

        effects.push(Effect::RequestLlmCompletion {
            request_id,
            prompt_id: PromptId::ArticleSummary,
            prompt_version: None,
            input_content: prepared_text,
            context,
        });
        state.mark_dirty();
        return;
    }

    if state.briefing().completed_summary_count() == 0 {
        state
            .briefing_mut()
            .fail("all article summaries failed".to_string());
        state.mark_dirty();
        return;
    }

    let collection_text = match state.briefing().collection_text() {
        Some(text) => text.to_string(),
        None => {
            state
                .briefing_mut()
                .fail("missing briefing collection".to_string());
            state.mark_dirty();
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

#[cfg(test)]
mod tests {
    use std::sync::Once;

    use super::*;
    use crate::briefing::{ArticleSummaryState, BriefingPhase, LoadedArticle};

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

    fn summary_json(title: &str) -> String {
        format!("{{\"title\":\"{title}\",\"summary\":\"Summary\",\"key_points\":[\"p1\"]}}")
    }

    fn briefing_json(article_count: u32) -> String {
        format!(
            "{{\"executive_summary\":\"Exec\",\"themes\":[{{\"name\":\"Theme\",\"description\":\"Desc\"}}],\"article_count\":{article_count}}}"
        )
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
                Effect::LoadArticlesForBriefing
            ]
        );
        assert_eq!(state.briefing().phase(), &BriefingPhase::LoadingArticles);
    }

    #[test]
    fn articles_loaded_dispatches_first_summary() {
        init_logging();
        let state = AppState::new();
        let (state, _effects) = update(state, Msg::GenerateBriefingClicked);
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
                request_id: 1,
                prompt_id: PromptId::ArticleSummary,
                prompt_version: None,
                input_content: "Article A text".to_string(),
                context: Vec::new(),
            }]
        );
        assert!(matches!(
            state.briefing().phase(),
            BriefingPhase::Summarizing { .. }
        ));
        assert!(matches!(
            state.briefing().articles()[0].summary_state,
            ArticleSummaryState::InProgress { request_id: 1 }
        ));
    }

    #[test]
    fn summary_completion_advances_and_generates_briefing() {
        init_logging();
        let state = AppState::new();
        let (state, _effects) = update(state, Msg::GenerateBriefingClicked);
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
                request_id: 1,
                result: LlmResultKind::Success {
                    output_json: summary_json("Article A"),
                    input_tokens: 10,
                    output_tokens: 5,
                    prompt_version: 1,
                    model_id: "test-model".to_string(),
                },
            },
        );

        assert_eq!(
            effects,
            vec![Effect::RequestLlmCompletion {
                request_id: 2,
                prompt_id: PromptId::ArticleSummary,
                prompt_version: None,
                input_content: "Article B text".to_string(),
                context: Vec::new(),
            }]
        );

        let (state, effects) = update(
            state,
            Msg::LlmCompleted {
                request_id: 2,
                result: LlmResultKind::Success {
                    output_json: summary_json("Article B"),
                    input_tokens: 10,
                    output_tokens: 5,
                    prompt_version: 1,
                    model_id: "test-model".to_string(),
                },
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

        let (state, effects) = update(
            state,
            Msg::LlmCompleted {
                request_id: 3,
                result: LlmResultKind::Success {
                    output_json: briefing_json(2),
                    input_tokens: 20,
                    output_tokens: 8,
                    prompt_version: 1,
                    model_id: "test-model".to_string(),
                },
            },
        );

        assert!(effects.is_empty());
        assert_eq!(state.briefing().phase(), &BriefingPhase::Complete);
        assert!(state.briefing().briefing_result().is_some());
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
}
