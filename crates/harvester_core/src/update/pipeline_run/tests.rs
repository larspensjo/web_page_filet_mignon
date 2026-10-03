use super::*;
use crate::briefing::{ArticleSummaryResult, LoadedArticle};
use crate::{ActivityEntry, BatchStatus, Stage, SummaryCacheKey, ACTIVITY_FEED_CAPACITY};
use chrono::{DateTime, Utc};
use harvester_engine::llm::prompt::PromptId;
use harvester_engine::{SourceId, SourceKind};
use std::collections::HashMap;

const BASE_TIME: i64 = 1_700_000_000;

fn tick(state: AppState, second: i64) -> AppState {
    crate::update::test_support::update(
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

fn seed_cached_summary(state: &mut AppState, article: &LoadedArticle) {
    state.store_summary_result(
        SummaryCacheKey::try_new(
            &article.content_hash,
            PromptId::ArticleSummary,
            Some(1),
            Some("test-summary-model"),
            &[],
        )
        .expect("summary cache key"),
        ArticleSummaryResult {
            title: article.source_title.clone().expect("article title"),
            summary: "Cached summary".into(),
            key_points: vec!["cached point".into()],
            input_tokens: 10,
            output_tokens: 5,
            entities: Default::default(),
        },
        "2026-09-07T12:00:00Z".into(),
    );
}

fn add_metadata(state: AppState) -> AppState {
    let active_versions = HashMap::from([
        (PromptId::ArticleTriage, 1),
        (PromptId::ArticleSummary, 1),
        (PromptId::ArticleSignalCandidate, 1),
    ]);
    let effective_models = HashMap::from([
        (PromptId::ArticleTriage, "test-triage-model".to_string()),
        (PromptId::ArticleSummary, "test-summary-model".to_string()),
        (
            PromptId::ArticleSignalCandidate,
            "test-signal-model".to_string(),
        ),
    ]);
    let (state, _) = crate::update::test_support::update(
        state,
        Msg::LlmMetadataLoaded {
            active_versions,
            effective_models,
        },
    );
    let (state, _) = crate::update::test_support::update(state, Msg::PromptTemplateFilesLoaded);
    crate::update::test_support::update(
        state,
        Msg::PromptContextsLoaded {
            contexts: HashMap::new(),
        },
    )
    .0
}

#[test]
fn unarmed_full_run_settles_and_next_full_polls_again() {
    let (state, _) = crate::update(
        AppState::new(),
        Msg::AiAvailabilityDetected {
            availability: crate::AiAvailability::Unavailable {
                reason: crate::AiUnavailableReason::MissingApiKey,
            },
        },
    );
    let (state, effects) = crate::update(
        state,
        Msg::PipelineRunRequested {
            scope: crate::PipelineRunScope::Full,
        },
    );
    assert_eq!(effects, vec![Effect::PollAllSources]);
    assert!(!state.pipeline_run_armed());
    let (mut state, _) = crate::update(state, Msg::AllSourcesPollEnded);
    for _ in 0..20 {
        if state.run_progress().unwrap().terminal {
            break;
        }
        state = crate::update(state, Msg::PipelineRunAdvance).0;
    }
    assert_eq!(state.batch_status(), BatchStatus::Settled);
    assert!(state.run_progress().unwrap().terminal);
    let (state, effects) = crate::update(
        state,
        Msg::PipelineRunRequested {
            scope: crate::PipelineRunScope::Full,
        },
    );
    assert_eq!(effects, vec![Effect::PollAllSources]);
    assert!(!state.run_progress().unwrap().terminal);
}

#[test]
fn excluded_initial_window_does_not_disarm_later_waves() {
    let state = add_metadata(AppState::new());
    let mut excluded = loaded_article(100);
    excluded.prepared_text = "short".into();
    let (state, _) = crate::update(
        state,
        Msg::RestoreCompletedJobs(vec![crate::CompletedJobSnapshot {
            url: excluded.url.clone(),
            tokens: Some(1),
            bytes: Some(5),
            links: Vec::new(),
            fetched_utc: excluded.fetched_utc.clone(),
        }]),
    );
    let (state, effects) = crate::update(
        state,
        Msg::PipelineRunRequested {
            scope: crate::PipelineRunScope::Full,
        },
    );
    let (state, effects) =
        crate::fixture_support::complete_processing_configuration(state, effects, 100_000);
    let first_load_id = effects
        .iter()
        .find_map(|effect| match effect {
            Effect::LoadArticlesForTriage { request_id, .. } => Some(*request_id),
            _ => None,
        })
        .unwrap();
    let (mut state, _) = crate::update(
        state,
        Msg::TriageArticlesLoaded {
            request_id: first_load_id,
            delta: harvester_engine::TriageArticleDelta::full_window(vec![excluded], 100_000),
        },
    );
    assert!(state.pipeline_run_armed());
    assert!(state.processing_start.is_none());

    let valid = loaded_article(101);
    let load_id = state.pre_triage_coordinator.begin_preparation_load();
    state.set_triage_in_flight(load_id);
    let (state, effects) = crate::update(
        state,
        Msg::TriageArticlesLoaded {
            request_id: load_id,
            delta: harvester_engine::TriageArticleDelta::full_window(vec![valid], 100_000),
        },
    );
    assert!(state.pipeline_run_armed());
    assert!(request_id(&effects, PromptId::ArticleTriage).is_some());
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
    let state = tick(add_metadata(AppState::new()), 0);
    let (state, effects) = crate::update(
        state,
        Msg::PipelineRunRequested {
            scope: crate::PipelineRunScope::Full,
        },
    );
    assert!(effects.contains(&Effect::PollAllSources));
    let (state, _) =
        crate::fixture_support::complete_processing_configuration(state, effects, 100_000);
    let (state, _) = crate::update::test_support::update(state, Msg::PollStarted { total: 1 });
    let urls = (0..total)
        .map(|index| format!("https://progress.invalid/article-{index}"))
        .collect::<Vec<_>>();
    let (mut state, effects) = crate::update::test_support::update(
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
    let (next, _) = crate::update::test_support::update(
        state,
        Msg::PipelineRunRequested {
            scope: crate::PipelineRunScope::Full,
        },
    );
    state = next;
    state = crate::update::test_support::update(state, Msg::AllSourcesPollEnded).0;

    for (index, job_id) in job_ids.into_iter().enumerate() {
        let progress_repetitions = if index == 0 { 8 } else { 1 };
        for _ in 0..progress_repetitions {
            state = crate::update::test_support::update(
                state,
                Msg::JobProgress {
                    job_id,
                    stage: Stage::Downloading,
                    tokens: None,
                    bytes: Some(1_024),
                },
            )
            .0;
        }
        let succeeds = index < download_successes;
        state = crate::update::test_support::update(
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

                extracted_links: Vec::new(),
                fetched_utc: Some("2026-09-07T12:00:00Z".into()),
            },
        )
        .0;
    }
    let articles = (0..download_successes)
        .map(loaded_article)
        .collect::<Vec<_>>();
    let (next, _) = crate::update::test_support::update(
        state,
        Msg::EvaluatePreTriageRefresh {
            ordered_urls: articles.iter().map(|article| article.url.clone()).collect(),
            triggered_by_job_done: true,
        },
    );
    state = next;
    let (next, effects) = crate::update::test_support::update(state, Msg::PipelineRunAdvance);
    state = next;
    let load_request_id = effects
        .iter()
        .find_map(|effect| match effect {
            Effect::LoadArticlesForTriage { request_id, .. } => Some(*request_id),
            _ => None,
        })
        .expect("settled intake releases the fresh run window");
    state = crate::update::test_support::update(
        state,
        Msg::TriageArticlesLoadProgress {
            request_id: load_request_id,
            files_scanned: articles.len().saturating_sub(1),
            files_total: articles.len(),
        },
    )
    .0;
    state = crate::update::test_support::update(
        state,
        Msg::TriageArticlesLoaded {
            request_id: load_request_id,
            delta: harvester_engine::TriageArticleDelta::full_window(articles.clone(), 100_000),
        },
    )
    .0;
    assert!(state.triage().in_progress_count() > 0);
    (state, articles)
}

fn run_to_completion(
    mut state: AppState,
    _articles: &[LoadedArticle],
    summary_outcomes: &[bool],
) -> AppState {
    let mut previous = progress_snapshot(&state);
    let mut summary_index = 0;
    for _ in 0..1000 {
        let msg = if let Some(id) = scoring_request(&state) {
            signal_success(id)
        } else if let Some(id) = summary_request(&state) {
            let succeeds = summary_outcomes[summary_index];
            summary_index += 1;
            summary_result(id, succeeds)
        } else if let Some(id) = triage_request(&state) {
            triage_success(id)
        } else {
            break;
        };
        let (next, effects) = crate::update(state, msg);
        state = next;
        assert!(effects
            .iter()
            .all(|e| !matches!(e, Effect::LoadProcessingConfiguration { .. })));
        assert_progress_does_not_regress(&mut previous, &state);
    }
    assert!(
        state.pipeline_activity().is_settled(),
        "activity={:?}",
        state.pipeline_activity()
    );
    assert!(state.run_progress().unwrap().terminal);
    state
}

fn triage_request(state: &AppState) -> Option<u64> {
    state.triage().articles().iter().find_map(|a| {
        if let crate::triage::ArticleTriageState::InProgress { request_id } = a.triage_state {
            Some(request_id)
        } else {
            None
        }
    })
}
fn summary_request(state: &AppState) -> Option<u64> {
    state.briefing().articles().iter().find_map(|a| {
        if let crate::briefing::ArticleSummaryState::InProgress { request_id } = a.summary_state {
            Some(request_id)
        } else {
            None
        }
    })
}
fn scoring_request(state: &AppState) -> Option<u64> {
    state.signal_candidate().iter_states().find_map(|(_, a)| {
        if let crate::SignalCandidateState::Scoring { request_id } = a {
            Some(*request_id)
        } else {
            None
        }
    })
}

type ProgressSnapshot = [(u8, u32, u32, u32, u32); 6];

fn progress_snapshot(state: &AppState) -> ProgressSnapshot {
    let progress = state.run_progress().expect("run progress");
    std::array::from_fn(|index| {
        let stage = &progress.stages[index];
        let rank = match stage.status {
            StageStatus::Pending => 0,
            StageStatus::Active => 1,
            StageStatus::Done | StageStatus::Failed => 2,
        };
        (
            rank,
            stage.completed,
            stage.failed,
            stage.total,
            stage.reused,
        )
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
    for (index, (prior, next)) in previous.iter().zip(current.iter()).enumerate() {
        assert!(next.4 <= next.1, "reused exceeds completed: {next:?}");
        assert!(next.4 >= prior.4, "reuse regressed: {prior:?} -> {next:?}");
        assert!(
            next.3 - next.4 >= prior.3 - prior.4,
            "new work regressed: {prior:?} -> {next:?}"
        );
        if index < 3 {
            assert_eq!(next.4, 0);
        }
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
fn load_progress_counts_admission_instead_of_directory_scans() {
    let state = add_metadata(AppState::new());
    let (state, effects) = crate::update(
        state,
        Msg::PipelineRunRequested {
            scope: crate::PipelineRunScope::Resume,
        },
    );
    let (state, effects) =
        crate::fixture_support::complete_processing_configuration(state, effects, 100_000);
    let id = effects
        .iter()
        .find_map(|e| {
            if let Effect::LoadArticlesForTriage { request_id, .. } = e {
                Some(*request_id)
            } else {
                None
            }
        })
        .unwrap();
    let (state, _) = crate::update(
        state,
        Msg::TriageArticlesLoadProgress {
            request_id: id,
            files_scanned: 2,
            files_total: 4,
        },
    );
    let loading = &state.run_progress().unwrap().stages[PipelineStage::LoadingArticles.index()];
    assert_eq!(
        (
            loading.status,
            loading.completed,
            loading.total,
            loading.total_is_final
        ),
        (StageStatus::Active, 0, 0, false)
    );
    assert!(!state.pipeline_activity().is_settled());
}

#[test]
fn scoring_stays_active_from_triage_cache_hit_through_the_last_summary_wave() {
    let (mut state, articles) = prepare_pipeline(2, 0);
    seed_cached_summary(&mut state, &articles[0]);
    let id = triage_request(&state).unwrap();
    let (state, effects) = crate::update(state, triage_success(id));
    let score = request_id(&effects, PromptId::ArticleSignalCandidate).unwrap();
    let (state, effects) = crate::update(state, signal_success(score));
    let triage = request_id(&effects, PromptId::ArticleTriage).unwrap();
    assert_eq!(
        state.run_progress().unwrap().stages[PipelineStage::ScoringSignals.index()].status,
        StageStatus::Active
    );
    let (state, effects) = crate::update(state, triage_success(triage));
    let summary = request_id(&effects, PromptId::ArticleSummary).unwrap();
    assert_eq!(
        state.run_progress().unwrap().stages[PipelineStage::ScoringSignals.index()].status,
        StageStatus::Active
    );
    let (state, effects) = crate::update(state, summary_result(summary, true));
    let score = request_id(&effects, PromptId::ArticleSignalCandidate).unwrap();
    let (state, _) = crate::update(state, signal_success(score));
    let scoring = &state.run_progress().unwrap().stages[PipelineStage::ScoringSignals.index()];
    assert_eq!(
        (scoring.status, scoring.completed, scoring.total),
        (StageStatus::Done, 2, 2)
    );
    assert!(state.run_progress().unwrap().terminal);
}

#[test]
fn accepted_stop_drains_an_in_flight_score_before_terminalizing_the_run() {
    let (state, _) = prepare_pipeline(1, 0);
    let id = triage_request(&state).unwrap();
    let (state, effects) = crate::update(state, triage_success(id));
    let id = request_id(&effects, PromptId::ArticleSummary).unwrap();
    let (state, effects) = crate::update(state, summary_result(id, true));
    let score_id = request_id(&effects, PromptId::ArticleSignalCandidate).unwrap();
    let (state, effects) = crate::update(state, Msg::StopFinishClicked);
    assert!(effects
        .iter()
        .any(|e| matches!(e, Effect::StopFinish { .. })));
    assert!(!state.run_progress().unwrap().terminal);
    assert_eq!(state.pipeline_run_phase(), PipelineRunPhase::Stopping);
    assert_eq!(
        state.run_state(),
        crate::RunState::Stopping { in_flight: 1 }
    );
    assert_eq!(
        state.run_progress().unwrap().stages[PipelineStage::ScoringSignals.index()].status,
        StageStatus::Active
    );
    let (state, effects) = crate::update(state, signal_success(score_id));
    assert!(effects
        .iter()
        .all(|e| !matches!(e, Effect::RequestLlmCompletion { .. })));
    assert!(state.run_progress().unwrap().terminal);
    assert_eq!(state.pipeline_run_phase(), PipelineRunPhase::Idle);
    assert_eq!(
        state.batch_observation().session_state,
        crate::SessionState::Idle
    );
    assert_eq!(state.run_state(), crate::RunState::Idle);
    assert!(state
        .run_progress()
        .unwrap()
        .stages
        .iter()
        .all(|s| s.status != StageStatus::Active));
    assert_eq!(
        state.run_progress().unwrap().stages[PipelineStage::ScoringSignals.index()].status,
        StageStatus::Done
    );
}

#[test]
fn desktop_run_actions_follow_lifecycle_unfinished_work_and_ai_availability() {
    let idle = AppState::new().view();
    assert!(idle.run_enabled);
    assert!(!idle.resume_enabled);
    assert_eq!(idle.unfinished_work, crate::UnfinishedWork::Unknown);

    let (state, _articles) = prepare_pipeline(2, 0);
    assert!(!state.view().run_enabled);
    assert!(!state.view().resume_enabled);

    let request_id = triage_request(&state).expect("one in-flight triage request");
    let (state, _) = crate::update(state, Msg::StopFinishClicked);
    assert!(!state.view().run_enabled);
    assert!(!state.view().resume_enabled);
    assert!(matches!(
        state.view().run_state,
        crate::RunState::Stopping { in_flight: 1 }
    ));

    let (state, effects) = crate::update(state, triage_success(request_id));
    assert!(effects
        .iter()
        .all(|effect| !matches!(effect, Effect::RequestLlmCompletion { .. })));
    assert_eq!(state.run_state(), crate::RunState::Idle);
    assert!(state.view().run_enabled);
    assert!(matches!(
        state.unfinished_work(),
        crate::UnfinishedWork::Known(summary) if summary.articles_with_work > 0
    ));
    assert!(state.view().resume_enabled);

    let (unavailable, _) = crate::update(
        state,
        Msg::AiAvailabilityDetected {
            availability: crate::AiAvailability::Unavailable {
                reason: crate::AiUnavailableReason::MissingApiKey,
            },
        },
    );
    assert!(unavailable.view().run_enabled);
    assert!(!unavailable.view().resume_enabled);
    assert_eq!(
        unavailable.view().resume_disabled_reason.as_deref(),
        Some("AI features unavailable: OPENAI_API_KEY is not set")
    );

    let (complete, articles) = prepare_pipeline(1, 0);
    let complete = run_to_completion(complete, &articles, &[true]);
    assert!(complete.view().run_enabled);
    assert!(!complete.view().resume_enabled);
    assert!(matches!(
        complete.unfinished_work(),
        crate::UnfinishedWork::Known(summary) if summary.articles_with_work == 0
    ));
    assert_eq!(
        complete.view().resume_disabled_reason.as_deref(),
        Some("There is no unfinished work to process.")
    );
}

#[test]
fn reprocess_notice_disappears_after_normal_and_stopped_settlement() {
    let (mut state, articles) = prepare_pipeline(1, 0);
    state.pipeline_admission.as_mut().unwrap().reprocess_notice = Some((151, 453));
    assert!(state.view().reprocess_notice.is_some());
    let settled = run_to_completion(state, &articles, &[true]);
    assert!(settled.run_progress().unwrap().terminal);
    assert!(settled.view().reprocess_notice.is_none());

    let (mut state, _) = prepare_pipeline(1, 0);
    state.pipeline_admission.as_mut().unwrap().reprocess_notice = Some((151, 453));
    let request_id = triage_request(&state).expect("in-flight triage");
    let (state, _) = crate::update(state, Msg::StopFinishClicked);
    assert!(state.view().reprocess_notice.is_some());
    let (state, _) = crate::update(state, triage_success(request_id));
    assert!(state.run_progress().unwrap().terminal);
    assert!(state.view().reprocess_notice.is_none());
}

#[test]
fn stop_drain_recounts_each_completion_without_releasing_more_work() {
    let (mut state, _) = prepare_pipeline(2, 0);
    state.set_llm_max_in_flight(2);
    let (state, _) = crate::update(state, Msg::PipelineRunAdvance);
    let in_flight = state
        .triage()
        .articles()
        .iter()
        .filter_map(|article| match article.triage_state {
            crate::triage::ArticleTriageState::InProgress { request_id } => Some(request_id),
            _ => None,
        })
        .collect::<Vec<_>>();
    assert_eq!(in_flight.len(), 2);
    let (state, _) = crate::update(state, Msg::StopFinishClicked);
    let (state, effects) = crate::update(state, triage_success(in_flight[0]));
    assert_eq!(state.pipeline_run_phase(), PipelineRunPhase::Stopping);
    assert_eq!(
        state.run_progress().unwrap().stages[PipelineStage::Triaging.index()].completed,
        1
    );
    assert!(effects
        .iter()
        .all(|effect| !matches!(effect, Effect::RequestLlmCompletion { .. })));

    let (state, effects) = crate::update(state, triage_success(in_flight[1]));
    assert!(effects
        .iter()
        .all(|effect| !matches!(effect, Effect::RequestLlmCompletion { .. })));
    assert!(state.run_progress().unwrap().terminal);
    assert_eq!(
        state.run_progress().unwrap().stages[PipelineStage::Triaging.index()].completed,
        2
    );
}

#[test]
fn waiting_stage_starts_its_clock_when_work_is_admitted() {
    let mut state = tick(add_metadata(AppState::new()), 0);
    begin_run_if_needed(&mut state);
    state.pipeline_admission = Some(crate::pipeline_waves::PipelineAdmission::new(
        crate::PipelineRunScope::Full,
        true,
        Default::default(),
    ));
    super::super::waves::record_progress(&mut state);
    let waiting = &state.run_progress().unwrap().stages[PipelineStage::Triaging.index()];
    assert_eq!(waiting.status, StageStatus::Active);
    assert_eq!(waiting.started_at_utc, None);

    state = tick(state, 20);
    let now = state.last_observed_utc();
    state.pipeline_admission.as_mut().unwrap().admitted[0]
        .insert(("https://progress.invalid/admitted".into(), "hash".into()));
    super::super::waves::record_progress(&mut state);
    let admitted = &state.run_progress().unwrap().stages[PipelineStage::Triaging.index()];
    assert_eq!(admitted.total, 1);
    assert_eq!(admitted.started_at_utc, now);
}

#[test]
fn settle_run_terminalizes_any_residual_active_stage() {
    let mut state = tick(AppState::new(), 0);
    begin_run_if_needed(&mut state);
    let now = state.last_observed_utc();
    state
        .run_progress_mut()
        .expect("run progress")
        .activate(PipelineStage::ScoringSignals, 1, now);

    settle_run(&mut state);

    let progress = state.run_progress().expect("settled run");
    assert!(progress.terminal);
    assert!(progress
        .stages
        .iter()
        .all(|stage| stage.status != StageStatus::Active));
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
    let state = crate::update(
        state,
        Msg::PipelineRunRequested {
            scope: crate::PipelineRunScope::Full,
        },
    )
    .0;
    let state = crate::update::test_support::update(state, Msg::PollStarted { total: 2 }).0;
    let source_id = SourceId::new("failed-source").expect("valid source id");
    let state = crate::update::test_support::update(
        state,
        Msg::SourcePollFailed {
            source_id: source_id.clone(),
            error: "first failure".into(),
        },
    )
    .0;
    let state = crate::update::test_support::update(
        state,
        Msg::SourcePollFailed {
            source_id,
            error: "second failure".into(),
        },
    )
    .0;
    let state = crate::update::test_support::update(state, Msg::AllSourcesPollEnded).0;
    let scan = &state.run_progress().expect("run").stages[PipelineStage::ScanningSources.index()];
    assert_eq!(
        (scan.status, scan.completed, scan.failed),
        (StageStatus::Failed, 0, 2)
    );
}

#[test]
fn poll_only_run_becomes_terminal_when_source_poll_settles() {
    let state = tick(AppState::new(), 0);
    let (state, _) = crate::update(
        state,
        Msg::AiAvailabilityDetected {
            availability: crate::AiAvailability::Unavailable {
                reason: crate::AiUnavailableReason::MissingApiKey,
            },
        },
    );
    let (state, effects) = crate::update(
        state,
        Msg::PipelineRunRequested {
            scope: crate::PipelineRunScope::Full,
        },
    );
    assert_eq!(effects, vec![Effect::PollAllSources]);
    let state = crate::update::test_support::update(state, Msg::PollStarted { total: 1 }).0;
    let state = crate::update::test_support::update(
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
    let state = crate::update::test_support::update(state, Msg::AllSourcesPollEnded).0;

    assert!(matches!(state.pipeline_run_phase(), PipelineRunPhase::Idle));
    assert!(state.run_progress().expect("poll run").terminal);
    assert!(!state.view().run_progress.run_active);
}

#[test]
fn accepted_stop_during_poll_drains_the_poll_without_ingesting_its_urls() {
    let state = tick(AppState::new(), 0);
    let state = crate::update(
        state,
        Msg::PipelineRunRequested {
            scope: crate::PipelineRunScope::Full,
        },
    )
    .0;
    let state = crate::update::test_support::update(state, Msg::PollStarted { total: 1 }).0;
    assert!(state.stop_finish_button_state().is_enabled());

    let (state, effects) = crate::update::test_support::update(state, Msg::StopFinishClicked);

    assert!(effects
        .iter()
        .any(|effect| matches!(effect, Effect::StopFinish { .. })));
    let progress = state.run_progress().expect("stopped poll run");
    assert!(!progress.terminal);
    assert_eq!(state.pipeline_run_phase(), PipelineRunPhase::Stopping);
    assert_eq!(
        state.view().run_state,
        crate::RunState::Stopping { in_flight: 1 }
    );

    let (state, effects) = crate::update::test_support::update(
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
        .all(|effect| !matches!(effect, Effect::EnqueueUrl { .. })));
    let state = crate::update::test_support::update(state, Msg::AllSourcesPollEnded).0;
    let progress = state.run_progress().expect("terminal stopped poll run");
    assert!(progress.terminal);
    assert!(progress
        .stages
        .iter()
        .all(|stage| stage.status != StageStatus::Active));
    assert!(!state.view().run_progress.run_active);
    assert!(matches!(state.pipeline_run_phase(), PipelineRunPhase::Idle));
    assert_eq!(
        state.batch_observation().session_state,
        crate::SessionState::Idle
    );
}

#[test]
fn post_stop_poll_urls_persist_and_full_run_reingests_before_polling_after_restore() {
    let state = tick(add_metadata(AppState::new()), 0);
    let (state, effects) = crate::update(
        state,
        Msg::PipelineRunRequested {
            scope: crate::PipelineRunScope::Full,
        },
    );
    assert!(effects
        .iter()
        .any(|effect| matches!(effect, Effect::PollAllSources)));
    let (state, _) = crate::update(state, Msg::PollStarted { total: 1 });
    let (state, effects) = crate::update(state, Msg::StopFinishClicked);
    assert!(effects
        .iter()
        .any(|effect| matches!(effect, Effect::StopFinish { .. })));

    let (state, effects) = crate::update(
        state,
        Msg::SourcePollCompleted {
            source_id: SourceId::new("stop-first-poll").expect("valid source id"),
            urls: vec!["https://progress.invalid/returned-after-stop".into()],
            kind: SourceKind::Rss,
            parsed: 1,
            dedup_filtered: 0,
        },
    );
    assert!(effects
        .iter()
        .all(|effect| !matches!(effect, Effect::EnqueueUrl { .. })));
    let snapshot = effects
        .iter()
        .find_map(|effect| match effect {
            Effect::PersistRuntimeState { snapshot } => Some(snapshot.clone()),
            _ => None,
        })
        .expect("post-stop poll persists pending intake");
    assert_eq!(
        snapshot.pending_intake,
        ["https://progress.invalid/returned-after-stop"]
    );
    let state = crate::update(state, Msg::AllSourcesPollEnded).0;
    assert!(state.run_progress().unwrap().terminal);
    assert_eq!(
        state.batch_observation().session_state,
        crate::SessionState::Idle
    );

    let restored = crate::update(
        AppState::new(),
        Msg::RestorePendingIntake(snapshot.pending_intake.clone()),
    )
    .0;
    assert_eq!(restored.pending_intake_urls(), snapshot.pending_intake);

    let (resume_state, resume_effects) = crate::update(
        restored.clone(),
        Msg::PipelineRunRequested {
            scope: crate::PipelineRunScope::Resume,
        },
    );
    assert!(resume_effects
        .iter()
        .all(|effect| !matches!(effect, Effect::PollAllSources | Effect::EnqueueUrl { .. })));
    assert_eq!(resume_state.pending_intake_urls(), snapshot.pending_intake);

    let (full_state, effects) = crate::update(
        restored,
        Msg::PipelineRunRequested {
            scope: crate::PipelineRunScope::Full,
        },
    );
    let enqueue_index = effects
        .iter()
        .position(|effect| {
            matches!(
                effect,
                Effect::EnqueueUrl { url, .. }
                    if url == "https://progress.invalid/returned-after-stop"
            )
        })
        .expect("pending URL is re-enqueued");
    let poll_index = effects
        .iter()
        .position(|effect| matches!(effect, Effect::PollAllSources))
        .expect("Full run polls sources");
    assert!(
        enqueue_index < poll_index,
        "pending intake precedes polling"
    );
    assert!(full_state.pending_intake_urls().is_empty());
}

#[test]
fn stop_preserves_downloads_that_were_queued_but_never_started() {
    let (state, _) = crate::update(
        tick(AppState::new(), 0),
        Msg::InputChanged("https://progress.invalid/cancelled-before-start".into()),
    );
    let (state, effects) = crate::update(state, Msg::UrlsSubmitted);
    assert!(effects
        .iter()
        .any(|effect| matches!(effect, Effect::EnqueueUrl { .. })));
    let (state, stop_effects) = crate::update(state, Msg::StopFinishClicked);

    assert!(stop_effects.iter().any(|effect| matches!(
        effect,
        Effect::PersistRuntimeState { snapshot }
            if snapshot.pending_intake == ["https://progress.invalid/cancelled-before-start"]
    )));
    assert_eq!(
        state.pending_intake_urls(),
        ["https://progress.invalid/cancelled-before-start"],
        "queued work cancelled before its first download progress event is retried"
    );

    let (state, _) = crate::update(
        state,
        Msg::JobDone {
            job_id: 1,
            result: crate::JobResultKind::Failed {
                reason: harvester_engine::FailureKind::Cancelled.to_string(),
            },

            extracted_links: Vec::new(),
            fetched_utc: None,
        },
    );
    let (state, effects) = crate::update(
        state,
        Msg::PipelineRunRequested {
            scope: crate::PipelineRunScope::Full,
        },
    );
    assert_eq!(effects.iter().filter(|effect| matches!(effect,
        Effect::EnqueueUrl { url, .. } if url == "https://progress.invalid/cancelled-before-start"
    )).count(), 1);
    assert!(state.pending_intake_urls().is_empty());
}

#[test]
fn pending_url_matching_restored_completed_job_is_not_downloaded_again() {
    let url = "https://progress.invalid/already-downloaded";
    let (state, _) = crate::update(
        AppState::new(),
        Msg::RestoreCompletedJobs(vec![crate::CompletedJobSnapshot {
            url: url.into(),
            tokens: None,
            bytes: None,
            links: Vec::new(),
            fetched_utc: None,
        }]),
    );
    let (state, _) = crate::update(state, Msg::RestorePendingIntake(vec![url.into()]));
    let (state, effects) = crate::update(
        state,
        Msg::PipelineRunRequested {
            scope: crate::PipelineRunScope::Full,
        },
    );
    assert!(effects.iter().all(|effect| !matches!(effect,
        Effect::EnqueueUrl { url: enqueued, .. } if enqueued == url
    )));
    assert!(state.pending_intake_urls().is_empty());
}

#[test]
fn pending_url_completed_successfully_during_stop_is_not_downloaded_again() {
    let url = "https://progress.invalid/finished-during-stop";
    let (state, _) = crate::update(tick(AppState::new(), 0), Msg::InputChanged(url.into()));
    let (state, _) = crate::update(state, Msg::UrlsSubmitted);
    let (state, _) = crate::update(state, Msg::StopFinishClicked);
    assert_eq!(state.pending_intake_urls(), [url]);
    let (state, _) = crate::update(
        state,
        Msg::JobDone {
            job_id: 1,
            result: crate::JobResultKind::Success,
            extracted_links: Vec::new(),
            fetched_utc: None,
        },
    );
    let (state, effects) = crate::update(
        state,
        Msg::PipelineRunRequested {
            scope: crate::PipelineRunScope::Full,
        },
    );
    assert!(effects.iter().all(|effect| !matches!(effect,
        Effect::EnqueueUrl { url: enqueued, .. } if enqueued == url
    )));
    assert!(state.pending_intake_urls().is_empty());
}

#[test]
fn stop_without_new_pending_intake_skips_runtime_snapshot() {
    let (state, _) = crate::update(
        tick(add_metadata(AppState::new()), 0),
        Msg::PipelineRunRequested {
            scope: crate::PipelineRunScope::Full,
        },
    );
    let (_, effects) = crate::update(state, Msg::StopFinishClicked);
    assert!(effects
        .iter()
        .any(|effect| matches!(effect, Effect::StopFinish { .. })));
    assert!(effects
        .iter()
        .all(|effect| !matches!(effect, Effect::PersistRuntimeState { .. })));
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
    let state =
        crate::update::test_support::update(AppState::new(), Msg::RestoreCompletedJobs(snapshots))
            .0;
    let state = crate::update::test_support::update(
        state,
        Msg::PipelineRunRequested {
            scope: crate::PipelineRunScope::Resume,
        },
    )
    .0;
    let stage =
        &state.run_progress().expect("run").stages[PipelineStage::DownloadingArticles.index()];
    assert_eq!((stage.status, stage.total), (StageStatus::Pending, 0));
}

#[test]
fn accepted_stop_during_triage_drains_before_terminalizing_waiting_stages() {
    let (state, _) = prepare_pipeline(1, 0);
    assert_eq!(
        state.run_progress().expect("run").stages[PipelineStage::Triaging.index()].status,
        StageStatus::Active
    );
    let frozen_counts =
        state.run_progress().expect("run").stages[PipelineStage::Triaging.index()].clone();
    let request_id = state
        .triage()
        .articles()
        .iter()
        .find_map(|article| match article.triage_state {
            crate::triage::ArticleTriageState::InProgress { request_id } => Some(request_id),
            _ => None,
        })
        .expect("triage request is in flight");
    let (state, effects) = crate::update::test_support::update(state, Msg::StopFinishClicked);
    assert!(effects
        .iter()
        .any(|effect| matches!(effect, Effect::StopFinish { .. })));
    let progress = state.run_progress().expect("run");
    assert!(!progress.terminal);
    assert_eq!(state.pipeline_run_phase(), PipelineRunPhase::Stopping);
    assert_eq!(
        progress.stages[PipelineStage::Triaging.index()].status,
        StageStatus::Active
    );
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
    let (state, effects) = crate::update::test_support::update(state, Msg::PipelineRunAdvance);
    assert!(effects.is_empty());
    assert_eq!(state.pipeline_run_phase(), PipelineRunPhase::Stopping);
    let (state, effects) = crate::update(
        state,
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
        },
    );
    assert!(effects
        .iter()
        .all(|effect| !matches!(effect, Effect::RequestLlmCompletion { .. })));
    assert!(state.run_progress().unwrap().terminal);
    assert_eq!(
        state.run_progress().unwrap().stages[PipelineStage::Triaging.index()].completed,
        frozen_counts.completed + 1,
        "the in-flight completion must count during the Stop drain"
    );
    assert_eq!(state.pipeline_run_phase(), PipelineRunPhase::Idle);
    assert_eq!(
        state.batch_observation().session_state,
        crate::SessionState::Idle
    );
    assert!(state
        .run_progress()
        .unwrap()
        .stages
        .iter()
        .all(|stage| matches!(stage.status, StageStatus::Done | StageStatus::Failed)));
}

#[test]
fn disabled_stop_intent_does_not_freeze_the_run() {
    let state = add_metadata(AppState::new());
    let (state, _) = crate::update(
        state,
        Msg::PipelineRunRequested {
            scope: crate::PipelineRunScope::Resume,
        },
    );
    let (state, effects) = crate::update(state, Msg::StopFinishClicked);
    assert!(effects.is_empty());
    assert_eq!(state.pipeline_run_phase(), PipelineRunPhase::Requested);
    assert!(!state.run_progress().unwrap().terminal);
}

#[test]
fn triage_loading_is_never_reported_as_settled() {
    let mut state = AppState::new();
    state.set_triage(crate::triage::TriageSession::new_loading(None));
    assert!(!state.pipeline_activity().is_settled());
    assert_eq!(state.batch_status(), BatchStatus::Running);
}

#[test]
fn rerun_counts_current_triage_and_summary_hits_without_readmitting_scoring() {
    let (state, articles) = prepare_pipeline(1, 0);
    let state = run_to_completion(state, &articles, &[true]);
    let old_id = state.run_progress().unwrap().run_id;
    let (state, effects) = crate::update::test_support::update(
        state,
        Msg::PipelineRunRequested {
            scope: crate::PipelineRunScope::Resume,
        },
    );
    assert_ne!(state.run_progress().unwrap().run_id, old_id);
    assert!(state.run_progress().unwrap().terminal);
    assert!(effects
        .iter()
        .all(|e| !matches!(e, Effect::RequestLlmCompletion { .. })));
    for stage in [PipelineStage::Triaging, PipelineStage::Summarizing] {
        let s = &state.run_progress().unwrap().stages[stage.index()];
        assert_eq!(s.reused, 1);
        assert_eq!(
            (s.status, s.completed, s.total, s.total_is_final),
            (StageStatus::Done, 1, 1, true)
        );
    }
    let scoring = &state.run_progress().unwrap().stages[PipelineStage::ScoringSignals.index()];
    assert_eq!(
        (
            scoring.status,
            scoring.completed,
            scoring.total,
            scoring.total_is_final
        ),
        (StageStatus::Done, 0, 0, true)
    );
    assert_eq!(scoring.reused, 0);
    assert!(state.pipeline_admission.as_ref().unwrap().admitted[2].is_empty());
    assert_eq!(
        state
            .pipeline_waves()
            .waves()
            .iter()
            .filter(|w| w.run_id == state.run_progress().unwrap().run_id
                && w.stage == PipelineStage::ScoringSignals)
            .count(),
        0
    );
}

#[test]
fn pipeline_request_joins_an_active_poll_run_without_resetting() {
    let state = tick(AppState::new(), 0);
    let state = crate::update(
        state,
        Msg::PipelineRunRequested {
            scope: crate::PipelineRunScope::Full,
        },
    )
    .0;
    let state = crate::update::test_support::update(state, Msg::PollStarted { total: 3 }).0;
    let run_id = state.run_progress().expect("poll run").run_id;
    let state = crate::update::test_support::update(
        state,
        Msg::PipelineRunRequested {
            scope: crate::PipelineRunScope::Resume,
        },
    )
    .0;
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
    let state = crate::update::test_support::update(state, Msg::PipelineRunAdvance).0;
    assert_eq!(state.run_completion_notice(), Some(&notice));
    let state = crate::update::test_support::update(state, Msg::RunFinishedNoticeDismissed).0;
    assert!(state.run_completion_notice().is_none());
}

#[test]
fn advance_pulses_cannot_overtake_configuration_or_article_loading() {
    let state = add_metadata(AppState::new());
    let (mut state, effects) = crate::update(
        state,
        Msg::PipelineRunRequested {
            scope: crate::PipelineRunScope::Resume,
        },
    );
    assert!(effects
        .iter()
        .any(|e| matches!(e, Effect::LoadProcessingConfiguration { .. })));
    for _ in 0..3 {
        let (next, effects) = crate::update(state, Msg::PipelineRunAdvance);
        state = next;
        assert!(effects.is_empty());
        assert!(!state.run_progress().unwrap().terminal);
    }
}

#[test]
fn second_pipeline_request_is_ignored_while_driver_is_active() {
    let (state, _) = prepare_pipeline(1, 0);
    let state = crate::update::test_support::update(
        state,
        Msg::PipelineRunRequested {
            scope: crate::PipelineRunScope::Resume,
        },
    )
    .0;
    let run_id = state.run_progress().expect("run").run_id;
    let phase = state.pipeline_run_phase();
    let state = crate::update::test_support::update(
        state,
        Msg::PipelineRunRequested {
            scope: crate::PipelineRunScope::Resume,
        },
    )
    .0;
    assert_eq!(state.pipeline_run_phase(), phase);
    assert_eq!(state.run_progress().expect("same run").run_id, run_id);
}

#[test]
fn gui_and_batch_paths_reach_the_same_terminal_pipeline_activity() {
    let articles = vec![loaded_article(0)];
    let mut prepared = add_metadata(AppState::new());
    prepared.set_pre_triage(crate::pre_triage_filter::PreTriageSession::load_articles(
        articles.clone(),
        &crate::pre_triage_filter::PreTriagePolicy::default(),
    ));
    let (gui, gui_effects) = crate::update::test_support::update(
        prepared.clone(),
        Msg::PipelineRunRequested {
            scope: crate::PipelineRunScope::Resume,
        },
    );
    let (batch, batch_effects) = crate::update::test_support::update(
        prepared,
        Msg::PipelineRunRequested {
            scope: crate::PipelineRunScope::Resume,
        },
    );
    assert!(request_id(&gui_effects, PromptId::ArticleTriage).is_some());
    assert!(request_id(&batch_effects, PromptId::ArticleTriage).is_some());
    let gui = run_to_completion(gui, &articles, &[true]);
    let batch = run_to_completion(batch, &articles, &[true]);
    assert_eq!(gui.pipeline_activity(), batch.pipeline_activity());
    assert_eq!(gui.batch_status(), BatchStatus::Settled);
    assert_eq!(batch.batch_status(), BatchStatus::Settled);
    assert!(gui.run_progress().unwrap().terminal && batch.run_progress().unwrap().terminal);
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

#[path = "wave_tests.rs"]
mod wave_tests;

#[path = "reused_work_tests.rs"]
mod reused_work_tests;
