//! Shared in-memory response for processing configuration in reducer and host fixtures.

use crate::{AppState, Effect, Msg};
use harvester_engine::llm::PromptId;

#[doc(hidden)]
pub fn complete_processing_configuration(
    state: AppState,
    effects: Vec<Effect>,
    preparation_budget: usize,
) -> (AppState, Vec<Effect>) {
    let Some(request_id) = effects.iter().find_map(|effect| match effect {
        Effect::LoadProcessingConfiguration { request_id, .. } => Some(*request_id),
        _ => None,
    }) else {
        return (state, effects);
    };
    let ids = [
        PromptId::ArticleTriage,
        PromptId::ArticleSummary,
        PromptId::ArticleSignalCandidate,
    ];
    let contexts = ids
        .iter()
        .map(|&id| (id, state.context_for(id).to_vec()))
        .collect();
    let active_versions = ids
        .iter()
        .filter_map(|&id| Some((id, state.active_version_for(id)?)))
        .collect();
    let effective_models = ids
        .iter()
        .filter_map(|&id| Some((id, state.effective_model_for(id)?.to_owned())))
        .collect();
    crate::update(
        state,
        Msg::ProcessingConfigurationLoaded {
            request_id,
            contexts,
            active_versions,
            effective_models,
            preparation_budget,
        },
    )
}

/// Completes the configuration and fresh membership read using the fixture's held corpus.
#[doc(hidden)]
pub fn complete_processing_start(
    state: AppState,
    effects: Vec<Effect>,
    preparation_budget: usize,
) -> (AppState, Vec<Effect>) {
    let articles = state.pre_triage().resolved_included_articles();
    let configuring = effects
        .iter()
        .any(|e| matches!(e, Effect::LoadProcessingConfiguration { .. }));
    let (state, effects) = complete_processing_configuration(state, effects, preparation_budget);
    if configuring {
        if let Some(request_id) = effects.iter().find_map(|e| match e {
            Effect::LoadArticlesForTriage { request_id, .. } => Some(*request_id),
            _ => None,
        }) {
            return crate::update(
                state,
                Msg::TriageArticlesLoaded {
                    request_id,
                    delta: harvester_engine::TriageArticleDelta::full_window(
                        articles,
                        preparation_budget,
                    ),
                },
            );
        }
    }
    (state, effects)
}

#[doc(hidden)]
pub use crate::pre_triage_filter::test_support::ManualPreTriageDecisions;

/// Model hydration without discarding results a fixture has already produced.
#[doc(hidden)]
pub fn hydrate_fixture_result_stores(mut state: AppState) -> AppState {
    let replies = [
        Msg::TriageCacheHydrated {
            cache: state.triage_cache().clone(),
        },
        Msg::SummaryCacheHydrated {
            cache: state.summary_cache().clone(),
        },
        Msg::SignalCandidateCacheLoaded {
            cache: state.signal_candidate_cache().clone(),
        },
    ];
    for reply in replies {
        state = crate::update(state, reply).0;
    }
    state
}

/// Deliver missing startup replies without discarding a fixture's saved results or keys.
#[cfg(test)]
pub fn complete_archive_meter_startup(mut state: AppState) -> AppState {
    let ids = [
        PromptId::ArticleTriage,
        PromptId::ArticleSummary,
        PromptId::ArticleSignalCandidate,
    ];
    let active_versions = ids
        .iter()
        .map(|&id| (id, state.active_version_for(id).unwrap_or(1)))
        .collect();
    let effective_models = ids
        .iter()
        .map(|&id| {
            (
                id,
                state
                    .effective_model_for(id)
                    .unwrap_or("fixture-model")
                    .to_owned(),
            )
        })
        .collect();
    let contexts = ids
        .iter()
        .map(|&id| (id, state.context_for(id).to_vec()))
        .collect();
    let replies = [
        Msg::LlmMetadataLoaded {
            active_versions,
            effective_models,
        },
        Msg::PromptContextsLoaded { contexts },
        Msg::BriefingCheckpointLoaded {
            since_utc: state.briefing_since_utc().map(|since| since.to_rfc3339()),
        },
        Msg::TriageCacheHydrated {
            cache: state.triage_cache().clone(),
        },
        Msg::SummaryCacheHydrated {
            cache: state.summary_cache().clone(),
        },
        Msg::SignalCandidateCacheLoaded {
            cache: state.signal_candidate_cache().clone(),
        },
    ];
    for reply in replies {
        state = crate::update(state, reply).0;
    }
    if state.startup_inputs().initial_article_window == crate::InitialArticleWindowOutcome::Pending
    {
        // Hand-built sessions can omit the prepared body; preserve their admitted population.
        let articles = state
            .triage()
            .articles()
            .iter()
            .map(|a| crate::LoadedArticle {
                url: a.url.clone(),
                content_hash: a.content_hash.clone(),
                source_title: a.source_title.clone(),
                fetched_utc: a.fetched_utc.clone(),
                prepared_text: "fixture article content ".repeat(220),
            })
            .collect::<Vec<_>>();
        state = crate::update(
            state,
            Msg::EvaluatePreTriageRefresh {
                ordered_urls: articles.iter().map(|a| a.url.clone()).collect(),
                triggered_by_job_done: false,
            },
        )
        .0;
        let mut request_id = None;
        for tick in 0..200 {
            let (next, effects) = crate::update(
                state,
                Msg::tick_at(chrono::DateTime::from_timestamp(1_780_000_000 + tick, 0).unwrap()),
            );
            state = next;
            request_id = effects.iter().find_map(|effect| match effect {
                Effect::LoadArticlesForTriage { request_id, .. } => Some(*request_id),
                _ => None,
            });
            if request_id.is_some() {
                break;
            }
        }
        let request_id = request_id.expect("fixture startup article request");
        let delta = harvester_engine::TriageArticleDelta::full_window(articles, 100_000);
        state = crate::update(
            state,
            Msg::SavedArticlesLoaded {
                request_id,
                articles: delta.members.clone(),
            },
        )
        .0;
        state = crate::update(state, Msg::TriageArticlesLoaded { request_id, delta }).0;
    }
    assert_eq!(
        state.startup_readiness(),
        crate::StartupReadinessStatus::Ready
    );
    state
}

/// Seed current-key scores across the production-scale index for keyless view-cost checks.
#[cfg(feature = "host-drain-fixture")]
#[doc(hidden)]
pub fn save_host_drain_scores(state: AppState, scored_articles: usize) -> AppState {
    use harvester_engine::llm::dto::{Confidence, SignalCandidateResult, SourceTier};
    let mut cache = crate::SignalCandidateCache::default();
    for key in state
        .saved_results_entries()
        .filter_map(|entry| entry.signal_key.as_ref())
        .take(scored_articles)
    {
        cache.insert(
            key.clone(),
            crate::SignalCandidateCacheEntry {
                result: SignalCandidateResult {
                    signal_key: key.signal_input_hash.clone(),
                    signal_score: 90,
                    themes: vec!["topic".into()],
                    draft_gist: "Representative scored article".into(),
                    source_tier: SourceTier::Tier1,
                    confidence: Confidence::High,
                    reasoning: "cost fixture".into(),
                    input_tokens: 100,
                    output_tokens: 20,
                },
                created_at_utc: "2026-09-06T12:00:00Z".into(),
            },
        );
    }
    crate::update(state, Msg::SignalCandidateCacheLoaded { cache }).0
}

/// Persist explicitly hand-built session fixtures as current-key results, as a
/// real completion does. This adapter is for fixtures, never executable hosts.
#[doc(hidden)]
#[cfg(test)]
pub fn save_session_results(state: &mut AppState) {
    state.prepare_saved_session_fixture();
    let mut versions = [
        PromptId::ArticleTriage,
        PromptId::ArticleSummary,
        PromptId::ArticleSignalCandidate,
    ]
    .into_iter()
    .filter_map(|id| state.active_version_for(id).map(|version| (id, version)))
    .collect::<std::collections::HashMap<_, _>>();
    let mut models = [
        PromptId::ArticleTriage,
        PromptId::ArticleSummary,
        PromptId::ArticleSignalCandidate,
    ]
    .into_iter()
    .filter_map(|id| {
        state
            .effective_model_for(id)
            .map(|model| (id, model.to_owned()))
    })
    .collect::<std::collections::HashMap<_, _>>();
    for id in [
        PromptId::ArticleTriage,
        PromptId::ArticleSummary,
        PromptId::ArticleSignalCandidate,
    ] {
        versions.entry(id).or_insert(1);
        models.entry(id).or_insert_with(|| match id {
            PromptId::ArticleSummary => state
                .summary_cache()
                .iter()
                .next()
                .map(|(key, _)| key.model_id.clone())
                .unwrap_or_else(|| "fixture-summary".into()),
            PromptId::ArticleTriage => state
                .triage()
                .articles()
                .iter()
                .find_map(|article| {
                    state
                        .triage()
                        .triage_model_for_url(&article.url)
                        .map(str::to_owned)
                })
                .unwrap_or_else(|| "fixture-triage".into()),
            _ => "fixture-scoring".into(),
        });
    }
    state.set_llm_metadata(versions, models);
    state.mark_triage_metadata_ready();
    state.rebuild_saved_results();
    let score_only: Vec<_> = state
        .signal_candidate()
        .iter_completed()
        .filter_map(|(url, _)| {
            let entry = state.saved_results_for_url(url)?;
            (state.triage().article_content_hash(url).is_none())
                .then(|| entry.article.content_hash.clone())
        })
        .collect();
    for hash in score_only {
        if let Some(key) = state.current_triage_cache_key(&hash) {
            state.store_frozen_triage_result(
                key,
                crate::ArticleTriageResult {
                    category: "news".into(),
                    priority: 3,
                    tags: vec![],
                    rationale: "fixture".into(),
                    input_tokens: 1,
                    output_tokens: 1,
                },
                "2026-01-01T00:00:00Z".into(),
            );
        }
        if let Ok(key) = state.current_summary_cache_key(&hash) {
            state.store_summary_result(
                key,
                crate::ArticleSummaryResult {
                    title: "Fixture".into(),
                    summary: "Summary".into(),
                    key_points: vec![],
                    input_tokens: 1,
                    output_tokens: 1,
                    entities: Default::default(),
                },
                "2026-01-01T00:00:00Z".into(),
            );
        }
    }
    let triage: Vec<_> = state
        .triage()
        .articles()
        .iter()
        .filter_map(|a| {
            state
                .triage()
                .result_for_url(&a.url)
                .map(|result| (a.content_hash.clone(), result.clone()))
        })
        .collect();
    for (hash, result) in triage {
        if let Some(key) = state.current_triage_cache_key(&hash) {
            state.store_frozen_triage_result(key, result, "2026-01-01T00:00:00Z".into());
        }
    }
    let summaries: Vec<_> = state
        .briefing()
        .articles()
        .iter()
        .filter_map(|a| {
            state
                .briefing()
                .summary_for_url(&a.url)
                .map(|result| (a.content_hash.clone(), result.clone()))
        })
        .collect();
    for (hash, result) in summaries {
        if let Ok(key) = state.current_summary_cache_key(&hash) {
            state.store_summary_result(key, result, "2026-01-01T00:00:00Z".into());
        }
    }
    let missing_summaries: Vec<_> = state
        .signal_candidate()
        .iter_completed()
        .filter_map(|(url, _)| {
            state
                .saved_results_for_url(url)
                .filter(|entry| entry.summary.is_none())
                .map(|entry| entry.article.content_hash.clone())
        })
        .collect();
    for hash in missing_summaries {
        if let Ok(key) = state.current_summary_cache_key(&hash) {
            state.store_summary_result(
                key,
                crate::ArticleSummaryResult {
                    title: "Fixture".into(),
                    summary: "Summary".into(),
                    key_points: vec![],
                    input_tokens: 0,
                    output_tokens: 0,
                    entities: Default::default(),
                },
                "2026-01-01T00:00:00Z".into(),
            );
        }
    }
    let scores: Vec<_> = state
        .signal_candidate()
        .iter_completed()
        .map(|(url, result)| (url.to_string(), result.clone()))
        .collect();
    for (url, result) in scores {
        if let Some(key) = state
            .saved_results_for_url(&url)
            .and_then(|entry| entry.signal_key.clone())
        {
            state.store_signal_candidate_result(key, result, "2026-01-01T00:00:00Z".into());
        }
    }
    state.discard_fixture_pending_results();
    state.rebuild_saved_results();
}
