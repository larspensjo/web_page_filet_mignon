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
        PromptId::AggregateBriefing,
        PromptId::BriefingExecutiveSummary,
        PromptId::BriefingNextItem,
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
