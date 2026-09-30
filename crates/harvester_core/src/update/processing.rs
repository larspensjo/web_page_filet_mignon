use crate::{AppState, Effect};
use engine_logging::{engine_info, engine_warn};

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct PendingStart {
    pub configuration_request: Option<u64>,
    preparation_requested: bool,
}

pub(super) fn begin(state: &mut AppState) -> Vec<Effect> {
    if state.processing_start.is_some() {
        return Vec::new();
    }
    let reuse = state.pipeline_ready() && state.processing_budget.is_some();
    state.summaries_follow_triage = true;
    let configuration_request = if reuse {
        None
    } else {
        Some(state.pre_triage_coordinator.allocate_request_id())
    };
    state.processing_start = Some(PendingStart {
        configuration_request,
        preparation_requested: false,
    });
    engine_info!("[processing-start] configuration_request={configuration_request:?}");
    state.mark_dirty();
    if let Some(request_id) = configuration_request {
        state.start_summary_cache_run();
        state.processing_budget = None;
        vec![Effect::LoadProcessingConfiguration {
            request_id,
            require_triage_context: true,
        }]
    } else {
        resume(state)
    }
}

pub(super) fn fail(state: &mut AppState, reason: String) {
    if state.processing_start.take().is_some() {
        engine_warn!("[processing-start] failed: {}", reason);
        if !matches!(state.triage().phase(), crate::TriagePhase::Complete) {
            state.triage_mut().fail(reason)
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
    let current_window_has_members = !state.ordered_completed_job_urls_snapshot().is_empty()
        || state.pre_triage().window_articles().next().is_some()
        || !state.triage().articles().is_empty();
    let wait_for_first_download = state
        .pipeline_admission
        .as_ref()
        .is_some_and(|run| run.scope == crate::PipelineRunScope::Full)
        && (state.is_poll_in_progress() || state.batch_observation().jobs_in_flight > 0)
        && !current_window_has_members;
    if let Some(run) = state.pipeline_admission.as_mut() {
        run.configured = true;

        if run.fresh_load {
            if run.scope == crate::PipelineRunScope::Full && wait_for_first_download {
                return Vec::new();
            }
            run.fresh_load = false;
            state
                .processing_start
                .as_mut()
                .unwrap()
                .preparation_requested = true;
            let mut urls = state.ordered_completed_job_urls_snapshot();
            // Restored sessions can carry prepared articles before their job snapshot.
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
    if !state.triage_metadata_ready() {
        fail(state, "Triage configuration is unavailable".into());
        return Vec::new();
    }
    let articles = state.pre_triage().resolved_included_articles();
    let valid = !articles.is_empty()
        && articles
            .iter()
            .all(|a| state.pre_triage().preparation_budget(&a.url) == Some(budget));
    let urls: Vec<String> = state
        .pre_triage()
        .entries()
        .iter()
        .map(|e| e.key.url.clone())
        .collect();
    if valid {
        state.processing_start = None;
        return super::triage::start_triage_from_pretriage(state);
    }
    if urls.is_empty() || articles.is_empty() {
        state.processing_start = None;
        super::waves::admit_triage(state, Vec::new());
        return Vec::new();
    }
    if start.preparation_requested {
        engine_warn!(
            "[processing-start] article preparation unavailable budget={} urls={} retry={}",
            budget,
            urls.len(),
            start.preparation_requested
        );
        fail(state, "Article preparation is unavailable for the current configuration; refresh and run triage again".to_string());
        return Vec::new();
    }
    let held = state.pre_triage().held_articles();
    let mut window_urls = state.ordered_completed_job_urls_snapshot();
    window_urls.extend(urls);
    let mut seen = std::collections::HashSet::new();
    window_urls.retain(|url| seen.insert(url.clone()));
    let request_id = state.pre_triage_coordinator.begin_preparation_load();
    engine_info!(
        "[processing-start] reprepare request_id={} budget={} urls={}",
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
