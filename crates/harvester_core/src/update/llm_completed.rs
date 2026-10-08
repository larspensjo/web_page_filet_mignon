use super::summary_cache_support::{
    build_summary_cache_key, log_summary_cache_completion_metadata,
    log_summary_cache_lookup_mismatch, short_hash, summary_cache_key_error_reason,
};
use crate::briefing::ArticleSummaryResult;
use crate::state::ModelDispatchHalt;
use crate::triage::ArticleTriageResult;
use crate::update::signal_candidate::{handle_signal_candidate_completion, try_enqueue};
use crate::{AppState, Effect, LlmRequestState, LlmResultKind};
use engine_logging::{engine_info, engine_warn};
use harvester_engine::llm::prompt::PromptId;
use harvester_engine::llm::{validate_summary, validate_triage, LlmRunMetadata, QuotaOrigin};

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

    if state
        .signal_candidate()
        .url_for_request(request_id)
        .is_some()
    {
        note_article_model_result(state, &result);
        handle_signal_candidate_completion(state, request_id, &result);
    } else if let Some(article_idx) = state.briefing().find_article_by_request_id(request_id) {
        note_article_model_result(state, &result);
        handle_summary_completion(state, article_idx, &result);
    } else if let Some(article_idx) = state.triage().find_article_by_request_id(request_id) {
        note_article_model_result(state, &result);
        handle_triage_completion(state, article_idx, &result);
        let article = &state.triage().articles()[article_idx];
        let identity = (article.url.clone(), article.content_hash.clone());
        super::waves::triage_changed(state, &identity.0, &identity.1);
    }

    Vec::new()
}

fn note_article_model_result(state: &mut AppState, result: &LlmResultKind) {
    match result {
        LlmResultKind::QuotaExhausted { .. } => note_owned_quota(state, result),
        LlmResultKind::RateLimited { reason } => {
            if state.note_provider_rate_limited() {
                state.halt_model_dispatch(ModelDispatchHalt::RateLimit(reason.clone()));
            }
        }
        LlmResultKind::Success { .. } => state.note_owned_llm_success(),
        _ => {}
    }
}

fn note_owned_quota(state: &mut AppState, result: &LlmResultKind) {
    if let LlmResultKind::QuotaExhausted { reason, origin } = result {
        match origin {
            QuotaOrigin::SessionBudget => {
                state.halt_model_dispatch(ModelDispatchHalt::SessionQuota(reason.clone()));
            }
            QuotaOrigin::Provider => {
                state.halt_model_dispatch(ModelDispatchHalt::ProviderCredits(reason.clone()));
            }
        }
    }
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
        LlmResultKind::QuotaExhausted { reason, .. } => LlmRequestState::Failed {
            reason: reason.clone(),
        },
        LlmResultKind::RateLimited { reason } | LlmResultKind::Failed { reason } => {
            LlmRequestState::Failed {
                reason: reason.clone(),
            }
        }
    };

    if state.llm_request_state(request_id).is_some() {
        state.record_llm_result(request_id, new_state);
    } else {
        engine_warn!("LLM completion for unknown request_id {request_id}");
    }
}

fn handle_summary_completion(state: &mut AppState, article_idx: usize, result: &LlmResultKind) {
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

                        let article_url = state.briefing().articles()[article_idx].url.clone();

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
                        let _ = try_enqueue(state, &article_url);
                    }
                    Err(err) => {
                        engine_warn!(
                            "[summary-cache] article={} skip storing result: {}",
                            article_idx,
                            summary_cache_key_error_reason(&err)
                        );
                    }
                }
            }
            Err(err) => {
                state
                    .briefing_mut()
                    .fail_article(article_idx, format!("validation failed: {err}"));
            }
        },
        LlmResultKind::QuotaExhausted { reason, .. } => {
            engine_info!("[briefing] quota exhausted during summaries: {reason}");
            state
                .briefing_mut()
                .fail_article(article_idx, reason.clone());
        }
        LlmResultKind::RateLimited { reason }
        | LlmResultKind::ValidationFailed { reason, .. }
        | LlmResultKind::Failed { reason } => {
            state
                .briefing_mut()
                .fail_article(article_idx, reason.clone());
        }
    }
}

fn handle_triage_completion(state: &mut AppState, article_idx: usize, result: &LlmResultKind) {
    match result {
        LlmResultKind::Success {
            output_json,
            input_tokens,
            output_tokens,
            ..
        } => match validate_triage(output_json) {
            Ok(triage) => {
                let content_hash = state.triage().articles()[article_idx].content_hash.clone();

                let result = ArticleTriageResult {
                    category: triage.category,
                    priority: triage.priority.value(),
                    tags: triage.tags,
                    rationale: triage.rationale,
                    input_tokens: *input_tokens,
                    output_tokens: *output_tokens,
                };
                let triage_priority = result.priority;

                let triage_model =
                    state.store_triage_result_with_model(&content_hash, result.clone());
                state.triage_mut().complete_article_with_model(
                    article_idx,
                    result.clone(),
                    triage_model,
                );
                let article_url = state.triage().articles()[article_idx].url.clone();
                engine_info!(
                    "[signal-dispatch] triage completed url={} triage_priority={} signal_state_present_before_enqueue={}",
                    article_url,
                    triage_priority,
                    state
                        .signal_candidate()
                        .state_for(&state.triage().articles()[article_idx].url)
                        .is_some(),
                );
                let _ = try_enqueue(state, &article_url);
            }
            Err(err) => {
                state
                    .triage_mut()
                    .fail_article(article_idx, format!("validation: {err}"));
            }
        },
        LlmResultKind::QuotaExhausted { reason, .. } => {
            state.triage_mut().fail_article(article_idx, reason.clone());
        }
        LlmResultKind::RateLimited { reason }
        | LlmResultKind::ValidationFailed { reason, .. }
        | LlmResultKind::Failed { reason } => {
            state.triage_mut().fail_article(article_idx, reason.clone());
        }
    }
}
