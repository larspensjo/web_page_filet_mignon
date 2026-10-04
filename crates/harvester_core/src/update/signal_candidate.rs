//! Signal-candidate scoring reducer.
//!
//! Owns enqueue logic, completion handling, duplicate-enqueue prevention, and
//! persistence-effect emission for the signal-candidate cache and overrides.

use engine_logging::{engine_debug, engine_info, engine_warn};
use harvester_engine::llm::dto::SignalCandidateResult;
use harvester_engine::llm::prompt::{PromptId, PromptVersion};
use harvester_engine::llm::validation::validate_signal_candidate;

use crate::msg::LlmResultKind;
use crate::signal_candidate::OverrideKey;
use crate::signal_candidate_cache::{
    SignalCandidateCacheKey, SignalCandidateCacheKeyError, SignalCandidateInputBundle,
};
use crate::{AppState, Effect};

pub(crate) const PRIORITY_CUTOFF_INCLUSIVE: u8 = 2;

/// Inputs required to compute the signal-candidate cache key for a URL.
///
/// The snapshot is built at enqueue time and is the single source of truth for
/// the cache key — including the prompt version and canonical model id — so
/// completion-time persistence cannot drift from lookup-time keys (e.g. when
/// the provider returns a dated model variant like `gpt-5.4-mini-2026-03-17`
/// while the configured canonical model is `gpt-5.4-mini`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SignalCandidateInputSnapshot {
    pub outlet: String,
    pub title: String,
    pub published_at: String,
    pub triage_priority: u8,
    pub triage_tags_sorted: Vec<String>,
    pub summary: String,
    pub key_points: Vec<String>,
    pub upstream_summary_cache_digest: String,
    pub context: Vec<(String, String)>,
    pub prompt_version: PromptVersion,
    pub model_id: String,
}

pub(crate) fn handle_signal_candidate_completion(
    state: &mut AppState,
    request_id: u64,
    result: &LlmResultKind,
) {
    let url = match state.signal_candidate().url_for_request(request_id) {
        Some(url) => url.to_string(),
        None => {
            engine_warn!(
                "[signal-dispatch] completion for unknown request_id={}",
                request_id
            );
            return;
        }
    };

    match result {
        LlmResultKind::Success {
            output_json,
            input_tokens,
            output_tokens,
            prompt_version,
            resolved_model,
        } => match validate_signal_candidate(output_json) {
            Ok(mut parsed) => {
                parsed.input_tokens = *input_tokens;
                parsed.output_tokens = *output_tokens;

                if let Some(snapshot) = state.signal_candidate_input_snapshot(&url).cloned() {
                    log_completion_metadata_drift(&url, &snapshot, *prompt_version, resolved_model);
                    persist_signal_candidate_result(state, &url, &snapshot, parsed.clone());
                } else {
                    engine_warn!(
                        "[signal-cache] url={} no input snapshot present; skipping cache write",
                        url
                    );
                }

                state.signal_candidate_mut().complete(&url, parsed);
                state.clear_signal_candidate_input_snapshot(&url);
            }
            Err(err) => {
                engine_warn!(
                    "[signal-dispatch] validation failed url={} reason={}",
                    url,
                    err
                );
                state
                    .signal_candidate_mut()
                    .fail(&url, format!("validation: {err}"));
                state.clear_signal_candidate_input_snapshot(&url);
            }
        },
        LlmResultKind::ValidationFailed { reason, .. } => {
            engine_warn!(
                "[signal-dispatch] validation failed url={} reason={}",
                url,
                reason
            );
            state
                .signal_candidate_mut()
                .fail(&url, format!("validation: {reason}"));
            state.clear_signal_candidate_input_snapshot(&url);
        }
        LlmResultKind::QuotaExhausted { reason, .. } => {
            engine_warn!(
                "[signal-dispatch] quota exhausted url={} reason={}",
                url,
                reason
            );
            state
                .signal_candidate_mut()
                .fail(&url, format!("quota exhausted: {reason}"));
            state.clear_signal_candidate_input_snapshot(&url);
        }
        LlmResultKind::RateLimited { reason } => {
            engine_warn!(
                "[signal-dispatch] rate limited url={} reason={}",
                url,
                reason
            );
            state.signal_candidate_mut().fail(&url, reason.clone());
            state.clear_signal_candidate_input_snapshot(&url);
        }
        LlmResultKind::Failed { reason } => {
            engine_warn!("[signal-dispatch] llm failed url={} reason={}", url, reason);
            state.signal_candidate_mut().fail(&url, reason.clone());
            state.clear_signal_candidate_input_snapshot(&url);
        }
    }

    state.mark_dirty();
}

pub fn try_enqueue(state: &mut AppState, url: &str) -> bool {
    if !state.pipeline_ready() {
        return false;
    }
    let snapshot = match build_input_snapshot(state, url) {
        Ok(snapshot) => snapshot,
        Err(reason) => {
            engine_debug!(
                "[signal-dispatch] enqueue rejected url={} reason={}",
                url,
                reason
            );
            return false;
        }
    };
    if snapshot.triage_priority < PRIORITY_CUTOFF_INCLUSIVE {
        engine_debug!(
            "[signal-dispatch] enqueue rejected url={} reason=below-priority-cutoff",
            url
        );
        return false;
    }
    let Some(key) = input_key(url, &snapshot) else {
        engine_debug!(
            "[signal-dispatch] enqueue rejected url={} reason=cache-key-unavailable",
            url
        );
        return false;
    };
    let Some(hash) = state.content_hash_for_url(url).map(str::to_owned) else {
        return false;
    };
    let member = (url.to_owned(), hash);
    if state
        .pipeline_admission
        .as_ref()
        .is_some_and(|run| run.admitted[2].contains(&member))
    {
        return false;
    }
    let digest = key.digest();
    if !state
        .signal_candidate_mut()
        .enqueue(url.to_string(), digest.clone())
    {
        return false;
    }
    assert!(super::waves::scoring_admitted(
        state,
        member.clone(),
        digest
    ));
    state.set_signal_candidate_input_snapshot(url, snapshot);
    if super::reuse::reuse_score(state, url) {
        super::waves::record_reused(state, crate::PipelineStage::ScoringSignals, &member);
    }
    state.mark_dirty();
    true
}

pub(crate) fn input_key(
    url: &str,
    snapshot: &SignalCandidateInputSnapshot,
) -> Option<SignalCandidateCacheKey> {
    try_input_key(url, snapshot)
        .map_err(|err| {
            engine_warn!(
                "[signal-cache] url={} cache key unavailable reason={}",
                url,
                err
            );
        })
        .ok()
}

fn try_input_key(
    url: &str,
    snapshot: &SignalCandidateInputSnapshot,
) -> Result<SignalCandidateCacheKey, SignalCandidateCacheKeyError> {
    SignalCandidateCacheKey::try_new(
        &build_input_bundle(url, snapshot),
        Some(snapshot.prompt_version),
        Some(&snapshot.model_id),
        &snapshot.context,
    )
}

pub fn handle_cache_loaded(
    state: &mut AppState,
    cache: crate::signal_candidate_cache::SignalCandidateCache,
) {
    state.set_signal_candidate_cache(cache);
    state.mark_dirty();
}

pub fn handle_overrides_loaded(
    state: &mut AppState,
    overrides: std::collections::HashSet<OverrideKey>,
) {
    state.signal_candidate_mut().set_excluded(overrides);
    state.mark_dirty();
}

pub fn handle_toggle_exclusion(
    state: &mut AppState,
    signal_key: String,
    effects: &mut Vec<Effect>,
) {
    let prompt_version = state
        .active_version_for(PromptId::ArticleSignalCandidate)
        .unwrap_or_default();
    let key = OverrideKey {
        signal_key,
        prompt_id: PromptId::ArticleSignalCandidate.to_string(),
        prompt_version,
    };

    if state.signal_candidate().excluded().contains(&key) {
        state.signal_candidate_mut().remove_exclusion(&key);
    } else {
        state.signal_candidate_mut().add_exclusion(key);
    }

    effects.push(Effect::PersistSignalCandidateOverrides {
        overrides: state.signal_candidate().excluded().clone(),
    });
    state.mark_dirty();
}

pub(crate) fn build_input_snapshot(
    state: &AppState,
    url: &str,
) -> Result<SignalCandidateInputSnapshot, &'static str> {
    let article = state
        .triage()
        .articles()
        .iter()
        .find(|article| article.url == url)
        .ok_or("triage-article-unavailable")?;
    let triage_key = state
        .current_triage_cache_key(&article.content_hash)
        .ok_or("current-triage-key-unavailable")?;
    let (_, triage) = state
        .triage_cache()
        .lookup(&triage_key)
        .ok_or("current-triage-result-unavailable")?;
    let summary_key = state
        .current_summary_cache_key(&article.content_hash)
        .map_err(|_| "current-summary-key-unavailable")?;
    let summary = state
        .briefing()
        .articles()
        .iter()
        .enumerate()
        .find_map(|(idx, candidate)| {
            if candidate.url != url || state.briefing().article_cache_key(idx) != Some(&summary_key)
            {
                return None;
            }
            match &candidate.summary_state {
                crate::briefing::ArticleSummaryState::Completed { result } => Some(result),
                _ => None,
            }
        })
        .or_else(|| state.try_reuse_summary(&summary_key))
        .ok_or("current-summary-result-unavailable")?;
    build_input_snapshot_from_current_result_fields(
        state,
        &article.url,
        article.source_title.as_deref(),
        article.fetched_utc.as_deref(),
        triage,
        &summary_key,
        summary,
    )
}

/// Build the signal-scoring key from already-resolved current-key upstream
/// results. Dispatch and completeness use this same path so scoring identity
/// cannot drift between queueing and unfinished-work classification.
pub(crate) struct SignalArticleFields<'a> {
    pub url: &'a str,
    pub source_title: Option<&'a str>,
    pub fetched_utc: Option<&'a str>,
}

#[cfg(test)]
pub(crate) fn input_key_for_current_results(
    state: &AppState,
    article: &crate::briefing::LoadedArticle,
    triage: &crate::triage::ArticleTriageResult,
    summary_key: &crate::summary_cache::SummaryCacheKey,
    summary: &crate::briefing::ArticleSummaryResult,
) -> Option<SignalCandidateCacheKey> {
    input_key_for_current_result_fields(
        state,
        &article.url,
        article.source_title.as_deref(),
        article.fetched_utc.as_deref(),
        triage,
        summary_key,
        summary,
    )
}

#[cfg(test)]
pub(crate) fn input_key_for_current_result_fields(
    state: &AppState,
    url: &str,
    source_title: Option<&str>,
    fetched_utc: Option<&str>,
    triage: &crate::triage::ArticleTriageResult,
    summary_key: &crate::summary_cache::SummaryCacheKey,
    summary: &crate::briefing::ArticleSummaryResult,
) -> Option<SignalCandidateCacheKey> {
    let context_hash = crate::context_hash(state.context_for(PromptId::ArticleSignalCandidate));
    input_key_for_current_result_fields_with_context_hash(
        state,
        SignalArticleFields {
            url,
            source_title,
            fetched_utc,
        },
        triage,
        summary_key,
        summary,
        &context_hash,
    )
}

pub(crate) fn input_key_for_current_result_fields_with_context_hash(
    state: &AppState,
    article: SignalArticleFields<'_>,
    triage: &crate::triage::ArticleTriageResult,
    summary_key: &crate::summary_cache::SummaryCacheKey,
    summary: &crate::briefing::ArticleSummaryResult,
    context_hash: &str,
) -> Option<SignalCandidateCacheKey> {
    let snapshot = build_input_snapshot_from_current_result_fields(
        state,
        article.url,
        article.source_title,
        article.fetched_utc,
        triage,
        summary_key,
        summary,
    )
    .ok()?;
    SignalCandidateCacheKey::try_new_with_context_hash(
        &build_input_bundle(article.url, &snapshot),
        Some(snapshot.prompt_version),
        Some(&snapshot.model_id),
        context_hash,
    )
    .ok()
}

fn build_input_snapshot_from_current_result_fields(
    state: &AppState,
    url: &str,
    source_title: Option<&str>,
    fetched_utc: Option<&str>,
    triage: &crate::triage::ArticleTriageResult,
    summary_key: &crate::summary_cache::SummaryCacheKey,
    summary: &crate::briefing::ArticleSummaryResult,
) -> Result<SignalCandidateInputSnapshot, &'static str> {
    let prompt_version = state
        .active_version_for(PromptId::ArticleSignalCandidate)
        .ok_or("scoring-prompt-version-unavailable")?;
    let model_id = state
        .effective_model_for(PromptId::ArticleSignalCandidate)
        .ok_or("scoring-model-unavailable")?
        .to_string();
    let published_at = fetched_utc.unwrap_or_default().to_string();
    let title = crate::preview::best_effort_article_title(source_title, url).unwrap_or_else(|| {
        source_title
            .map(str::to_string)
            .unwrap_or_else(|| url.to_string())
    });
    let outlet = best_effort_outlet(source_title, url);
    let upstream_summary_cache_digest = summary_key.digest();
    let mut triage_tags_sorted = triage.tags.clone();
    triage_tags_sorted.sort();

    Ok(SignalCandidateInputSnapshot {
        outlet,
        title,
        published_at,
        triage_priority: triage.priority,
        triage_tags_sorted,
        summary: summary.summary.clone(),
        key_points: summary.key_points.clone(),
        upstream_summary_cache_digest,
        context: state.context_for(PromptId::ArticleSignalCandidate).to_vec(),
        prompt_version,
        model_id,
    })
}

fn build_input_bundle<'a>(
    url: &'a str,
    snapshot: &'a SignalCandidateInputSnapshot,
) -> SignalCandidateInputBundle<'a> {
    SignalCandidateInputBundle {
        url,
        outlet: &snapshot.outlet,
        title: &snapshot.title,
        published_at: &snapshot.published_at,
        triage_priority: snapshot.triage_priority,
        triage_tags_sorted: snapshot
            .triage_tags_sorted
            .iter()
            .map(String::as_str)
            .collect(),
        summary: &snapshot.summary,
        key_points: &snapshot.key_points,
        upstream_summary_cache_digest: snapshot.upstream_summary_cache_digest.clone(),
    }
}

pub(super) fn render_input_content(url: &str, snapshot: &SignalCandidateInputSnapshot) -> String {
    format!("signal-candidate scoring for {url} [{}]", snapshot.title)
}

pub(super) fn render_extra_template_vars(
    url: &str,
    snapshot: &SignalCandidateInputSnapshot,
) -> Vec<(String, String)> {
    let key_points = if snapshot.key_points.is_empty() {
        String::new()
    } else {
        snapshot
            .key_points
            .iter()
            .map(|point| format!("- {point}"))
            .collect::<Vec<_>>()
            .join("\n")
    };

    vec![
        ("url".to_string(), url.to_string()),
        ("outlet".to_string(), snapshot.outlet.clone()),
        ("title".to_string(), snapshot.title.clone()),
        ("published_at".to_string(), snapshot.published_at.clone()),
        (
            "triage_priority".to_string(),
            snapshot.triage_priority.to_string(),
        ),
        (
            "triage_tags".to_string(),
            snapshot.triage_tags_sorted.join(", "),
        ),
        ("summary".to_string(), snapshot.summary.clone()),
        ("key_points".to_string(), key_points),
    ]
}

fn persist_signal_candidate_result(
    state: &mut AppState,
    url: &str,
    snapshot: &SignalCandidateInputSnapshot,
    result: SignalCandidateResult,
) {
    let bundle = build_input_bundle(url, snapshot);
    match SignalCandidateCacheKey::try_new(
        &bundle,
        Some(snapshot.prompt_version),
        Some(snapshot.model_id.as_str()),
        &snapshot.context,
    ) {
        Ok(key) => {
            let now = chrono::Utc::now().to_rfc3339();
            state.store_signal_candidate_result(key.clone(), result, now);
            engine_info!(
                "[signal-cache] url={} decision=store prompt_version={} model_id={} key_digest={}",
                url,
                snapshot.prompt_version,
                snapshot.model_id,
                key.digest()
            );
        }
        Err(err) => {
            engine_warn!(
                "[signal-cache] url={} cache key unavailable on completion reason={:?}",
                url,
                err
            );
        }
    }
}

/// Logs when the provider-returned metadata differs from the run-frozen snapshot
/// metadata that owns the cache key. Mirrors the summary-cache diagnostic so
/// dated model variants (e.g. `gpt-5.4-mini-2026-03-17` vs canonical
/// `gpt-5.4-mini`) are visible without breaking cache reuse.
fn log_completion_metadata_drift(
    url: &str,
    snapshot: &SignalCandidateInputSnapshot,
    completion_prompt_version: PromptVersion,
    completion_model_id: &str,
) {
    if snapshot.model_id != completion_model_id {
        if crate::cache_utils::model_ids_compatible(&snapshot.model_id, completion_model_id) {
            engine_info!(
                "[signal-cache] completion metadata differs by model variant url={} cache_model={} completion_model={}",
                url,
                snapshot.model_id,
                completion_model_id,
            );
        } else {
            engine_warn!(
                "[signal-cache] completion model_id mismatch url={} cache_model={} completion_model={}",
                url,
                snapshot.model_id,
                completion_model_id,
            );
        }
    }
    if snapshot.prompt_version != completion_prompt_version {
        engine_warn!(
            "[signal-cache] completion prompt_version mismatch url={} cache_version={} completion_version={}",
            url,
            snapshot.prompt_version,
            completion_prompt_version,
        );
    }
}

fn best_effort_outlet(source_title: Option<&str>, url: &str) -> String {
    if let Some(source_title) = source_title
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        return source_title.to_string();
    }

    match url::Url::parse(url) {
        Ok(parsed) => parsed.host_str().unwrap_or_default().to_string(),
        Err(_) => String::new(),
    }
}
