use super::summary_cache_support::{
    build_summary_cache_key, log_summary_cache_completion_metadata,
    log_summary_cache_lookup_mismatch, log_summary_cache_run_summary, short_hash,
    summary_cache_key_error_reason,
};
use crate::briefing::{ArticleSummaryResult, BriefingResult, BriefingStoryResult};
use crate::prompt_lab::{PromptLabCompareBatchStatus, PromptLabRunStatus, PromptLabStage};
use crate::tabs::{AppTab, LeftTab};
use crate::triage::ArticleTriageResult;
use crate::update::signal_candidate::{handle_signal_candidate_completion, try_enqueue};
use crate::{AppState, Effect, LlmRequestState, LlmResultKind};
use engine_logging::{engine_info, engine_warn};
use harvester_engine::llm::prompt::PromptId;
use harvester_engine::llm::{validate_briefing, validate_summary, validate_triage, LlmRunMetadata};

pub(super) fn handle(
    state: &mut AppState,
    request_id: u64,
    result: LlmResultKind,
    metadata: Option<LlmRunMetadata>,
) -> Vec<Effect> {
    record_llm_result(state, request_id, &result);
    if let Some(run_metadata) = metadata.as_ref() {
        state.record_llm_usage_from_metadata(run_metadata);
    }

    let mut effects = Vec::new();
    if state
        .signal_candidate()
        .url_for_request(request_id)
        .is_some()
    {
        handle_signal_candidate_completion(state, request_id, &result, &mut effects);
    } else if let Some(article_idx) = state.briefing().find_article_by_request_id(request_id) {
        handle_summary_completion(state, article_idx, &result, &mut effects);
        super::briefing::dispatch_next_briefing_step(state, &mut effects);
    } else if let Some(article_idx) = state.triage().find_article_by_request_id(request_id) {
        handle_triage_completion(state, article_idx, &result, &mut effects);
        super::triage::dispatch_next_triage_step(state, &mut effects);
    } else if state.briefing().is_briefing_request(request_id) {
        handle_briefing_completion(state, &result, &mut effects);
    } else if let Some(run_id) = state.prompt_lab().ownership_for(request_id) {
        handle_prompt_lab_completion(state, request_id, run_id, &result, metadata, &mut effects);
    }

    effects
}

fn record_llm_result(state: &mut AppState, request_id: u64, result: &LlmResultKind) {
    let new_state = match result {
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
}

fn handle_summary_completion(
    state: &mut AppState,
    article_idx: usize,
    result: &LlmResultKind,
    effects: &mut Vec<Effect>,
) {
    match result {
        LlmResultKind::Success {
            output_json,
            input_tokens,
            output_tokens,
            prompt_version,
            resolved_model,
        } => match validate_summary(output_json) {
            Ok(summary) => {
                let summary_result = ArticleSummaryResult {
                    title: summary.title,
                    summary: summary.summary,
                    key_points: summary.key_points,
                    input_tokens: *input_tokens,
                    output_tokens: *output_tokens,
                    entities: summary.entities,
                };

                let content_hash = state.briefing().articles()[article_idx]
                    .content_hash
                    .clone();
                let context = state.context_for(PromptId::ArticleSummary).to_vec();
                let lookup_key = state.briefing().article_cache_key(article_idx).cloned();
                let run_metadata = state
                    .summary_cache_metadata()
                    .map(|(version, model)| (version, model.to_string()));

                state
                    .briefing_mut()
                    .complete_article(article_idx, summary_result.clone());
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
                            log_summary_cache_lookup_mismatch(article_idx, lookup, &store_key);
                        }

                        let completion_key = build_summary_cache_key(
                            &content_hash,
                            PromptId::ArticleSummary,
                            Some(*prompt_version),
                            Some(resolved_model.as_str()),
                            &context,
                        );
                        if let Ok(completion_key) = completion_key {
                            log_summary_cache_completion_metadata(
                                article_idx,
                                &store_key,
                                &completion_key,
                            );
                        }

                        let article_entities = summary_result.entities.clone();
                        let article_url = state.briefing().articles()[article_idx].url.clone();
                        let article_fetched_utc =
                            state.briefing().articles()[article_idx].fetched_utc.clone();
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
                        effects.push(Effect::UpsertEntityIndexEntry {
                            url: article_url.clone(),
                            fetched_utc: article_fetched_utc,
                            content_hash: Some(content_hash.clone()),
                            summary_entities: Some(article_entities),
                            themes: None,
                        });
                        let _ = try_enqueue(state, &article_url, effects);
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
        LlmResultKind::ValidationFailed { reason, .. } | LlmResultKind::Failed { reason } => {
            state
                .briefing_mut()
                .fail_article(article_idx, reason.clone());
        }
    }
}

fn handle_triage_completion(
    state: &mut AppState,
    article_idx: usize,
    result: &LlmResultKind,
    effects: &mut Vec<Effect>,
) {
    match result {
        LlmResultKind::Success {
            output_json,
            input_tokens,
            output_tokens,
            ..
        } => match validate_triage(output_json) {
            Ok(triage) => {
                let content_hash = state.triage().articles()[article_idx].content_hash.clone();
                let url = state.triage().articles()[article_idx].url.clone();
                let fetched_utc = state.triage().articles()[article_idx].fetched_utc.clone();
                let result = ArticleTriageResult {
                    category: triage.category,
                    priority: triage.priority.value(),
                    tags: triage.tags,
                    rationale: triage.rationale,
                    input_tokens: *input_tokens,
                    output_tokens: *output_tokens,
                };
                let triage_priority = result.priority;
                let themes = result.tags.clone();
                state
                    .triage_mut()
                    .complete_article(article_idx, result.clone());
                state.store_triage_result(&content_hash, result);
                effects.push(Effect::UpsertEntityIndexEntry {
                    url,
                    fetched_utc,
                    content_hash: Some(content_hash),
                    summary_entities: None,
                    themes: Some(themes),
                });
                let article_url = state.triage().articles()[article_idx].url.clone();
                let summary_ready = state.summary_result_for_url(&article_url).is_some();
                engine_info!(
                    "[signal-dispatch] triage completed url={} summary_ready={} triage_priority={} signal_state_present_before_enqueue={}",
                    article_url,
                    summary_ready,
                    triage_priority,
                    state
                        .signal_candidate()
                        .state_for(&state.triage().articles()[article_idx].url)
                        .is_some(),
                );
                let _ = try_enqueue(state, &article_url, effects);
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
        LlmResultKind::ValidationFailed { reason, .. } | LlmResultKind::Failed { reason } => {
            state.triage_mut().fail_article(article_idx, reason.clone());
        }
    }
}

fn handle_briefing_completion(
    state: &mut AppState,
    result: &LlmResultKind,
    effects: &mut Vec<Effect>,
) {
    match result {
        LlmResultKind::Success {
            output_json,
            input_tokens,
            output_tokens,
            ..
        } => match validate_briefing(output_json) {
            Ok(briefing) => {
                let top_stories = briefing
                    .top_stories
                    .into_iter()
                    .map(|story| BriefingStoryResult {
                        headline: story.headline,
                        body: story.body,
                    })
                    .collect();
                let result = BriefingResult {
                    executive_summary: briefing.executive_summary,
                    top_stories,
                    article_count: briefing.article_count,
                    input_tokens: *input_tokens,
                    output_tokens: *output_tokens,
                };
                state.briefing_mut().complete_briefing(result.clone());
                state.revert_preview_to_briefing();
                let now = chrono::Utc::now().to_rfc3339();
                if let Some(entry) =
                    crate::briefing::BriefingHistoryEntry::from_result(&result, &now)
                {
                    state.push_briefing_history(entry);
                    effects.push(Effect::SaveBriefingHistory {
                        entries: state.briefing_history().to_vec(),
                    });
                }
                effects.push(Effect::PersistSummaryCache {
                    cache: state.summary_cache().clone(),
                });
            }
            Err(err) => {
                engine_warn!("[briefing] briefing validation failed: {err}");
                state
                    .briefing_mut()
                    .fail(format!("validation failed: {err}"));
                state.revert_preview_to_briefing();
                effects.push(Effect::PersistSummaryCache {
                    cache: state.summary_cache().clone(),
                });
            }
        },
        LlmResultKind::QuotaExhausted { reason } | LlmResultKind::Failed { reason } => {
            state.briefing_mut().fail(reason.clone());
            state.revert_preview_to_briefing();
            effects.push(Effect::PersistSummaryCache {
                cache: state.summary_cache().clone(),
            });
        }
        LlmResultKind::ValidationFailed { reason, .. } => {
            state
                .briefing_mut()
                .fail(format!("validation failed: {reason}"));
            state.revert_preview_to_briefing();
            effects.push(Effect::PersistSummaryCache {
                cache: state.summary_cache().clone(),
            });
        }
    }

    log_summary_cache_run_summary(state);
    state.mark_dirty();
}

fn handle_prompt_lab_completion(
    state: &mut AppState,
    request_id: u64,
    run_id: crate::prompt_lab::PromptLabRunId,
    result: &LlmResultKind,
    metadata: Option<LlmRunMetadata>,
    effects: &mut Vec<Effect>,
) {
    let reason_from_result = |r: &LlmResultKind| -> String {
        match r {
            LlmResultKind::ValidationFailed {
                reason,
                raw_response,
            } => format!("validation failed: {reason}; response: {raw_response}"),
            LlmResultKind::QuotaExhausted { reason } => format!("quota exhausted: {reason}"),
            LlmResultKind::Failed { reason } => reason.clone(),
            LlmResultKind::Success { .. } => String::new(),
        }
    };

    let metadata_for_failure = metadata.clone();
    match result {
        LlmResultKind::Success {
            output_json,
            input_tokens,
            output_tokens,
            ..
        } => {
            engine_info!(
                "[prompt-lab] run completed run_id={} request_id={} tokens_in={} tokens_out={}",
                run_id.0,
                request_id,
                input_tokens,
                output_tokens
            );
            if let Some(run_metadata) = metadata {
                state.complete_prompt_lab_run(run_id, output_json.clone(), run_metadata);
            } else {
                engine_warn!(
                    "[prompt-lab] run completed but metadata missing run_id={} request_id={}",
                    run_id.0,
                    request_id
                );
                state.fail_prompt_lab_run(run_id, "metadata missing".to_string(), None);
            }
        }
        _ => {
            let reason = reason_from_result(result);
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

    if state.left_tab() == LeftTab::PromptLab {
        if let Some(run) = state.prompt_lab().run_by_id(run_id) {
            let target_tab = match run.stage {
                PromptLabStage::Triage => AppTab::Triage,
                PromptLabStage::Summary => AppTab::Summary,
                PromptLabStage::Briefing => AppTab::Briefing,
            };
            engine_info!(
                "[prompt-lab-auto-tab] lab run {} completed, switching right tab to {:?}",
                run_id.0,
                target_tab
            );
            state.select_tab(target_tab);
        }
    }

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
            return;
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
                    .map(|run| !matches!(run.status, PromptLabRunStatus::Pending { .. }))
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
                        .map(|run| matches!(run.status, PromptLabRunStatus::Failed { .. }))
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
            effects.extend(super::prompt_lab::dispatch_next_compare_candidate(
                state, batch_id,
            ));
        }
    }

    state.mark_dirty();
}
