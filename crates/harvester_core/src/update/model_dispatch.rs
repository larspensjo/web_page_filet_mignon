//! Shared article-model admission scheduler. Cache completions yield back to
//! priority selection before the next request is issued.
use super::signal_candidate::{render_extra_template_vars, render_input_content, try_enqueue};
use super::summary_cache_support::{
    context_hash_for_log, log_summary_cache_warmup_if_needed, short_hash,
    summary_cache_key_error_reason,
};
use crate::briefing::BriefingPhase;
use crate::state::TriageCacheLookupResult;
use crate::{AppState, Effect};
use engine_logging::{engine_info, engine_warn};
use harvester_engine::llm::prompt::PromptId;

const STAGE_PRIORITY: &[PromptId] = &[
    PromptId::ArticleSignalCandidate,
    PromptId::ArticleSummary,
    PromptId::ArticleTriage,
];

fn has_slot(state: &AppState) -> bool {
    state.article_model_requests_in_flight() < state.llm_max_in_flight()
}

pub(super) fn dispatch_model_work(state: &mut AppState, effects: &mut Vec<Effect>) {
    if let Some(reason) = state
        .result_store_failure()
        .or_else(|| state.model_dispatch_halt_reason())
        .map(str::to_owned)
    {
        state.triage_mut().fail_all_pending(&reason);
        state.briefing_mut().fail_all_pending(&reason);
        for url in state.signal_candidate_mut().fail_all_pending(&reason) {
            state.clear_signal_candidate_input_snapshot(&url);
        }
    } else {
        if state.pipeline_ready() {
            loop {
                super::waves::release_ready(state);
                let progressed = STAGE_PRIORITY.iter().any(|stage| match stage {
                    PromptId::ArticleSignalCandidate => dispatch_scoring(state, effects),
                    PromptId::ArticleSummary => dispatch_summary(state, effects),
                    PromptId::ArticleTriage => dispatch_triage(state, effects),
                });
                if !progressed {
                    break;
                }
            }
        }
    }
    if matches!(state.triage().phase(), crate::TriagePhase::Triaging) {
        super::triage::settle_triage(state, effects);
    }
    if matches!(state.briefing().phase(), BriefingPhase::Summarizing)
        && state.is_briefing_metadata_ready()
    {
        super::briefing::settle_summaries(state, effects);
    }
}

fn dispatch_scoring(state: &mut AppState, effects: &mut Vec<Effect>) -> bool {
    let Some(url) = super::waves::next_score(state) else {
        return false;
    };
    let Some(snapshot) = state.signal_candidate_input_snapshot(&url).cloned() else {
        state
            .signal_candidate_mut()
            .fail(&url, "missing scoring input snapshot");
        return true;
    };
    let key = super::signal_candidate::input_key(&url, &snapshot);
    if let Some(mut cached) = key
        .as_ref()
        .and_then(|key| state.try_reuse_signal_candidate(key))
    {
        engine_info!(
            "[signal-cache] url={} decision=hit signal_score={} signal_key={} key_digest={}",
            url,
            cached.signal_score,
            cached.signal_key,
            key.as_ref().expect("cache hit requires key").digest()
        );
        cached.input_tokens = 0;
        cached.output_tokens = 0;
        state.signal_candidate_mut().complete(&url, cached);
        state.clear_signal_candidate_input_snapshot(&url);
        state.mark_dirty();
        return true;
    }
    if !has_slot(state) {
        return false;
    }
    let request_id = state.allocate_next_llm_request_id();
    state.record_pending_llm_request(request_id, PromptId::ArticleSignalCandidate);
    state.signal_candidate_mut().mark_scoring(&url, request_id);
    effects.push(Effect::RequestLlmCompletion {
        request_id,
        prompt_id: PromptId::ArticleSignalCandidate,
        prompt_version: Some(snapshot.prompt_version),
        input_content: render_input_content(&url, &snapshot),
        context: snapshot.context.clone(),
        extra_template_vars: render_extra_template_vars(&url, &snapshot),
    });
    engine_info!(
        "[signal-dispatch] url={} request_id={} decision=dispatched",
        url,
        request_id
    );
    state.mark_dirty();
    true
}

fn dispatch_triage(state: &mut AppState, effects: &mut Vec<Effect>) -> bool {
    if !matches!(state.triage().phase(), crate::TriagePhase::Triaging) {
        return false;
    }
    super::triage::log_triage_cache_run_start_if_needed(state);
    let Some(next_idx) = super::waves::next_article(state, crate::PipelineStage::Triaging) else {
        return false;
    };
    let content_hash = state.triage().articles()[next_idx].content_hash.clone();
    let content_hash_short = short_hash(&content_hash);
    let current_key = state.current_triage_cache_key(&content_hash);
    state
        .triage_mut()
        .set_article_cache_key(next_idx, current_key);

    match state.try_reuse_triage(&content_hash) {
        TriageCacheLookupResult::Hit {
            result: cached,
            stored_model_id,
        } => {
            let themes = cached.tags.clone();
            let url = state.triage().articles()[next_idx].url.clone();
            let fetched_utc = state.triage().articles()[next_idx].fetched_utc.clone();
            let triage_priority = cached.priority;
            let result = cached.clone();
            let stored_model_id = stored_model_id.to_string();
            let signal_state_present_before_enqueue =
                state.signal_candidate().state_for(&url).is_some();
            state.record_triage_cache_hit();
            engine_info!("[triage-cache] hit content_hash={}", content_hash_short);
            state
                .triage_mut()
                .complete_article_with_model(next_idx, result, Some(stored_model_id));
            engine_info!(
                    "[signal-dispatch] triage cache-hit url={} triage_priority={} signal_state_present_before_enqueue={}",
                    url,
                    triage_priority,
                    signal_state_present_before_enqueue
                );
            super::waves::triage_changed(state, &url, &content_hash);
            let enqueued = try_enqueue(state, &url);
            engine_info!(
                "[signal-dispatch] triage cache-hit enqueue url={} enqueued={}",
                url,
                enqueued
            );
            state.refresh_selected_preview();
            state.mark_dirty();
            effects.push(Effect::UpsertEntityIndexEntry {
                url,
                fetched_utc,
                content_hash: Some(content_hash.clone()),
                summary_entities: None,
                themes: Some(themes),
            });
            return true;
        }
        TriageCacheLookupResult::Miss => {
            if !has_slot(state) {
                return false;
            }
            state.record_triage_cache_miss();
            engine_info!("[triage-cache] miss content_hash={}", content_hash_short);
        }
        TriageCacheLookupResult::KeyUnavailable => {
            if !has_slot(state) {
                return false;
            }
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

    if !has_slot(state) {
        return false;
    }
    let prepared_text = state.triage().articles()[next_idx].prepared_text.clone();
    let request_id = state.allocate_next_llm_request_id();
    state.record_pending_llm_request(request_id, PromptId::ArticleTriage);
    state.triage_mut().start_article(next_idx, request_id);

    let context = state.context_for(PromptId::ArticleTriage).to_vec();

    engine_info!(
        "[llm-concurrency] triage dispatch request_id={} article={} outstanding={} llm_max_in_flight={}",
        request_id,
        next_idx,
        state.article_model_requests_in_flight(),
        state.llm_max_in_flight()
    );

    effects.push(Effect::RequestLlmCompletion {
        request_id,
        prompt_id: PromptId::ArticleTriage,
        prompt_version: None,
        input_content: prepared_text,
        context,
        extra_template_vars: vec![],
    });
    state.mark_dirty();
    true
}

fn dispatch_summary(state: &mut AppState, effects: &mut Vec<Effect>) -> bool {
    if !matches!(state.briefing().phase(), BriefingPhase::Summarizing)
        || !state.is_briefing_metadata_ready()
    {
        return false;
    }
    log_summary_cache_warmup_if_needed(state);
    let Some(next_idx) = super::waves::next_article(state, crate::PipelineStage::Summarizing)
    else {
        return false;
    };
    let article = &state.briefing().articles()[next_idx];
    let prepared_text = article.prepared_text.clone();
    let content_hash = article.content_hash.clone();
    let content_hash_short = short_hash(&content_hash);
    let context = state.context_for(PromptId::ArticleSummary).to_vec();
    let context_hash_value = context_hash_for_log(&context);
    let metadata = state.summary_cache_metadata();
    let version_display = metadata
        .map(|(version, _)| version.to_string())
        .unwrap_or_else(|| "<none>".to_string());
    let model_display = metadata
        .map(|(_, model)| model.to_string())
        .unwrap_or_else(|| "<none>".to_string());

    match state.current_summary_cache_key(&content_hash) {
        Ok(key) => {
            state
                .briefing_mut()
                .set_article_cache_key(next_idx, Some(key.clone()));
            if let Some(cached_result) = state.try_reuse_summary(&key) {
                let article_entities = cached_result.entities.clone();
                let url = state.briefing().articles()[next_idx].url.clone();
                let fetched_utc = state.briefing().articles()[next_idx].fetched_utc.clone();
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
                state.refresh_selected_preview();
                state.mark_dirty();
                effects.push(Effect::UpsertEntityIndexEntry {
                    url,
                    fetched_utc,
                    content_hash: Some(content_hash.clone()),
                    summary_entities: Some(article_entities),
                    themes: None,
                });
                let article_url = state.briefing().articles()[next_idx].url.clone();
                let _ = crate::update::signal_candidate::try_enqueue(state, &article_url);
                // Cache hit: slot not consumed, continue filling.
                return true;
            }

            if !has_slot(state) {
                return false;
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
                "[llm-concurrency] summary dispatch request_id={} article={} outstanding={} llm_max_in_flight={}",
                request_id,
                next_idx,
                state.article_model_requests_in_flight(),
                state.llm_max_in_flight()
            );
            effects.push(Effect::RequestLlmCompletion {
                request_id,
                prompt_id: PromptId::ArticleSummary,
                prompt_version: None,
                input_content: prepared_text,
                context,
                extra_template_vars: vec![],
            });
            state.mark_dirty();
            true
        }
        Err(err) => {
            if !has_slot(state) {
                return false;
            }
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
                    "[llm-concurrency] summary dispatch (no-cache-key) request_id={} article={} outstanding={} llm_max_in_flight={}",
                    request_id,
                    next_idx,
                    state.article_model_requests_in_flight(),
                    state.llm_max_in_flight()
                );
            effects.push(Effect::RequestLlmCompletion {
                request_id,
                prompt_id: PromptId::ArticleSummary,
                prompt_version: None,
                input_content: prepared_text,
                context,
                extra_template_vars: vec![],
            });
            state.mark_dirty();
            true
        }
    }
}

#[cfg(test)]
#[path = "model_dispatch_tests.rs"]
mod tests;
