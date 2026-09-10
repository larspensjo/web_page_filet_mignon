use crate::run_progress::ACTIVITY_REASON_MAX_CHARS;
use crate::state::PollPipelineJobSnapshot;
use crate::{
    ActivityOutcome, AppState, BatchNextAction, Effect, JobResultKind, LlmRequestState,
    LlmResultKind, Msg, PipelineRunPhase, PipelineStage, RunCompletionNotice, RunProgress,
    StageStatus,
};
use harvester_engine::llm::prompt::PromptId;
use harvester_engine::truncate_to_char_boundary;

pub(super) struct ProgressBefore {
    counts_as_work_message: bool,
    event: ProgressEvent,
    signal_enqueued_before: u32,
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
    LoadingProgress {
        completed: u32,
        total: u32,
    },
    LoadingDone {
        total: u32,
    },
    LoadingFailed,
    TriageRequested {
        can_start: bool,
    },
    SummariesLoaded {
        was_loading: bool,
    },
    AiStateChanged,
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
    let counts_as_work_message = !matches!(msg, Msg::NoOp | Msg::PipelineRunAdvance);
    if !state.run_progress_is_active() {
        return ProgressBefore {
            counts_as_work_message,
            event: ProgressEvent::None,
            signal_enqueued_before: 0,
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
        Msg::TriageArticlesLoadProgress {
            request_id,
            files_scanned,
            files_total,
        } if state.triage_in_flight_request_id() == Some(*request_id) => {
            ProgressEvent::LoadingProgress {
                completed: *files_scanned as u32,
                total: *files_total as u32,
            }
        }
        Msg::TriageArticlesLoaded {
            request_id,
            articles,
        } if state.triage_in_flight_request_id() == Some(*request_id) => {
            ProgressEvent::LoadingDone {
                total: articles.len() as u32,
            }
        }
        Msg::TriageArticlesLoadFailed { request_id, .. }
            if state.triage_in_flight_request_id() == Some(*request_id) =>
        {
            ProgressEvent::LoadingFailed
        }
        Msg::TriageClicked => ProgressEvent::TriageRequested {
            can_start: state.triage_ai_available()
                && state.triage().can_start()
                && state.can_start_triage_from_pre_triage()
                && state.triage_metadata_ready(),
        },
        Msg::ArticlesLoaded { .. } => ProgressEvent::SummariesLoaded {
            was_loading: matches!(
                state.briefing().phase(),
                crate::BriefingPhase::LoadingArticles
            ),
        },
        Msg::BatchResultsCollected { .. } | Msg::RearmDeferredBatchStages => {
            ProgressEvent::AiStateChanged
        }
        Msg::LlmCompleted {
            request_id, result, ..
        } => llm_event(state, *request_id, result),
        _ => ProgressEvent::None,
    };

    ProgressBefore {
        counts_as_work_message,
        event,
        signal_enqueued_before: state.signal_candidate().enqueued_count(),
    }
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
    state.replace_run_progress(progress);
    state.clear_run_completion_notice();
    state.mark_dirty();
}

pub(super) fn handle_pipeline_requested(state: &mut AppState) {
    if !matches!(state.pipeline_run_phase(), PipelineRunPhase::Idle) {
        return;
    }
    begin_run_if_needed(state);
    state.set_pipeline_run_phase(PipelineRunPhase::Requested);
    state.mark_dirty();
}

pub(super) fn handle_pipeline_advance(state: &mut AppState) -> Vec<Effect> {
    match state.pipeline_run_phase() {
        PipelineRunPhase::Idle => Vec::new(),
        PipelineRunPhase::Stopping => {
            state.set_pipeline_run_phase(PipelineRunPhase::Idle);
            state.mark_dirty();
            Vec::new()
        }
        PipelineRunPhase::Requested => dispatch_or_await(state),
        PipelineRunPhase::Dispatched { since_seq } => {
            if state.reduced_message_seq() > since_seq {
                state.set_pipeline_run_phase(PipelineRunPhase::AwaitingSettle);
                state.mark_dirty();
            }
            Vec::new()
        }
        PipelineRunPhase::AwaitingSettle => {
            if pipeline_run_is_settled(state) {
                settle_run(state);
            } else if !matches!(state.batch_next_action(), BatchNextAction::None) {
                return dispatch_or_await(state);
            }
            Vec::new()
        }
    }
}

fn dispatch_or_await(state: &mut AppState) -> Vec<Effect> {
    let action = state.batch_next_action();
    let effects = match action {
        BatchNextAction::DispatchTriage => {
            let effects = super::triage::handle_triage_clicked(state);
            if !state.can_start_triage_from_pre_triage() {
                record_triage(state, true);
                record_signal_scoring(state);
            }
            effects
        }
        BatchNextAction::DispatchSummaries => {
            super::briefing::handle_prepare_summaries_clicked(state)
        }
        BatchNextAction::None => {
            state.set_pipeline_run_phase(PipelineRunPhase::AwaitingSettle);
            state.mark_dirty();
            return Vec::new();
        }
    };
    state.set_pipeline_run_phase(PipelineRunPhase::Dispatched {
        since_seq: state.reduced_message_seq(),
    });
    state.mark_dirty();
    effects
}

fn settle_run(state: &mut AppState) {
    let now = state.last_observed_utc();
    let completed_at_utc = now.unwrap_or_default();
    let completed_count = state.signal_candidate().completed_count() as usize;
    let Some(run) = state.run_progress_mut() else {
        return;
    };
    let new_result_count = completed_count.saturating_sub(run.signal_completed_at_reset);
    run.stop(now);
    state.set_run_completion_notice(RunCompletionNotice {
        new_result_count,
        completed_at_utc,
    });
    state.set_pipeline_run_phase(PipelineRunPhase::Idle);
    state.mark_dirty();
}

pub(super) fn handle_stop_for_pipeline(state: &mut AppState) {
    if !matches!(state.pipeline_run_phase(), PipelineRunPhase::Idle) {
        state.set_pipeline_run_phase(PipelineRunPhase::Stopping);
    }
    let now = state.last_observed_utc();
    if let Some(run) = state.run_progress_mut() {
        run.stop(now);
    }
    state.mark_dirty();
}

pub(super) fn dismiss_run_notice(state: &mut AppState) {
    if state.clear_run_completion_notice() {
        state.mark_dirty();
    }
}

pub(super) fn record_progress_after(state: &mut AppState, before: ProgressBefore) {
    if before.counts_as_work_message {
        state.note_reduced_work_message();
    }
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
        ProgressEvent::LoadingProgress { completed, total } => state
            .run_progress_mut()
            .expect("active run")
            .counts(PipelineStage::LoadingArticles, completed, 0, total, now),
        ProgressEvent::LoadingDone { total } => {
            let run = state.run_progress_mut().expect("active run");
            run.counts(PipelineStage::LoadingArticles, total, 0, total, now);
            run.finish(PipelineStage::LoadingArticles, now);
        }
        ProgressEvent::LoadingFailed => {
            let run = state.run_progress_mut().expect("active run");
            run.counts(PipelineStage::LoadingArticles, 0, 1, 1, now);
            run.finish(PipelineStage::LoadingArticles, now);
        }
        ProgressEvent::TriageRequested { can_start } => {
            if can_start && !state.can_start_triage_from_pre_triage() {
                record_triage(state, true);
            }
        }
        ProgressEvent::SummariesLoaded { was_loading } => {
            if was_loading {
                record_summaries(state, true);
            }
        }
        ProgressEvent::AiStateChanged => {
            record_triage(state, false);
            record_summaries(state, false);
        }
        ProgressEvent::LlmCompleted { stage, activity } => {
            match stage {
                Some(PipelineStage::Triaging) => record_triage(state, false),
                Some(PipelineStage::Summarizing) => record_summaries(state, false),
                Some(PipelineStage::ScoringSignals) | None => {}
                Some(_) => {}
            }
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

    if state.signal_candidate().enqueued_count() > before.signal_enqueued_before
        || stage_is_active(state, PipelineStage::ScoringSignals)
    {
        record_signal_scoring(state);
    }

    if may_settle_poll_only_run
        && matches!(state.pipeline_run_phase(), PipelineRunPhase::Idle)
        && state.pipeline_activity().is_settled()
    {
        settle_run(state);
    }
}

fn record_triage(state: &mut AppState, allow_activation: bool) {
    if !allow_activation && !stage_is_active(state, PipelineStage::Triaging) {
        return;
    }
    let (total, _, _, completed, failed) = state.triage().observation_counts();
    let terminal = matches!(
        state.triage().phase(),
        crate::TriagePhase::Complete | crate::TriagePhase::Failed { .. }
    );
    if total == 0 && !matches!(state.triage().phase(), crate::TriagePhase::Triaging) {
        return;
    }
    let now = state.last_observed_utc();
    let run = state.run_progress_mut().expect("active run");
    run.counts(
        PipelineStage::Triaging,
        completed as u32,
        failed as u32,
        total as u32,
        now,
    );
    if terminal {
        run.finish(PipelineStage::Triaging, now);
    }
}

fn record_summaries(state: &mut AppState, allow_activation: bool) {
    if !allow_activation && !stage_is_active(state, PipelineStage::Summarizing) {
        return;
    }
    let total = state.briefing().articles().len();
    let completed = state.briefing().completed_summary_count();
    let failed = state.briefing().failed_summary_count();
    let terminal = matches!(
        state.briefing().phase(),
        crate::BriefingPhase::Complete | crate::BriefingPhase::Failed { .. }
    );
    if total == 0 && !matches!(state.briefing().phase(), crate::BriefingPhase::Summarizing) {
        return;
    }
    let now = state.last_observed_utc();
    let run = state.run_progress_mut().expect("active run");
    run.counts(
        PipelineStage::Summarizing,
        completed as u32,
        failed as u32,
        total as u32,
        now,
    );
    if terminal {
        run.finish(PipelineStage::Summarizing, now);
    }
}

fn record_signal_scoring(state: &mut AppState) {
    let enqueued = state.signal_candidate().enqueued_count();
    let completed = state.signal_candidate().completed_count();
    let failed = state.signal_candidate().failed_count();
    let now = state.last_observed_utc();
    let run = state.run_progress_mut().expect("active run");
    let total = enqueued.saturating_sub(run.signal_enqueued_at_reset);
    if total == 0 {
        return;
    }
    run.counts(
        PipelineStage::ScoringSignals,
        completed.saturating_sub(run.signal_completed_at_reset as u32),
        failed.saturating_sub(run.signal_failed_at_reset),
        total,
        now,
    );
    // Triage, summaries, and deferred batch rearming can enqueue signal work in
    // waves. Only settle_run terminalizes this stage, after the run driver has
    // applied its full completion rule; until then counts continue accumulating.
}

fn pipeline_run_is_settled(state: &AppState) -> bool {
    matches!(state.batch_next_action(), BatchNextAction::None)
        && state.pipeline_activity().is_settled()
}

fn stage_is_active(state: &AppState, stage: PipelineStage) -> bool {
    state
        .run_progress()
        .is_some_and(|run| matches!(run.stages[stage.index()].status, StageStatus::Active))
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
