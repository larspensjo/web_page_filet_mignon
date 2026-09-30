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
