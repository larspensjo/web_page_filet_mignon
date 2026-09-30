use crate::run_progress::ACTIVITY_REASON_MAX_CHARS;
use crate::state::PollPipelineJobSnapshot;
use crate::{
    ActivityOutcome, AppState, Effect, JobResultKind, LlmRequestState, LlmResultKind, Msg,
    PipelineRunPhase, PipelineStage, RunCompletionNotice, RunProgress, StageStatus,
};
use harvester_engine::llm::prompt::PromptId;
use harvester_engine::truncate_to_char_boundary;

pub(super) struct ProgressBefore {
    event: ProgressEvent,
}

enum ProgressEvent {
    None,
    PollStarted {
        total: u32,
    },
    SourcePollCompleted,
    SourcePollFailed,
    AllSourcesPollEnded,
    JobProgress {
        job_id: crate::JobId,
        snapshot: PollPipelineJobSnapshot,
    },
    JobDone {
        job_id: crate::JobId,
        snapshot: PollPipelineJobSnapshot,
        outcome: ActivityOutcome,
    },
    Loading,
    LoadingFailed,
    LlmCompleted {
        stage: Option<PipelineStage>,
        activity: Option<LlmActivity>,
    },
}

struct LlmActivity {
    url: String,
    title: Option<String>,
    outcome: ActivityOutcome,
}

pub(super) fn progress_before(state: &AppState, msg: &Msg) -> ProgressBefore {
    if !state.run_progress_is_active() {
        return ProgressBefore {
            event: ProgressEvent::None,
        };
    }

    let event = match msg {
        Msg::PollStarted { total } => ProgressEvent::PollStarted {
            total: *total as u32,
        },
        Msg::SourcePollCompleted { .. } => ProgressEvent::SourcePollCompleted,
        Msg::SourcePollFailed { .. } => ProgressEvent::SourcePollFailed,
        Msg::AllSourcesPollEnded => ProgressEvent::AllSourcesPollEnded,
        Msg::JobProgress { job_id, .. } => {
            let needs_url = state
                .run_progress()
                .is_some_and(|run| !run.download_started_job_ids.contains(job_id));
            state.poll_pipeline_job_snapshot(*job_id, needs_url).map_or(
                ProgressEvent::None,
                |snapshot| ProgressEvent::JobProgress {
                    job_id: *job_id,
                    snapshot,
                },
            )
        }
        Msg::JobDone { job_id, result, .. } => state
            .poll_pipeline_job_snapshot(*job_id, true)
            .map_or(ProgressEvent::None, |snapshot| ProgressEvent::JobDone {
                job_id: *job_id,
                snapshot,
                outcome: job_outcome(result),
            }),
        Msg::TriageArticlesLoadProgress { .. } | Msg::TriageArticlesLoaded { .. } => {
            ProgressEvent::Loading
        }
        Msg::TriageArticlesLoadFailed { request_id, .. }
            if state.triage_in_flight_request_id() == Some(*request_id) =>
        {
            ProgressEvent::LoadingFailed
        }
        Msg::LlmCompleted {
            request_id, result, ..
        } => llm_event(state, *request_id, result),
        _ => ProgressEvent::None,
    };

    ProgressBefore { event }
}

pub(super) fn begin_run_if_needed(state: &mut AppState) {
    if state.run_progress_is_active() {
        return;
    }
    let run_id = state.allocate_run_id();
    let signal = state.signal_candidate();
    let progress = RunProgress::new(
        run_id,
        state.last_observed_utc(),
        signal.completed_count() as usize,
        signal.failed_count(),
        signal.enqueued_count(),
    );
    state.pipeline_admission = None;
    state.replace_run_progress(progress);
    state.clear_run_completion_notice();
    state.mark_dirty();
}

pub(super) fn handle_pipeline_requested(
    state: &mut AppState,
    scope: crate::PipelineRunScope,
) -> Vec<Effect> {
    if state.pipeline_run_phase() == PipelineRunPhase::Stopping
        || state.session() == crate::SessionState::Finishing
    {
        engine_logging::engine_info!(
            "[run-request] run_id={} scope={scope:?} ignored=stopping",
            state.run_progress().map_or(0, |run| run.run_id)
        );
        return Vec::new();
    }
    if state.run_progress_is_active() && state.pipeline_admission.is_some() {
        if scope == crate::PipelineRunScope::Full || state.is_poll_in_progress() {
            if let Some(run) = state.pipeline_admission.as_mut() {
                run.scope = crate::PipelineRunScope::Full;
            }
        }
        return Vec::new();
    }
    if !state.run_progress_is_active() {
        begin_run_if_needed(state);
    }
    let polling = state.is_poll_in_progress() || state.batch_observation().jobs_in_flight > 0;
    let scope = if scope == crate::PipelineRunScope::Resume && polling {
        crate::PipelineRunScope::Full
    } else {
        scope
    };
    let previous = state
        .pre_triage()
        .window_articles()
        .map(|(_, article)| (article.url.clone(), article.content_hash.clone()))
        .collect();
    state.pipeline_admission = Some(crate::pipeline_waves::PipelineAdmission::new(
        scope,
        state.triage_ai_available(),
        previous,
    ));
    state.set_pipeline_run_phase(PipelineRunPhase::Requested);
    let mut effects = Vec::new();
    if scope == crate::PipelineRunScope::Full {
        let pending_urls = state.take_pending_intake_urls();
        if !pending_urls.is_empty() {
            let retryable = state.pending_intake_for_reingest(pending_urls);
            let intake = state.ingest_urls(retryable, chrono::Utc::now());
            engine_logging::engine_info!(
                "[pending-intake] run_id={} ingested={} skipped={}",
                state.run_progress().map_or(0, |run| run.run_id),
                intake.enqueued,
                intake.skipped
            );
            effects.extend(intake.effects);
        }
    }
    if scope == crate::PipelineRunScope::Full && !polling {
        effects.extend(super::polling::handle_poll_sources_clicked(state));
    }
    if state.pipeline_run_armed() {
        effects.extend(super::processing::begin(state));
    }
    effects
}

pub(super) fn handle_pipeline_advance(state: &mut AppState) -> Vec<Effect> {
    // Every reducer step advances the lifecycle; hosts may also request an
    // explicit advance while waiting for effects to complete.
    if state.pipeline_run_phase() == PipelineRunPhase::Stopping {
        Vec::new()
    } else {
        super::processing::resume(state)
    }
}

pub(super) fn finish_if_settled(state: &mut AppState) {
    if state.pipeline_run_phase() == PipelineRunPhase::Stopping {
        super::waves::record_progress(state);
        if state.pipeline_activity().is_settled() {
            settle_stopped_run(state);
        }
        return;
    }
    if !state.run_progress_is_active() {
        if state.session() == crate::SessionState::Finishing
            && state.pipeline_activity().is_settled()
        {
            state.reset_session_to_idle();
        }
        return;
    }
    let activity = state.pipeline_activity();
    let intake_done = activity.poll_in_progress == 0
        && activity.jobs_pending_or_in_flight == 0
        && !activity.intake_refresh_pending
        && activity.import_in_flight == 0;
    let mut can_finish = true;
    if let Some(run) = state.pipeline_admission.as_mut() {
        can_finish = !run.awaiting_rearm
            && (!run.fresh_load || !run.armed)
            && (!run.armed || run.configured);
        if intake_done && can_finish {
            run.intake_open = false;
        }
        can_finish &= !run.intake_open;
    }
    super::waves::record_progress(state);
    if can_finish && activity.is_settled() {
        settle_run(state);
    }
}
fn settle_run(state: &mut AppState) {
    let now = state.last_observed_utc();
    let completed_at_utc = now.unwrap_or_default();
    let completed_count = state.signal_candidate().completed_count() as usize;
    let Some(run) = state.run_progress_mut() else {
        return;
    };
    let run_id = run.run_id;
    let new_result_count = completed_count.saturating_sub(run.signal_completed_at_reset);
    run.settle(now);
    if let Some(admission) = state.pipeline_admission.as_mut() {
        admission.armed = false;
    }
    state.finalize_summary_cache_run();
    state.set_run_completion_notice(RunCompletionNotice {
        new_result_count,
        completed_at_utc,
    });
    state.set_pipeline_run_phase(PipelineRunPhase::Idle);
    clear_export_unavailable_status(state);
    engine_logging::engine_info!("[run-terminal] run_id={run_id} outcome=completed");
    state.mark_dirty();
}

fn settle_stopped_run(state: &mut AppState) {
    let now = state.last_observed_utc();
    let run_id = state.run_progress().map(|run| run.run_id);
    if let Some(run) = state.run_progress_mut() {
        if !run.terminal {
            run.settle(now);
        }
    }
    if let Some(admission) = state.pipeline_admission.as_mut() {
        admission.armed = false;
        admission.intake_open = false;
    }
    state.finalize_summary_cache_run();
    state.reset_session_to_idle();
    state.set_pipeline_run_phase(PipelineRunPhase::Idle);
    clear_export_unavailable_status(state);
    if let Some(run_id) = run_id {
        engine_logging::engine_info!("[run-terminal] run_id={run_id} outcome=stopped");
    }
    state.mark_dirty();
}

pub(super) fn handle_stop_for_pipeline(state: &mut AppState) {
    let preserved_downloads = state.preserve_unstarted_downloads_for_retry();
    if state.run_progress_is_active() {
        state.set_pipeline_run_phase(PipelineRunPhase::Stopping);
        if let Some(run) = state.run_progress_mut() {
            run.begin_stopping();
        }
    }
    let in_flight_llm = state.article_model_requests_in_flight();
    let in_flight_download = state.run_progress().map_or(0, |run| {
        run.download_started_job_ids
            .difference(&run.download_finished_job_ids)
            .count()
    });
    let triage = state.triage_mut().withdraw_pending();
    let summary = state.briefing_mut().withdraw_pending();
    let scoring = state.signal_candidate_mut().withdraw_pending();
    for url in &scoring {
        state.clear_signal_candidate_input_snapshot(url);
    }
    for queue in &mut state.pipeline_waves.pending {
        queue.clear();
    }
    state.processing_start = None;
    state.pre_triage_coordinator.close_intake();
    let _ = state.take_pre_triage_refresh_evaluation_request();
    if let Some(run) = state.pipeline_admission.as_mut() {
        run.armed = false;
        run.intake_open = false;
    }
    engine_logging::engine_info!(
        "[run-stop] run_id={} withdrawn_triage={} withdrawn_summary={} withdrawn_scoring={} in_flight_llm={} in_flight_download={}",
        run_id_for_stop(state),
        triage,
        summary,
        scoring.len(),
        in_flight_llm,
        in_flight_download
    );
    if preserved_downloads > 0 {
        engine_logging::engine_info!(
            "[pending-intake] operation=stop preserved_unstarted_downloads={preserved_downloads}"
        );
    }
    state.mark_dirty();
}

fn run_id_for_stop(state: &AppState) -> u64 {
    state.run_progress().map_or(0, |run| run.run_id)
}

fn clear_export_unavailable_status(state: &mut AppState) {
    if state.briefing_checkpoint_status_message()
        == Some(crate::state::EXPORT_UNAVAILABLE_STATUS_MESSAGE)
    {
        state.set_briefing_checkpoint_status_message(None);
    }
}

pub(super) fn dismiss_run_notice(state: &mut AppState) {
    if state.clear_run_completion_notice() {
        state.mark_dirty();
    }
}

pub(super) fn record_progress_after(state: &mut AppState, before: ProgressBefore) {
    if !state.run_progress_is_active() {
        return;
    }
    let may_settle_poll_only_run = matches!(
        &before.event,
        ProgressEvent::AllSourcesPollEnded | ProgressEvent::JobDone { .. }
    );
    let now = state.last_observed_utc();

    match before.event {
        ProgressEvent::None => {}
        ProgressEvent::PollStarted { total } => {
            state.run_progress_mut().expect("active run").activate(
                PipelineStage::ScanningSources,
                total,
                now,
            );
        }
        ProgressEvent::SourcePollCompleted => {
            let download_total = state.poll_pipeline_job_total();
            let run = state.run_progress_mut().expect("active run");
            let record = run.stage_mut(PipelineStage::ScanningSources);
            record.completed = record.completed.saturating_add(1);
            if let Some(total) = download_total {
                let downloads = run.stage_mut(PipelineStage::DownloadingArticles);
                if matches!(downloads.status, StageStatus::Active) {
                    downloads.total = downloads.total.max(total);
                }
            }
        }
        ProgressEvent::SourcePollFailed => {
            let run = state.run_progress_mut().expect("active run");
            let record = run.stage_mut(PipelineStage::ScanningSources);
            record.failed = record.failed.saturating_add(1);
        }
        ProgressEvent::AllSourcesPollEnded => {
            let run = state.run_progress_mut().expect("active run");
            run.finish(PipelineStage::ScanningSources, now);
            let downloads = run.stage_mut(PipelineStage::DownloadingArticles);
            if matches!(downloads.status, StageStatus::Active)
                && downloads.completed.saturating_add(downloads.failed) == downloads.total
            {
                run.finish(PipelineStage::DownloadingArticles, now);
            }
        }
        ProgressEvent::JobProgress { job_id, snapshot } => {
            let run = state.run_progress_mut().expect("active run");
            run.activate(PipelineStage::DownloadingArticles, snapshot.total, now);
            if run.download_started_job_ids.insert(job_id) {
                if let Some(url) = snapshot.url {
                    run.push_activity(
                        url,
                        None,
                        PipelineStage::DownloadingArticles,
                        ActivityOutcome::Started,
                    );
                }
            }
        }
        ProgressEvent::JobDone {
            job_id,
            snapshot,
            outcome,
        } => {
            let run = state.run_progress_mut().expect("active run");
            if !run.download_finished_job_ids.insert(job_id) {
                return;
            }
            run.activate(PipelineStage::DownloadingArticles, snapshot.total, now);
            let record = run.stage_mut(PipelineStage::DownloadingArticles);
            match outcome {
                ActivityOutcome::Succeeded => {
                    record.completed = record.completed.saturating_add(1);
                }
                ActivityOutcome::Failed { .. } => {
                    record.failed = record.failed.saturating_add(1);
                }
                ActivityOutcome::Started | ActivityOutcome::Skipped { .. } => {}
            }
            let settled = record.completed.saturating_add(record.failed);
            if snapshot.source_scan_done && settled == snapshot.total {
                run.finish(PipelineStage::DownloadingArticles, now);
            }
            if let Some(url) = snapshot.url {
                run.push_activity(url, None, PipelineStage::DownloadingArticles, outcome);
            }
        }
        ProgressEvent::Loading => {}
        ProgressEvent::LoadingFailed => {
            let run = state.run_progress_mut().expect("active run");
            run.counts(PipelineStage::LoadingArticles, 0, 1, 1, now);
            run.finish(PipelineStage::LoadingArticles, now);
        }
        ProgressEvent::LlmCompleted { stage, activity } => {
            if let (Some(stage), Some(activity)) = (stage, activity) {
                let outcome =
                    activity_outcome_after(state, stage, &activity.url).unwrap_or(activity.outcome);
                state.run_progress_mut().expect("active run").push_activity(
                    activity.url,
                    activity.title,
                    stage,
                    outcome,
                );
            }
        }
    }

    if may_settle_poll_only_run
        && matches!(state.pipeline_run_phase(), PipelineRunPhase::Idle)
        && state.pipeline_activity().is_settled()
    {
        settle_run(state);
    }
}

fn llm_event(state: &AppState, request_id: u64, result: &LlmResultKind) -> ProgressEvent {
    let stage = state
        .llm_request_state(request_id)
        .and_then(|entry| match entry {
            LlmRequestState::Pending { prompt_id } | LlmRequestState::Deferred { prompt_id } => {
                match prompt_id {
                    PromptId::ArticleTriage => Some(PipelineStage::Triaging),
                    PromptId::ArticleSummary => Some(PipelineStage::Summarizing),
                    PromptId::ArticleSignalCandidate => Some(PipelineStage::ScoringSignals),
                    _ => None,
                }
            }
            LlmRequestState::Completed { .. } | LlmRequestState::Failed { .. } => None,
        });
    let activity = stage.and_then(|stage| {
        let (url, title) = llm_article(state, stage, request_id)?;
        Some(LlmActivity {
            url,
            title,
            outcome: llm_outcome(result),
        })
    });
    ProgressEvent::LlmCompleted { stage, activity }
}

fn llm_article(
    state: &AppState,
    stage: PipelineStage,
    request_id: u64,
) -> Option<(String, Option<String>)> {
    match stage {
        PipelineStage::Triaging => {
            let article = state
                .triage()
                .articles()
                .get(state.triage().find_article_by_request_id(request_id)?)?;
            Some((article.url.clone(), article.source_title.clone()))
        }
        PipelineStage::Summarizing => {
            let article = state
                .briefing()
                .articles()
                .get(state.briefing().find_article_by_request_id(request_id)?)?;
            Some((article.url.clone(), article.source_title.clone()))
        }
        PipelineStage::ScoringSignals => {
            let url = state.signal_candidate().url_for_request(request_id)?;
            let title = state
                .signal_candidate_input_snapshot(url)
                .map(|snapshot| snapshot.title.clone())
                .filter(|title| !title.is_empty());
            Some((url.to_string(), title))
        }
        _ => None,
    }
}

fn activity_outcome_after(
    state: &AppState,
    stage: PipelineStage,
    url: &str,
) -> Option<ActivityOutcome> {
    match stage {
        PipelineStage::Triaging => state
            .triage()
            .articles()
            .iter()
            .find(|article| article.url == url)
            .and_then(|article| match &article.triage_state {
                crate::triage::ArticleTriageState::Completed { .. } => {
                    Some(ActivityOutcome::Succeeded)
                }
                crate::triage::ArticleTriageState::Failed { reason } => {
                    Some(ActivityOutcome::Failed {
                        reason: bounded_reason(reason),
                    })
                }
                crate::triage::ArticleTriageState::Deferred => Some(ActivityOutcome::Skipped {
                    reason: "deferred to batch".into(),
                }),
                crate::triage::ArticleTriageState::Pending
                | crate::triage::ArticleTriageState::InProgress { .. } => None,
            }),
        PipelineStage::Summarizing => state
            .briefing()
            .articles()
            .iter()
            .find(|article| article.url == url)
            .and_then(|article| match &article.summary_state {
                crate::briefing::ArticleSummaryState::Completed { .. } => {
                    Some(ActivityOutcome::Succeeded)
                }
                crate::briefing::ArticleSummaryState::Failed { reason } => {
                    Some(ActivityOutcome::Failed {
                        reason: bounded_reason(reason),
                    })
                }
                crate::briefing::ArticleSummaryState::Deferred => Some(ActivityOutcome::Skipped {
                    reason: "deferred to batch".into(),
                }),
                crate::briefing::ArticleSummaryState::Pending
                | crate::briefing::ArticleSummaryState::InProgress { .. } => None,
            }),
        PipelineStage::ScoringSignals => match state.signal_candidate().state_for(url)? {
            crate::SignalCandidateState::Completed { .. } => Some(ActivityOutcome::Succeeded),
            crate::SignalCandidateState::Failed { reason } => Some(ActivityOutcome::Failed {
                reason: bounded_reason(reason),
            }),
            crate::SignalCandidateState::Deferred => Some(ActivityOutcome::Skipped {
                reason: "deferred to batch".into(),
            }),
            crate::SignalCandidateState::Pending | crate::SignalCandidateState::Scoring { .. } => {
                None
            }
        },
        _ => None,
    }
}

fn job_outcome(result: &JobResultKind) -> ActivityOutcome {
    match result {
        JobResultKind::Success => ActivityOutcome::Succeeded,
        JobResultKind::Failed { reason } => ActivityOutcome::Failed {
            reason: bounded_reason(reason),
        },
    }
}

fn llm_outcome(result: &LlmResultKind) -> ActivityOutcome {
    match result {
        LlmResultKind::Success { .. } => ActivityOutcome::Succeeded,
        LlmResultKind::DeferredToBatch => ActivityOutcome::Skipped {
            reason: "deferred to batch".into(),
        },
        LlmResultKind::ValidationFailed { reason, .. }
        | LlmResultKind::QuotaExhausted { reason, .. }
        | LlmResultKind::RateLimited { reason }
        | LlmResultKind::Failed { reason } => ActivityOutcome::Failed {
            reason: bounded_reason(reason),
        },
    }
}

fn bounded_reason(reason: &str) -> String {
    truncate_to_char_boundary(reason, ACTIVITY_REASON_MAX_CHARS).to_string()
}

#[cfg(test)]
mod tests;
