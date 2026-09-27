use crate::{AppState, Effect};
use engine_logging::{engine_info, engine_warn};
use std::collections::HashMap;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum StartTarget {
    Triage,
    Summaries,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct PendingStart {
    pub target: StartTarget,
    pub configuration_request: Option<u64>,
    preparation_requested: bool,
}

pub(super) fn begin(state: &mut AppState, target: StartTarget) -> Vec<Effect> {
    if state.processing_start.is_some() {
        return Vec::new();
    }
    super::pipeline_run::arm_legacy(state);
    let reuse = state.pipeline_ready();
    state.summaries_follow_triage = target == StartTarget::Triage;
    let configuration_request = if reuse {
        None
    } else {
        Some(state.pre_triage_coordinator.allocate_request_id())
    };
    state.processing_start = Some(PendingStart {
        target,
        configuration_request,
        preparation_requested: false,
    });
    engine_info!(
        "[processing-start] target={target:?} configuration_request={configuration_request:?}"
    );
    state.mark_dirty();
    if let Some(request_id) = configuration_request {
        state.start_summary_cache_run();
        state.processing_budget = None;
        vec![Effect::LoadProcessingConfiguration {
            request_id,
            require_triage_context: target == StartTarget::Triage,
        }]
    } else {
        resume(state)
    }
}

pub(super) fn fail(state: &mut AppState, reason: String) {
    if let Some(start) = state.processing_start.take() {
        engine_warn!(
            "[processing-start] target={:?} failed: {}",
            start.target,
            reason
        );
        match start.target {
            StartTarget::Triage
                if !matches!(state.triage().phase(), crate::TriagePhase::Complete) =>
            {
                state.triage_mut().fail(reason)
            }
            StartTarget::Summaries
                if !matches!(state.briefing().phase(), crate::BriefingPhase::Complete) =>
            {
                state.briefing_mut().fail(reason)
            }
            _ => {}
        }
        state.summaries_follow_triage = false;
        if let Some(run) = state.pipeline_admission.as_mut() {
            run.armed = false;
            run.fresh_load = false;
            run.initial_admitted = true;
        }
        state.mark_dirty();
    }
}

pub(super) fn resume(state: &mut AppState) -> Vec<Effect> {
    let Some(start) = state.processing_start.clone() else {
        return Vec::new();
    };
    // Only an in-flight load blocks the start. Waiting for refresh demand to go idle
    // would defer the start until downloads end, because every finished download
    // records new demand; later arrivals are admitted as later waves instead.
    if start.configuration_request.is_some() || state.triage_in_flight_request_id().is_some() {
        return Vec::new();
    }
    let Some(budget) = state.processing_budget else {
        return Vec::new();
    };
    if let Some(run) = state.pipeline_admission.as_mut() {
        run.configured = true;
        if run.scope == crate::PipelineRunScope::Continue {
            state.processing_start = None;
            return Vec::new();
        }
        if run.fresh_load {
            run.fresh_load = false;
            state
                .processing_start
                .as_mut()
                .unwrap()
                .preparation_requested = true;
            let mut urls = state.ordered_completed_job_urls_snapshot();
            // Legacy starts can carry prepared articles before their restored job snapshot.
            urls.extend(
                state
                    .pre_triage()
                    .window_articles()
                    .map(|(_, a)| a.url.clone()),
            );
            urls.extend(state.triage().articles().iter().map(|a| a.url.clone()));
            let mut seen = std::collections::HashSet::new();
            urls.retain(|url| seen.insert(url.clone()));
            let request_id = state.pre_triage_coordinator.begin_preparation_load();
            state.set_triage_in_flight(request_id);
            return vec![Effect::LoadArticlesForTriage {
                request_id,
                ordered_urls: urls,
                since_utc: state.briefing_since_utc(),
                held: state.pre_triage().held_articles(),
            }];
        }
    }
    if start.target == StartTarget::Triage && !state.triage_metadata_ready() {
        fail(state, "Triage configuration is unavailable".into());
        return Vec::new();
    }
    let (urls, valid) = match start.target {
        StartTarget::Triage => {
            let articles = state.pre_triage().resolved_included_articles();
            let valid = !articles.is_empty()
                && articles
                    .iter()
                    .all(|a| state.pre_triage().preparation_budget(&a.url) == Some(budget));
            (
                state
                    .pre_triage()
                    .entries()
                    .iter()
                    .map(|e| e.key.url.clone())
                    .collect(),
                valid,
            )
        }
        StartTarget::Summaries => {
            let urls = state.archive_corpus().ordered_urls().to_vec();
            let prepared: HashMap<_, _> = state
                .triage()
                .articles()
                .iter()
                .map(|a| (a.url.as_str(), a.preparation_budget))
                .collect();
            let valid = !urls.is_empty()
                && urls
                    .iter()
                    .all(|url| prepared.get(url.as_str()) == Some(&Some(budget)));
            (urls, valid)
        }
    };
    if valid {
        state.processing_start = None;
        return match start.target {
            StartTarget::Triage => super::triage::start_triage_from_pretriage(state),
            StartTarget::Summaries => super::briefing::start_summaries_from_triage(state),
        };
    }
    if urls.is_empty() {
        state.processing_start = None;
        super::waves::admit_triage(state, Vec::new());
        return Vec::new();
    }
    if start.preparation_requested {
        engine_warn!("[processing-start] target={:?} article preparation unavailable budget={} urls={} retry={}", start.target, budget, urls.len(), start.preparation_requested);
        fail(state, "Article preparation is unavailable for the current configuration; refresh and run triage again".to_string());
        return Vec::new();
    }
    let mut held = state.pre_triage().held_articles();
    if start.target == StartTarget::Summaries {
        // A standalone summary session may outlive pre-triage preparation.
        // Only claim preparation actually held by the triage session.
        held = state
            .triage()
            .articles()
            .iter()
            .filter_map(|a| {
                Some(harvester_engine::HeldArticle {
                    url: a.url.clone(),
                    content_hash: a.content_hash.clone(),
                    preparation_budget: a.preparation_budget?,
                })
            })
            .collect();
    }
    let mut window_urls = state.ordered_completed_job_urls_snapshot();
    window_urls.extend(urls);
    let mut seen = std::collections::HashSet::new();
    window_urls.retain(|url| seen.insert(url.clone()));
    let request_id = state.pre_triage_coordinator.begin_preparation_load();
    engine_info!(
        "[processing-start] target={:?} reprepare request_id={} budget={} urls={}",
        start.target,
        request_id,
        budget,
        window_urls.len()
    );
    state.set_triage_in_flight(request_id);
    state
        .processing_start
        .as_mut()
        .unwrap()
        .preparation_requested = true;
    vec![Effect::LoadArticlesForTriage {
        request_id,
        ordered_urls: window_urls,
        since_utc: state.briefing_since_utc(),
        held,
    }]
}
