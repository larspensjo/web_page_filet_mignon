use super::*;
use crate::briefing::LoadedArticle;
use crate::{ActivityEntry, BatchStatus, Stage, ACTIVITY_FEED_CAPACITY};
use chrono::{DateTime, Utc};
use harvester_engine::llm::prompt::PromptId;
use harvester_engine::{SourceId, SourceKind};
use std::collections::HashMap;

const BASE_TIME: i64 = 1_700_000_000;

fn tick(state: AppState, second: i64) -> AppState {
    crate::update(
        state,
        Msg::Tick {
            now: DateTime::from_timestamp(BASE_TIME + second, 0)
                .expect("valid test timestamp")
                .with_timezone(&Utc),
        },
    )
    .0
}

fn request_id(effects: &[Effect], prompt_id: PromptId) -> Option<u64> {
    effects.iter().find_map(|effect| match effect {
        Effect::RequestLlmCompletion {
            request_id,
            prompt_id: actual,
            ..
        } if *actual == prompt_id => Some(*request_id),
        _ => None,
    })
}

fn loaded_article(index: usize) -> LoadedArticle {
    LoadedArticle {
        url: format!("https://progress.invalid/article-{index}"),
        source_title: Some(format!("Progress article {index}")),
        prepared_text: std::iter::repeat_n(format!("article-{index}-content"), 220)
            .collect::<Vec<_>>()
            .join(" "),
        content_hash: format!("progress-hash-{index}"),
        fetched_utc: Some("2026-09-07T12:00:00Z".into()),
    }
}

fn add_metadata(state: AppState) -> AppState {
    let active_versions = HashMap::from([
        (PromptId::ArticleTriage, 1),
        (PromptId::ArticleSummary, 1),
        (PromptId::ArticleSignalCandidate, 1),
        (PromptId::BriefingExecutiveSummary, 1),
        (PromptId::BriefingNextItem, 1),
    ]);
    let effective_models = HashMap::from([
        (PromptId::ArticleTriage, "test-triage-model".to_string()),
        (PromptId::ArticleSummary, "test-summary-model".to_string()),
        (
            PromptId::ArticleSignalCandidate,
            "test-signal-model".to_string(),
        ),
        (
            PromptId::BriefingExecutiveSummary,
            "test-briefing-model".to_string(),
        ),
        (
            PromptId::BriefingNextItem,
            "test-briefing-model".to_string(),
        ),
    ]);
    let (state, _) = crate::update(
        state,
        Msg::LlmMetadataLoaded {
            active_versions,
            effective_models,
            templates: HashMap::new(),
        },
    );
    let (state, _) = crate::update(state, Msg::PromptTemplateFilesLoaded);
    crate::update(
        state,
        Msg::PromptContextsLoaded {
            contexts: HashMap::new(),
        },
    )
    .0
}

fn triage_success(request_id: u64) -> Msg {
    Msg::LlmCompleted {
        request_id,
        result: LlmResultKind::Success {
            output_json: r#"{"category":"news","priority":3,"tags":["tag"],"rationale":"ok"}"#
                .into(),
            input_tokens: 10,
            output_tokens: 5,
            prompt_version: 1,
            resolved_model: "test-triage-model".into(),
        },
        metadata: None,
    }
}

fn summary_result(request_id: u64, succeeds: bool) -> Msg {
    let result = if succeeds {
        LlmResultKind::Success {
            output_json: format!(
                "{{\"title\":\"Summary {request_id}\",\"summary\":\"Summary\",\"key_points\":[\"point\"]}}"
            ),
            input_tokens: 10,
            output_tokens: 5,
            prompt_version: 1,
            resolved_model: "test-summary-model".into(),
        }
    } else {
        LlmResultKind::Failed {
            reason: format!("summary failure {request_id}"),
        }
    };
    Msg::LlmCompleted {
        request_id,
        result,
        metadata: None,
    }
}

fn signal_success(request_id: u64) -> Msg {
    Msg::LlmCompleted {
        request_id,
        result: LlmResultKind::Success {
            output_json: r#"{
                "signal_score": 84,
                "signal_key": "example-signal-key",
                "themes": ["ai-infrastructure"],
                "draft_gist": "Example outlet reports a concrete AI infrastructure event.",
                "source_tier": "Tier1",
                "confidence": "High",
                "reasoning": "Concrete event."
            }"#
            .into(),
            input_tokens: 10,
            output_tokens: 5,
            prompt_version: 1,
            resolved_model: "test-signal-model".into(),
        },
        metadata: None,
    }
}

fn prepare_pipeline(
    download_successes: usize,
    download_failures: usize,
) -> (AppState, Vec<LoadedArticle>) {
    let total = download_successes + download_failures;
    let state = tick(AppState::new(), 0);
    let (state, effects) = crate::update(state, Msg::PollSourcesClicked);
    assert_eq!(effects, vec![Effect::PollAllSources]);
    let (state, _) = crate::update(state, Msg::PollStarted { total: 1 });
    let urls = (0..total)
        .map(|index| format!("https://progress.invalid/article-{index}"))
        .collect::<Vec<_>>();
    let (mut state, effects) = crate::update(
        state,
        Msg::SourcePollCompleted {
            source_id: SourceId::new("progress-source").expect("valid source id"),
            urls,
            kind: SourceKind::Rss,
            parsed: total,
            dedup_filtered: 0,
        },
    );
    let job_ids = effects
        .iter()
        .filter_map(|effect| match effect {
            Effect::EnqueueUrl { job_id, .. } => Some(*job_id),
            _ => None,
        })
        .collect::<Vec<_>>();
    assert_eq!(job_ids.len(), total);
    state = crate::update(state, Msg::PipelineRunRequested).0;
    state = crate::update(state, Msg::AllSourcesPollEnded).0;

    for (index, job_id) in job_ids.into_iter().enumerate() {
        let progress_repetitions = if index == 0 { 8 } else { 1 };
        for _ in 0..progress_repetitions {
            state = crate::update(
                state,
                Msg::JobProgress {
                    job_id,
                    stage: Stage::Downloading,
                    tokens: None,
                    bytes: Some(1_024),
                    content_preview: None,
                },
            )
            .0;
        }
        let succeeds = index < download_successes;
        state = crate::update(
            state,
            Msg::JobDone {
                job_id,
                result: if succeeds {
                    JobResultKind::Success
                } else {
                    JobResultKind::Failed {
                        reason: format!("download failure {index}"),
                    }
                },
                content_preview: None,
                extracted_links: Vec::new(),
                fetched_utc: Some("2026-09-07T12:00:00Z".into()),
            },
        )
        .0;
    }

    let articles = (0..download_successes)
        .map(loaded_article)
        .collect::<Vec<_>>();
    let ordered_urls = articles.iter().map(|article| article.url.clone()).collect();
    state = crate::update(
        state,
        Msg::EvaluatePreTriageRefresh {
            ordered_urls,
            triggered_by_job_done: true,
        },
    )
    .0;
    let mut load_request_id = None;
    for second in 1..=20 {
        let (next, effects) = crate::update(
            state,
            Msg::Tick {
                now: DateTime::from_timestamp(BASE_TIME + second, 0).expect("valid test timestamp"),
            },
        );
        state = next;
        load_request_id = effects.iter().find_map(|effect| match effect {
            Effect::LoadArticlesForTriage { request_id, .. } => Some(*request_id),
            _ => None,
        });
        if load_request_id.is_some() {
            break;
        }
    }
    let load_request_id = load_request_id.expect("pre-triage load dispatched");
    state = crate::update(
        state,
        Msg::TriageArticlesLoadProgress {
            request_id: load_request_id,
            files_scanned: articles.len().saturating_sub(1),
            files_total: articles.len(),
        },
    )
    .0;
    state = crate::update(
        state,
        Msg::TriageArticlesLoaded {
            request_id: load_request_id,
            articles: articles.clone(),
        },
    )
    .0;
    (add_metadata(state), articles)
}

fn dispatch_triage(mut state: AppState) -> (AppState, Vec<Effect>) {
    state = crate::update(state, Msg::PipelineRunRequested).0;
    crate::update(state, Msg::PipelineRunAdvance)
}

fn complete_triage(mut state: AppState, mut effects: Vec<Effect>) -> (AppState, Vec<Effect>) {
    loop {
        let Some(id) = request_id(&effects, PromptId::ArticleTriage) else {
            return (state, effects);
        };
        (state, effects) = crate::update(state, triage_success(id));
    }
}

fn dispatch_summaries(state: AppState) -> (AppState, Vec<Effect>) {
    let (state, effects) = crate::update(state, Msg::PipelineRunAdvance);
    assert!(effects.is_empty());
    assert!(matches!(
        state.pipeline_run_phase(),
        PipelineRunPhase::AwaitingSettle
    ));
    crate::update(state, Msg::PipelineRunAdvance)
}

fn complete_summaries(
    mut state: AppState,
    mut effects: Vec<Effect>,
    outcomes: &[bool],
) -> (AppState, Vec<u64>) {
    let mut outcome_index = 0;
    let mut signal_ids = Vec::new();
    loop {
        signal_ids.extend(effects.iter().filter_map(|effect| match effect {
            Effect::RequestLlmCompletion {
                request_id,
                prompt_id: PromptId::ArticleSignalCandidate,
                ..
            } => Some(*request_id),
            _ => None,
        }));
        let Some(id) = request_id(&effects, PromptId::ArticleSummary) else {
            return (state, signal_ids);
        };
        let succeeds = outcomes[outcome_index];
        outcome_index += 1;
        (state, effects) = crate::update(state, summary_result(id, succeeds));
    }
}

fn run_to_completion(
    state: AppState,
    articles: &[LoadedArticle],
    summary_outcomes: &[bool],
) -> AppState {
    let mut previous = progress_snapshot(&state);
    let (state, triage_effects) = dispatch_triage(state);
    assert_progress_does_not_regress(&mut previous, &state);
    let (state, _) = crate::update(state, Msg::PipelineRunAdvance);
    let (state, _) = crate::update(state, Msg::PipelineRunAdvance);
    assert_progress_does_not_regress(&mut previous, &state);
    assert!(matches!(
        state.pipeline_run_phase(),
        PipelineRunPhase::Dispatched { .. }
    ));
    let (state, _) = complete_triage(state, triage_effects);
    assert_progress_does_not_regress(&mut previous, &state);
    let (state, summary_load_effects) = dispatch_summaries(state);
    assert_progress_does_not_regress(&mut previous, &state);
    assert!(summary_load_effects
        .iter()
        .any(|effect| matches!(effect, Effect::LoadArticlesForBriefing { .. })));
    let state = add_metadata(state);
    let (state, summary_effects) = crate::update(
        state,
        Msg::ArticlesLoaded {
            articles: articles.to_vec(),
            collection_text: "progress collection".into(),
        },
    );
    assert_progress_does_not_regress(&mut previous, &state);
    let (state, _) = crate::update(state, Msg::PipelineRunAdvance);
    assert!(matches!(
        state.pipeline_run_phase(),
        PipelineRunPhase::AwaitingSettle
    ));
    let (mut state, signal_ids) = complete_summaries(state, summary_effects, summary_outcomes);
    assert_progress_does_not_regress(&mut previous, &state);
    for signal_id in signal_ids {
        state = crate::update(state, signal_success(signal_id)).0;
        assert_progress_does_not_regress(&mut previous, &state);
    }
    let state = crate::update(state, Msg::PipelineRunAdvance).0;
    assert_progress_does_not_regress(&mut previous, &state);
    state
}

type ProgressSnapshot = [(u8, u32, u32, u32); 6];

fn progress_snapshot(state: &AppState) -> ProgressSnapshot {
    let progress = state.run_progress().expect("run progress");
    std::array::from_fn(|index| {
        let stage = &progress.stages[index];
        let rank = match stage.status {
            StageStatus::Pending => 0,
            StageStatus::Active => 1,
            StageStatus::Done | StageStatus::Failed => 2,
        };
        (rank, stage.completed, stage.failed, stage.total)
    })
}

fn assert_progress_does_not_regress(previous: &mut ProgressSnapshot, state: &AppState) {
    let current = progress_snapshot(state);
    for (prior, next) in previous.iter().zip(current.iter()) {
        assert!(
            next.0 >= prior.0,
            "stage status regressed: {prior:?} -> {next:?}"
        );
        assert!(
            next.1 >= prior.1,
            "completed count regressed: {prior:?} -> {next:?}"
        );
        assert!(
            next.2 >= prior.2,
            "failed count regressed: {prior:?} -> {next:?}"
        );
        assert!(
            next.3 >= prior.3,
            "total count regressed: {prior:?} -> {next:?}"
        );
    }
    *previous = current;
}

#[test]
fn full_run_progress_walk_preserves_every_stage_and_counts() {
    let (state, articles) = prepare_pipeline(1, 0);
    let state = run_to_completion(state, &articles, &[true]);
    let progress = state.run_progress().expect("run progress");
    assert!(progress.terminal);
    assert_eq!(progress.stages.len(), PipelineStage::ALL.len());
    assert!(
        progress
            .stages
            .iter()
            .all(|stage| stage.status == StageStatus::Done),
        "stages: {:?}; activity: {:?}",
        progress.stages,
        progress.activity
    );
    assert_eq!(
        progress.stages[PipelineStage::ScanningSources.index()].completed,
        1
    );
    assert_eq!(
        progress.stages[PipelineStage::DownloadingArticles.index()].completed,
        1
    );
    assert_eq!(
        progress.stages[PipelineStage::LoadingArticles.index()].completed,
        1
    );
    assert_eq!(
        progress.stages[PipelineStage::Triaging.index()].completed,
        1
    );
    assert_eq!(
        progress.stages[PipelineStage::Summarizing.index()].completed,
        1
    );
    assert_eq!(
        progress.stages[PipelineStage::ScoringSignals.index()].completed,
        1
    );
    assert!(progress
        .activity
        .iter()
        .all(|entry| !entry.url.starts_with("request:")));
    assert!(progress.activity.iter().any(|entry| entry.title.is_some()));
    assert_eq!(
        progress
            .activity
            .iter()
            .filter(|entry| {
                entry.stage == PipelineStage::DownloadingArticles
                    && matches!(entry.outcome, ActivityOutcome::Started)
            })
            .count(),
        1,
        "streaming progress must produce one per-item Started row"
    );
    assert_eq!(
        progress
            .activity
            .iter()
            .filter(|entry| {
                entry.stage == PipelineStage::DownloadingArticles
                    && matches!(
                        entry.outcome,
                        ActivityOutcome::Succeeded | ActivityOutcome::Failed { .. }
                    )
            })
            .count(),
        1,
        "each download must produce one terminal row"
    );
}

#[test]
fn partial_failures_finish_done_and_retain_failure_counts() {
    let (state, articles) = prepare_pipeline(3, 3);
    let state = run_to_completion(state, &articles, &[false, false, true]);
    let progress = state.run_progress().expect("run progress");
    let downloads = &progress.stages[PipelineStage::DownloadingArticles.index()];
    let summaries = &progress.stages[PipelineStage::Summarizing.index()];
    assert_eq!((downloads.status, downloads.failed), (StageStatus::Done, 3));
    assert_eq!((summaries.status, summaries.failed), (StageStatus::Done, 2));
}

#[test]
fn total_source_failure_marks_scanning_failed() {
    let state = tick(AppState::new(), 0);
    let state = crate::update(state, Msg::PollSourcesClicked).0;
    let state = crate::update(state, Msg::PollStarted { total: 2 }).0;
    let source_id = SourceId::new("failed-source").expect("valid source id");
    let state = crate::update(
        state,
        Msg::SourcePollFailed {
            source_id: source_id.clone(),
            error: "first failure".into(),
        },
    )
    .0;
    let state = crate::update(
        state,
        Msg::SourcePollFailed {
            source_id,
            error: "second failure".into(),
        },
    )
    .0;
    let state = crate::update(state, Msg::AllSourcesPollEnded).0;
    let scan = &state.run_progress().expect("run").stages[PipelineStage::ScanningSources.index()];
    assert_eq!(
        (scan.status, scan.completed, scan.failed),
        (StageStatus::Failed, 0, 2)
    );
}

#[test]
fn poll_only_run_becomes_terminal_when_source_poll_settles() {
    let state = tick(AppState::new(), 0);
    let (state, effects) = crate::update(state, Msg::PollSourcesClicked);
    assert_eq!(effects, vec![Effect::PollAllSources]);
    let state = crate::update(state, Msg::PollStarted { total: 1 }).0;
    let state = crate::update(
        state,
        Msg::SourcePollCompleted {
            source_id: SourceId::new("poll-only-source").expect("valid source id"),
            urls: Vec::new(),
            kind: SourceKind::Rss,
            parsed: 0,
            dedup_filtered: 0,
        },
    )
    .0;
    let state = crate::update(state, Msg::AllSourcesPollEnded).0;

    assert!(matches!(state.pipeline_run_phase(), PipelineRunPhase::Idle));
    assert!(state.run_progress().expect("poll run").terminal);
    assert!(!state.view().run_progress.run_active);
}

#[test]
fn accepted_stop_freezes_poll_run_while_pipeline_driver_is_idle() {
    let state = tick(AppState::new(), 0);
    let state = crate::update(state, Msg::PollSourcesClicked).0;
    let state = crate::update(state, Msg::PollStarted { total: 1 }).0;
    let (state, effects) = crate::update(
        state,
        Msg::SourcePollCompleted {
            source_id: SourceId::new("stoppable-poll-source").expect("valid source id"),
            urls: vec!["https://progress.invalid/stoppable".into()],
            kind: SourceKind::Rss,
            parsed: 1,
            dedup_filtered: 0,
        },
    );
    assert!(effects
        .iter()
        .any(|effect| matches!(effect, Effect::EnqueueUrl { .. })));
    assert!(matches!(state.pipeline_run_phase(), PipelineRunPhase::Idle));
    assert!(state.stop_finish_button_state().is_enabled());

    let (state, effects) = crate::update(state, Msg::StopFinishClicked);

    assert!(effects
        .iter()
        .any(|effect| matches!(effect, Effect::StopFinish { .. })));
    let progress = state.run_progress().expect("stopped poll run");
    assert!(progress.terminal);
    assert!(progress
        .stages
        .iter()
        .all(|stage| stage.status != StageStatus::Active));
    assert!(!state.view().run_progress.run_active);
    assert!(matches!(state.pipeline_run_phase(), PipelineRunPhase::Idle));
}

#[test]
fn skipped_download_stage_stays_pending() {
    let snapshots = vec![crate::CompletedJobSnapshot {
        url: "https://progress.invalid/restored".into(),
        tokens: Some(100),
        bytes: Some(1_000),
        links: Vec::new(),
        fetched_utc: None,
    }];
    let state = crate::update(AppState::new(), Msg::RestoreCompletedJobs(snapshots)).0;
    let state = crate::update(state, Msg::PipelineRunRequested).0;
    let stage =
        &state.run_progress().expect("run").stages[PipelineStage::DownloadingArticles.index()];
    assert_eq!((stage.status, stage.total), (StageStatus::Pending, 0));
}

#[test]
fn accepted_stop_during_triage_leaves_skipped_stages_pending_and_never_dispatches_summaries() {
    let (state, _) = prepare_pipeline(1, 0);
    let (state, _) = dispatch_triage(state);
    assert_eq!(
        state.run_progress().expect("run").stages[PipelineStage::Triaging.index()].status,
        StageStatus::Active
    );
    let frozen_counts =
        state.run_progress().expect("run").stages[PipelineStage::Triaging.index()].clone();
    let (state, effects) = crate::update(state, Msg::StopFinishClicked);
    assert!(effects
        .iter()
        .any(|effect| matches!(effect, Effect::StopFinish { .. })));
    let progress = state.run_progress().expect("run");
    assert!(progress
        .stages
        .iter()
        .all(|stage| stage.status != StageStatus::Active));
    assert_eq!(
        progress.stages[PipelineStage::Summarizing.index()].status,
        StageStatus::Pending
    );
    assert_eq!(
        progress.stages[PipelineStage::ScoringSignals.index()].status,
        StageStatus::Pending
    );
    assert_eq!(
        progress.stages[PipelineStage::Triaging.index()].completed,
        frozen_counts.completed
    );
    assert_eq!(
        progress.stages[PipelineStage::Triaging.index()].failed,
        frozen_counts.failed
    );
    let (state, effects) = crate::update(state, Msg::PipelineRunAdvance);
    assert!(effects.is_empty());
    assert!(matches!(state.pipeline_run_phase(), PipelineRunPhase::Idle));
}

#[test]
fn disabled_stop_intent_does_not_freeze_the_run() {
    let state = tick(AppState::new(), 0);
    let state = crate::update(state, Msg::PipelineRunRequested).0;
    let (state, effects) = crate::update(state, Msg::StopFinishClicked);
    assert!(effects.is_empty());
    assert!(matches!(
        state.pipeline_run_phase(),
        PipelineRunPhase::Requested
    ));
    assert!(!state.run_progress().expect("run").terminal);
}

#[test]
fn triage_loading_is_never_reported_as_settled() {
    let mut state = AppState::new();
    state.set_triage(crate::triage::TriageSession::new_loading(None));
    assert!(!state.pipeline_activity().is_settled());
    assert_eq!(state.batch_status(), BatchStatus::Running);
}

#[test]
fn rerun_resets_six_pending_stages_with_a_new_run_id_and_ignores_stale_sessions() {
    let (state, articles) = prepare_pipeline(1, 0);
    let state = run_to_completion(state, &articles, &[true]);
    let old_run_id = state.run_progress().expect("first run").run_id;
    let state = crate::update(state, Msg::PipelineRunRequested).0;
    let new_run_id = state.run_progress().expect("second run").run_id;
    assert_ne!(new_run_id, old_run_id);
    let state = tick(state, 50);
    let progress = state.run_progress().expect("second run");
    assert_eq!(progress.stages.len(), 6);
    assert!(progress.stages.iter().all(|stage| {
        stage.status == StageStatus::Pending
            && stage.completed == 0
            && stage.failed == 0
            && stage.total == 0
    }));
}

#[test]
fn pipeline_request_joins_an_active_poll_run_without_resetting() {
    let state = tick(AppState::new(), 0);
    let state = crate::update(state, Msg::PollSourcesClicked).0;
    let state = crate::update(state, Msg::PollStarted { total: 3 }).0;
    let run_id = state.run_progress().expect("poll run").run_id;
    let state = crate::update(state, Msg::PipelineRunRequested).0;
    assert_eq!(state.run_progress().expect("joined run").run_id, run_id);
    assert_eq!(state.run_progress().expect("joined run").stages[0].total, 3);
}

#[test]
fn driver_runs_triage_summaries_scoring_and_sets_notice_exactly_once() {
    let (state, articles) = prepare_pipeline(1, 0);
    let expected_completed_at = state.last_observed_utc().expect("observed time");
    let state = run_to_completion(state, &articles, &[true]);
    assert!(matches!(state.pipeline_run_phase(), PipelineRunPhase::Idle));
    let notice = state
        .run_completion_notice()
        .expect("completion notice")
        .clone();
    assert_eq!(
        notice.new_result_count,
        1,
        "signal={:?} progress={:?}",
        state.signal_candidate().observation_counts(),
        state.run_progress()
    );
    assert_eq!(notice.completed_at_utc, expected_completed_at);
    let state = crate::update(state, Msg::PipelineRunAdvance).0;
    assert_eq!(state.run_completion_notice(), Some(&notice));
    let state = crate::update(state, Msg::RunFinishedNoticeDismissed).0;
    assert!(state.run_completion_notice().is_none());
}

#[test]
fn advance_pulses_do_not_satisfy_the_post_dispatch_message_guard() {
    let (state, _) = prepare_pipeline(1, 0);
    let (state, effects) = dispatch_triage(state);
    assert!(request_id(&effects, PromptId::ArticleTriage).is_some());
    let since_seq = match state.pipeline_run_phase() {
        PipelineRunPhase::Dispatched { since_seq } => since_seq,
        phase => panic!("expected dispatched phase, got {phase:?}"),
    };
    let state = crate::update(state, Msg::PipelineRunAdvance).0;
    let state = crate::update(state, Msg::PipelineRunAdvance).0;
    assert_eq!(
        state.pipeline_run_phase(),
        PipelineRunPhase::Dispatched { since_seq }
    );
}

#[test]
fn second_pipeline_request_is_ignored_while_driver_is_active() {
    let (state, _) = prepare_pipeline(1, 0);
    let state = crate::update(state, Msg::PipelineRunRequested).0;
    let run_id = state.run_progress().expect("run").run_id;
    let phase = state.pipeline_run_phase();
    let state = crate::update(state, Msg::PipelineRunRequested).0;
    assert_eq!(state.pipeline_run_phase(), phase);
    assert_eq!(state.run_progress().expect("same run").run_id, run_id);
}

#[test]
fn gui_and_batch_paths_reach_the_same_terminal_pipeline_activity() {
    let (prepared, articles) = prepare_pipeline(1, 0);
    let gui = run_to_completion(prepared.clone(), &articles, &[true]);

    let (batch, triage_effects) = crate::update(prepared, Msg::TriageClicked);
    let (batch, _) = complete_triage(batch, triage_effects);
    let (batch, _) = crate::update(batch, Msg::PrepareSummariesClicked);
    let batch = add_metadata(batch);
    let (batch, summary_effects) = crate::update(
        batch,
        Msg::ArticlesLoaded {
            articles,
            collection_text: "progress collection".into(),
        },
    );
    let (mut batch, signal_ids) = complete_summaries(batch, summary_effects, &[true]);
    for signal_id in signal_ids {
        batch = crate::update(batch, signal_success(signal_id)).0;
    }

    assert_eq!(gui.pipeline_activity(), batch.pipeline_activity());
    assert!(gui.pipeline_activity().is_settled());
    assert_eq!(gui.batch_status(), BatchStatus::Settled);
    assert_eq!(batch.batch_status(), BatchStatus::Settled);
}

#[test]
fn activity_feed_is_bounded_and_reason_truncation_is_char_safe() {
    let mut progress = RunProgress::new(1, None, 0, 0, 0);
    for index in 0..=ACTIVITY_FEED_CAPACITY {
        progress.push_activity(
            index.to_string(),
            None,
            PipelineStage::Triaging,
            ActivityOutcome::Failed {
                reason: bounded_reason(&"😀".repeat(201)),
            },
        );
    }
    assert_eq!(progress.activity.len(), ACTIVITY_FEED_CAPACITY);
    assert_eq!(progress.activity.front().map(|entry| entry.seq), Some(1));
    assert!(progress.activity.iter().all(
        |ActivityEntry { outcome, .. }| matches!(outcome, ActivityOutcome::Failed { reason } if reason.chars().count() <= ACTIVITY_REASON_MAX_CHARS)
    ));
}
