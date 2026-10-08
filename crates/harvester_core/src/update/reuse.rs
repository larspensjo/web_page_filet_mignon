//! Apply current in-memory results without allocating a model request.
use super::signal_candidate::try_enqueue;
use super::summary_cache_support::{context_hash_for_log, short_hash};
use crate::{state::TriageCacheLookupResult, AppState};
use engine_logging::engine_info;
use harvester_engine::llm::prompt::PromptId;

pub(super) enum TriageReuseOutcome {
    Hit,
    Miss,
    KeyUnavailable,
}

pub(super) fn reuse_triage(state: &mut AppState, next_idx: usize) -> TriageReuseOutcome {
    let content_hash = state.triage().articles()[next_idx].content_hash.clone();
    let content_hash_short = short_hash(&content_hash);
    match state.try_reuse_triage(&content_hash) {
        TriageCacheLookupResult::Hit {
            result: cached,
            stored_model_id,
        } => {
            let url = state.triage().articles()[next_idx].url.clone();

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

            state.mark_dirty();
            TriageReuseOutcome::Hit
        }
        TriageCacheLookupResult::Miss => TriageReuseOutcome::Miss,
        TriageCacheLookupResult::KeyUnavailable => TriageReuseOutcome::KeyUnavailable,
    }
}

pub(super) fn reuse_summary(state: &mut AppState, next_idx: usize) -> bool {
    let content_hash = state.briefing().articles()[next_idx].content_hash.clone();
    let content_hash_short = short_hash(&content_hash);
    let context_hash_value = context_hash_for_log(state.context_for(PromptId::ArticleSummary));
    let metadata = state.summary_cache_metadata();
    let version_display = metadata
        .map(|(v, _)| v.to_string())
        .unwrap_or_else(|| "<none>".into());
    let model_display = metadata
        .map(|(_, m)| m.to_string())
        .unwrap_or_else(|| "<none>".into());
    if let Ok(key) = state.current_summary_cache_key(&content_hash) {
        if state.briefing().article_cache_key(next_idx) != Some(&key) {
            state
                .briefing_mut()
                .set_article_cache_key(next_idx, Some(key.clone()));
        }
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

            state.mark_dirty();
            let article_url = state.briefing().articles()[next_idx].url.clone();
            let _ = crate::update::signal_candidate::try_enqueue(state, &article_url);
            return true;
        }
    }
    false
}

pub(super) fn reuse_score(state: &mut AppState, url: &str) -> bool {
    let Some(snapshot) = state.signal_candidate_input_snapshot(url) else {
        return false;
    };
    let key = super::signal_candidate::input_key(url, snapshot);
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
        state.signal_candidate_mut().complete(url, cached);
        state.clear_signal_candidate_input_snapshot(url);
        state.mark_dirty();
        return true;
    }
    false
}
