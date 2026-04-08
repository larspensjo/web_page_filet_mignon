use engine_logging::{engine_info, engine_warn};

use crate::{context_hash, AppState, SummaryCacheKey, SummaryCacheKeyError};
use harvester_engine::llm::prompt::{PromptId, PromptVersion};

pub(super) fn log_summary_cache_warmup_if_needed(state: &mut AppState) {
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

pub(super) fn log_summary_cache_run_summary(state: &mut AppState) {
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

pub(super) fn summary_cache_key_error_reason(error: &SummaryCacheKeyError) -> &'static str {
    match error {
        SummaryCacheKeyError::MissingPromptVersion => "missing prompt_version metadata",
        SummaryCacheKeyError::MissingModelId => "missing model_id metadata",
        SummaryCacheKeyError::EmptyContentHash => "empty content_hash",
    }
}

pub(super) fn build_summary_cache_key(
    content_hash: &str,
    prompt_id: PromptId,
    prompt_version: Option<PromptVersion>,
    model_id: Option<&str>,
    context: &[(String, String)],
) -> Result<SummaryCacheKey, SummaryCacheKeyError> {
    SummaryCacheKey::try_new(content_hash, prompt_id, prompt_version, model_id, context)
}

pub(super) fn short_hash(hash: &str) -> &str {
    let end = hash.len().min(8);
    &hash[..end]
}

pub(super) fn log_summary_cache_lookup_mismatch(
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

pub(super) fn log_summary_cache_completion_metadata(
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

pub(super) fn summary_cache_model_ids_compatible(
    store_model_id: &str,
    completion_model_id: &str,
) -> bool {
    crate::cache_utils::model_ids_compatible(store_model_id, completion_model_id)
}

pub(super) fn context_hash_for_log(context: &[(String, String)]) -> String {
    context_hash(context)
}
