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
    state.set_pipeline_run_phase(crate::PipelineRunPhase::Requested);
}

/// Seed completed triage results, then release the triage wave through the live
/// downstream scheduler. No retired loader or aggregate orchestration is involved.
pub(crate) fn summarize(
    mut state: AppState,
    articles: Vec<LoadedArticle>,
) -> (AppState, Vec<Effect>) {
    if !state.pipeline_ready() {
        arm_admitted(&mut state);
    }
    let mut triage = crate::TriageSession::new_loading(None);
    let mut members = Vec::new();
    for (index, article) in articles.into_iter().enumerate() {
        let key = state.current_triage_cache_key(&article.content_hash);
        members.push((article.url.clone(), article.content_hash.clone()));
        triage.admit(article, key, Some(100_000));
        triage.complete_article(
            index,
            crate::ArticleTriageResult {
                category: "tech".into(),
                priority: 3,
                tags: vec![],
                rationale: "fixture".into(),
                input_tokens: 0,
                output_tokens: 0,
            },
        );
    }
    triage.complete();
    state.set_triage(triage);
    super::waves::release(&mut state, crate::PipelineStage::Triaging, members);
    crate::update(state, Msg::PipelineRunAdvance)
}
