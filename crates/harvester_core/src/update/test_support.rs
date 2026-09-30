use crate::{AppState, Effect, LoadedArticle, Msg};

/// Emulate configuration and preparation IO for existing in-memory reducer fixtures.
/// Contract tests use crate::update directly and inspect each asynchronous boundary.
pub(crate) fn update(state: AppState, msg: Msg) -> (AppState, Vec<Effect>) {
    let mut articles = state.pre_triage().resolved_included_articles();
    if articles.is_empty() {
        articles = state
            .triage()
            .articles()
            .iter()
            .map(|a| LoadedArticle {
                url: a.url.clone(),
                source_title: a.source_title.clone(),
                prepared_text: a.prepared_text.clone(),
                content_hash: a.content_hash.clone(),
                fetched_utc: a.fetched_utc.clone(),
            })
            .collect();
    }
    let (mut state, mut effects) = crate::update(state, msg);
    if effects
        .iter()
        .any(|e| matches!(e, Effect::LoadProcessingConfiguration { .. }))
    {
        (state, effects) =
            crate::fixture_support::complete_processing_configuration(state, effects, 100_000);
        if let Some(request_id) = effects.iter().find_map(|e| match e {
            Effect::LoadArticlesForTriage { request_id, .. } => Some(*request_id),
            _ => None,
        }) {
            (state, effects) = crate::update(
                state,
                Msg::TriageArticlesLoaded {
                    request_id,
                    delta: harvester_engine::TriageArticleDelta::full_window(articles, 100_000),
                },
            );
        }
    }
    (state, effects)
}

/// Build a configured run around explicitly seeded admitted-work fixtures.
/// Production and lifecycle tests enter through messages instead.
pub(crate) fn arm_admitted(state: &mut AppState) {
    super::pipeline_run::begin_run_if_needed(state);
    let mut run = crate::pipeline_waves::PipelineAdmission::new(
        crate::PipelineRunScope::Resume,
        true,
        Default::default(),
    );
    state.start_summary_cache_run();
    state.mark_briefing_metadata_ready();
    run.configured = true;
    run.fresh_load = false;
    run.intake_open = false;
    state.pipeline_admission = Some(run);
    state.set_pipeline_run_phase(crate::PipelineRunPhase::AwaitingSettle);
}
