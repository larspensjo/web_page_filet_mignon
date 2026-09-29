use engine_logging::engine_warn;
use harvester_engine::llm::{validate_signal_candidate, validate_summary, validate_triage};

use crate::{
    ArticleSummaryResult, ArticleTriageResult, CollectedEntry, CollectedOutcome, Effect,
    SignalCandidateCacheKey, StageKind, SummaryCacheKey, TriageCacheKey,
};

pub(super) fn handle(state: &mut crate::AppState, entries: Vec<CollectedEntry>) -> Vec<Effect> {
    for entry in entries {
        let (raw_output_json, usage, resolved_model) = match entry.outcome {
            CollectedOutcome::Success {
                raw_output_json,
                usage,
                resolved_model,
            } => (raw_output_json, usage, resolved_model),
            CollectedOutcome::LineError { detail } => {
                engine_warn!(
                    "[batch-collect] batch_id={} custom_id={} cache_key={} line error: {}",
                    entry.batch_id,
                    entry.custom_id,
                    entry.key.content_hash,
                    detail
                );
                continue;
            }
        };
        if !state.frozen_batch_key_is_current(&entry.key) {
            engine_warn!(
                "[batch-collect] batch_id={} custom_id={} cache_key={} paid-but-unused: frozen prompt/context/model key differs from current configuration",
                entry.batch_id,
                entry.custom_id,
                entry.key.content_hash
            );
        }
        match entry.stage {
            StageKind::Triage => match validate_triage(&raw_output_json) {
                Ok(value) => {
                    let key = TriageCacheKey::try_new_with_context_hash(
                        &entry.key.content_hash,
                        entry.key.prompt_id,
                        Some(entry.key.prompt_version),
                        Some(&entry.key.model_id),
                        &entry.key.context_hash,
                    );
                    if let Ok(key) = key {
                        state.record_batch_llm_usage(&resolved_model, &usage);
                        state.store_frozen_triage_result(
                            key,
                            ArticleTriageResult {
                                category: value.category,
                                priority: value.priority.value(),
                                tags: value.tags,
                                rationale: value.rationale,
                                input_tokens: usage.input_tokens,
                                output_tokens: usage.output_tokens,
                            },
                            entry.created_at_utc,
                        );
                    }
                }
                Err(err) => engine_warn!(
                    "[batch-collect] batch_id={} custom_id={} cache_key={} invalid triage: {}",
                    entry.batch_id,
                    entry.custom_id,
                    entry.key.content_hash,
                    err
                ),
            },
            StageKind::Summary => match validate_summary(&raw_output_json) {
                Ok(value) => {
                    let key = SummaryCacheKey {
                        content_hash: entry.key.content_hash.clone(),
                        prompt_id: entry.key.prompt_id,
                        prompt_version: entry.key.prompt_version,
                        model_id: entry.key.model_id.clone(),
                        context_hash: entry.key.context_hash.clone(),
                    };
                    state.record_batch_llm_usage(&resolved_model, &usage);
                    state.store_summary_result(
                        key,
                        ArticleSummaryResult {
                            title: value.title,
                            summary: value.summary,
                            key_points: value.key_points,
                            input_tokens: usage.input_tokens,
                            output_tokens: usage.output_tokens,
                            entities: value.entities,
                        },
                        entry.created_at_utc,
                    );
                }
                Err(err) => engine_warn!(
                    "[batch-collect] batch_id={} custom_id={} cache_key={} invalid summary: {}",
                    entry.batch_id,
                    entry.custom_id,
                    entry.key.content_hash,
                    err
                ),
            },
            StageKind::SignalCandidate => match validate_signal_candidate(&raw_output_json) {
                Ok(mut value) => {
                    value.input_tokens = usage.input_tokens;
                    value.output_tokens = usage.output_tokens;
                    let key = SignalCandidateCacheKey {
                        signal_input_hash: entry.key.content_hash.clone(),
                        prompt_id: entry.key.prompt_id,
                        prompt_version: entry.key.prompt_version,
                        model_id: entry.key.model_id.clone(),
                        context_hash: entry.key.context_hash.clone(),
                    };
                    state.record_batch_llm_usage(&resolved_model, &usage);
                    state.store_signal_candidate_result(key, value, entry.created_at_utc);
                }
                Err(err) => engine_warn!(
                    "[batch-collect] batch_id={} custom_id={} cache_key={} invalid signal: {}",
                    entry.batch_id,
                    entry.custom_id,
                    entry.key.content_hash,
                    err
                ),
            },
        }
    }
    Vec::new()
}
