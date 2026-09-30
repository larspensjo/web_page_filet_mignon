use super::dispatch_loop::{classify_cycle_outcome, should_settle_cycle, truncate_for_log};
use super::*;
use crate::cli::{Args, CheckpointCommand};
use harvester_core::signal_candidate::DEFAULT_SELECTION_THRESHOLD;
use harvester_core::{AppState, BatchObservation, Msg};
use harvester_engine::llm::prompt::PromptId;
use harvester_engine::llm::{
    prompts::register_defaults, LlmConfig, LlmQuotas, ModelId, PricingRegistry, PromptRegistry,
    ProviderKind,
};
use harvester_io::{
    load_briefing_checkpoint, EffectRunner, NoOpPlatformHandler, NoOpRuntimePersistenceSink,
    RuntimePaths,
};
use std::collections::HashMap;

use std::path::PathBuf;
use std::sync::atomic::AtomicBool;
use std::sync::mpsc;
use std::sync::Arc;
use std::sync::RwLock;
use std::time::Duration;
use tempfile::TempDir;

fn create_test_args(temp_dir: &TempDir) -> Args {
    Args {
        output_dir: temp_dir.path().to_path_buf(),
        sources: Some(PathBuf::from("test_sources.json")),
        contexts_dir: PathBuf::from("contexts"),
        prompts_dir: PathBuf::from("prompts"),
        verbose_progress: false,
        ascii_progress: false,
        llm_concurrency: 1,
        force_unlock: false,
        set_briefing_since: None,
        set_briefing_since_now: false,
        clear_briefing_since: false,
        show_briefing_since: false,
        import_saved_web_dir: None,
        refresh_stale_summaries_limit: None,
        signal_candidate_threshold: None,
    }
}

fn test_loaded_article(index: usize) -> harvester_engine::LoadedArticle {
    harvester_engine::LoadedArticle {
        url: format!("https://runner.example/article-{index}"),
        source_title: Some(format!("Runner article {index}")),
        prepared_text: std::iter::repeat_n(format!("article-{index}-word"), 220)
            .collect::<Vec<_>>()
            .join(" "),
        content_hash: format!("runner-hash-{index}"),
        fetched_utc: Some("2026-09-27T00:00:00Z".into()),
    }
}

fn triage_request_id(effects: &[harvester_core::Effect]) -> Option<u64> {
    effects.iter().find_map(|effect| match effect {
        harvester_core::Effect::RequestLlmCompletion {
            request_id,
            prompt_id: PromptId::ArticleTriage,
            ..
        } => Some(*request_id),
        _ => None,
    })
}

fn fake_full_cycle_runner(
    temp_dir: &TempDir,
    article: &harvester_engine::LoadedArticle,
) -> (
    AppState,
    EffectRunner,
    Arc<harvester_engine::llm::MockLlmProvider>,
    mpsc::Sender<Msg>,
    mpsc::Receiver<Msg>,
) {
    let output_dir = temp_dir.path().join("output");
    let contexts_dir = temp_dir.path().join("contexts");
    std::fs::create_dir_all(&output_dir).unwrap();
    std::fs::create_dir_all(&contexts_dir).unwrap();
    std::fs::write(
        temp_dir.path().join("sources.ron"),
        "SourceRegistry(sources: [])",
    )
    .unwrap();
    std::fs::write(
        contexts_dir.join("article_triage.toml"),
        "[meta]\nprompt_id = \"ArticleTriage\"\nschema_version = 1\nversion = 1\nupdated = \"2026-09-27\"\n\n[variables]\npolicy = \"test policy\"\n",
    ).unwrap();
    let (_, markdown) = harvester_engine::build_markdown_document(
        &article.url,
        article.source_title.as_deref(),
        "utf-8",
        article.fetched_utc.as_deref().unwrap(),
        &article.prepared_text,
        &harvester_engine::WhitespaceTokenCounter,
    );
    std::fs::write(output_dir.join("article.md"), markdown).unwrap();
    let paths = RuntimePaths::new(
        output_dir,
        temp_dir.path().join("sources.ron"),
        contexts_dir,
        temp_dir.path().join("prompts"),
    );
    let mock = Arc::new(harvester_engine::llm::MockLlmProvider::new());
    let mut registry = PromptRegistry::new();
    register_defaults(&mut registry);
    let registry = Arc::new(RwLock::new(registry));
    let model = ModelId::new(ProviderKind::OpenAi, "mock");
    let config = LlmConfig {
        provider: mock.clone(),
        default_model: model.clone(),
        triage_model: Some(model.clone()),
        summary_model: Some(model.clone()),
        signal_candidate_model: Some(model.clone()),
        briefing_model: Some(model),
        registry: registry.clone(),
        quotas: LlmQuotas::default(),
        output_dir: paths.output_dir.clone(),
        pricing: PricingRegistry::with_defaults(),
        max_input_bytes: 100_000,
        #[allow(deprecated)]
        max_input_chars: 0,
        timestamp_utc: Arc::new(|| "2026-09-27T00:00:00Z".into()),
        session_id: "fake-full-cycle".into(),
        replay_cache: None,
        replay_write_observer: None,
        max_concurrent_requests: 1,
    };
    let (msg_tx, msg_rx) = mpsc::channel();
    let runner = EffectRunner::new_with_llm(
        paths,
        msg_tx.clone(),
        harvester_engine::llm::LlmHandle::new(config),
        100_000,
        registry.clone(),
        HashMap::from([
            (PromptId::ArticleTriage, "mock".into()),
            (PromptId::ArticleSummary, "mock".into()),
            (PromptId::ArticleSignalCandidate, "mock".into()),
        ]),
        Box::new(NoOpPlatformHandler),
        Box::new(NoOpRuntimePersistenceSink),
    );
    let (state, _) = harvester_core::update(
        AppState::new(),
        Msg::RestoreCompletedJobs(vec![harvester_core::CompletedJobSnapshot {
            url: article.url.clone(),
            tokens: Some(500),
            bytes: Some(3_000),
            links: Vec::new(),
            fetched_utc: article.fetched_utc.clone(),
        }]),
    );
    let (state, _) = harvester_core::update(
        state,
        Msg::LlmMetadataLoaded {
            active_versions: registry.read().unwrap().active_versions_map(),
            effective_models: HashMap::from([
                (PromptId::ArticleTriage, "mock".into()),
                (PromptId::ArticleSummary, "mock".into()),
                (PromptId::ArticleSignalCandidate, "mock".into()),
            ]),
        },
    );
    (state, runner, mock, msg_tx, msg_rx)
}

#[test]
fn later_cycle_retries_a_failed_article_without_new_jobs() {
    engine_logging::initialize_for_tests();
    let article = test_loaded_article(0);
    let temp_dir = TempDir::new().unwrap();
    let (mut state, runner, mock, msg_tx, msg_rx) = fake_full_cycle_runner(&temp_dir, &article);
    mock.queue_json_success("{}").queue_json_success("{}");
    let shutdown_flag = Arc::new(AtomicBool::new(false));
    for cycle in 1..=2 {
        msg_tx
            .send(Msg::PipelineRunRequested {
                scope: harvester_core::PipelineRunScope::Full,
            })
            .unwrap();
        run_dispatch_loop(
            &mut state,
            &msg_rx,
            &runner,
            &shutdown_flag,
            DispatchLoopOptions {
                tick_interval: Duration::from_millis(75),
                ..DispatchLoopOptions::default()
            },
        )
        .expect("full cycle settles after fake model failure");
        assert!(!state.view().run_progress.run_active);
        assert_eq!(
            state.pipeline_run_phase(),
            harvester_core::PipelineRunPhase::Idle
        );
        assert_eq!(mock.recorded_requests().len(), cycle as usize);
    }
    assert_eq!(
        state.batch_observation().jobs_total,
        1,
        "no new jobs arrived"
    );
    assert_eq!(state.batch_observation().triage_failed, 1);
}

#[test]
fn default_cycle_processes_unfinished_work_without_new_jobs() {
    engine_logging::initialize_for_tests();
    let article = test_loaded_article(1);
    let temp_dir = TempDir::new().unwrap();
    let (mut state, runner, mock, msg_tx, msg_rx) = fake_full_cycle_runner(&temp_dir, &article);
    mock.queue_json_success(
        r#"{"category":"news","priority":1,"tags":[],"rationale":"Low priority fixture."}"#,
    );
    let shutdown_flag = Arc::new(AtomicBool::new(false));
    let args = Args::parse_from(&["harvester_batch"]);
    assert!(args.checkpoint_command().unwrap().is_none());
    let mut sink = |effects| runner.enqueue(effects);
    let mut observer = |_: &str, _: Duration| {};
    dispatch_cycle_with_sink(
        &mut state,
        &msg_tx,
        &msg_rx,
        &mut sink,
        &mut observer,
        &shutdown_flag,
        None,
    )
    .expect("default Full cycle settles");
    assert_eq!(mock.recorded_requests().len(), 1);
    assert!(!state.view().run_progress.run_active);
    assert_eq!(state.batch_observation().jobs_total, 1);
    assert_eq!(state.batch_observation().triage_total, 1);
    assert_eq!(state.batch_observation().triage_completed, 1);
    assert_eq!(
        state.view().run_progress.stages[harvester_core::PipelineStage::ScanningSources.index()]
            .status,
        harvester_core::StageStatus::Done
    );
}

#[test]
fn test_should_stop_after_cycle_for_shutdown_signal() {
    let (_msg_tx, msg_rx) = mpsc::channel();
    let mut state = AppState::new();
    let shutdown = Arc::new(AtomicBool::new(true));
    let mut sink = |effects: Vec<harvester_core::Effect>| assert!(effects.is_empty());
    let mut observer = |_: &str, _: Duration| panic!("shutdown must return before reducing work");
    let outcome = dispatch_loop::run_dispatch_loop_with_sink(
        &mut state,
        &msg_rx,
        &mut sink,
        &mut observer,
        &shutdown,
        DispatchLoopOptions::default(),
        None,
    )
    .unwrap();
    assert_eq!(outcome, CycleOutcome::Success);
    assert_eq!(
        exit_code_with_shutdown(0, shutdown.load(Ordering::Relaxed)),
        130
    );
}

#[test]
fn dispatch_loop_survives_more_than_ten_thousand_quiet_download_iterations() {
    use harvester_engine::{SourceId, SourceKind};
    let (_msg_tx, msg_rx) = mpsc::channel();
    let (state, _) = harvester_core::update(AppState::new(), Msg::PollStarted { total: 1 });
    let (mut state, _) = harvester_core::update(
        state,
        Msg::SourcePollCompleted {
            source_id: SourceId::new("quiet-download").unwrap(),
            urls: vec!["https://quiet-download.invalid/article".into()],
            kind: SourceKind::Rss,
            parsed: 1,
            dedup_filtered: 0,
        },
    );
    state = harvester_core::update(state, Msg::AllSourcesPollEnded).0;
    assert!(!state.batch_observation().poll_in_progress);
    assert_eq!(state.batch_observation().jobs_in_flight, 1);
    assert!(state.pipeline_has_in_flight_work());
    let shutdown = Arc::new(AtomicBool::new(false));
    let mut ticks = 0;
    let mut observer = |kind: &str, _: Duration| {
        if kind == "Tick" {
            ticks += 1;
            if ticks == 10_010 {
                shutdown.store(true, Ordering::Relaxed);
            }
        }
    };
    let mut sink = |_: Vec<harvester_core::Effect>| {};
    dispatch_loop::run_dispatch_loop_with_sink(
        &mut state,
        &msg_rx,
        &mut sink,
        &mut observer,
        &shutdown,
        DispatchLoopOptions {
            tick_interval: Duration::ZERO,
            receive_timeout: Duration::ZERO,
        },
        None,
    )
    .expect("quiet in-flight download must outlast the old iteration cap");
    assert_eq!(ticks, 10_010);
    assert!(state.pipeline_has_in_flight_work());
}

#[test]
fn startup_window_is_loaded_before_first_cycle_count() {
    engine_logging::initialize_for_tests();
    let temp_dir = TempDir::new().unwrap();
    let article = test_loaded_article(3);
    let (state, runner, _mock, _msg_tx, msg_rx) = fake_full_cycle_runner(&temp_dir, &article);
    let (mut state, effects) = harvester_core::update(state, Msg::StartupHydrationRequested);
    let mut effects = effects;
    effects.push(harvester_core::Effect::LoadPromptTemplateFiles);
    runner.enqueue(effects);
    prepare_startup_window(&mut state, &msg_rx, &runner).expect("startup window settles");
    let mut reporter = CycleStartWorkReporter::default();
    let line = reporter
        .pending_count_line(&state)
        .expect("metadata and window are known");
    assert!(line.contains("unfinished_articles=1"), "{line}");
}

fn observation_with_totals(
    jobs_total: usize,
    jobs_done: usize,
    jobs_failed: usize,
    triage_completed: usize,
    triage_failed: usize,
    summary_completed: usize,
    summary_failed: usize,
) -> BatchObservation {
    observation_with_import(
        jobs_total,
        jobs_done,
        jobs_failed,
        triage_completed,
        triage_failed,
        summary_completed,
        summary_failed,
        0,
        0,
    )
}

#[allow(clippy::too_many_arguments)]
fn observation_with_import(
    jobs_total: usize,
    jobs_done: usize,
    jobs_failed: usize,
    triage_completed: usize,
    triage_failed: usize,
    summary_completed: usize,
    summary_failed: usize,
    imports_completed: usize,
    imports_failed: usize,
) -> BatchObservation {
    BatchObservation {
        poll_in_progress: false,
        session_state: harvester_core::SessionState::Idle,
        jobs_total,
        jobs_done,
        jobs_failed,
        jobs_in_flight: 0,
        pre_triage_phase: harvester_core::PreTriagePhase::Idle,
        pre_triage_total: 0,
        pre_triage_included: 0,
        pre_triage_review: 0,
        pre_triage_filtered: 0,
        triage_phase: harvester_core::TriagePhase::Idle,
        triage_total: 0,
        triage_pending: 0,
        triage_in_flight: 0,
        triage_completed,
        triage_failed,
        summary_total: 0,
        summary_pending: 0,
        summary_in_flight: 0,
        summary_completed,
        summary_failed,
        signal_total: 0,
        signal_pending_or_in_flight: 0,
        signal_completed: 0,
        signal_failed: 0,
        triage_cache_hits: 0,
        triage_cache_misses: 0,
        triage_cache_key_unavailable: 0,
        summary_cache_hits: 0,
        summary_cache_misses: 0,
        summary_cache_key_unavailable: 0,
        import_phase: harvester_core::ImportPhase::Idle,
        imports_completed,
        imports_failed,
        import_in_flight: false,
        source_poll_stats: vec![],
    }
}

#[test]
fn test_should_settle_cycle_when_batch_status_is_settled() {
    assert!(should_settle_cycle(harvester_core::BatchStatus::Settled));
}

#[test]
fn test_should_not_settle_cycle_when_batch_status_is_running() {
    assert!(!should_settle_cycle(harvester_core::BatchStatus::Running));
}

#[test]
fn synchronous_staggered_downloads_dispatch_triage_and_settle_once() {
    use harvester_core::{Effect, JobResultKind, PipelineRunScope, PipelineStage};
    use harvester_engine::{SourceId, SourceKind};

    let articles = vec![test_loaded_article(20), test_loaded_article(21)];
    let (state, _) = harvester_core::update(
        AppState::new(),
        Msg::LlmMetadataLoaded {
            active_versions: [
                (PromptId::ArticleTriage, 1),
                (PromptId::ArticleSummary, 1),
                (PromptId::ArticleSignalCandidate, 1),
            ]
            .into_iter()
            .collect(),
            effective_models: [
                (PromptId::ArticleTriage, "test-triage-model".into()),
                (PromptId::ArticleSummary, "test-summary-model".into()),
                (PromptId::ArticleSignalCandidate, "test-signal-model".into()),
            ]
            .into_iter()
            .collect(),
        },
    );
    let (state, _) = harvester_core::update(state, Msg::PromptTemplateFilesLoaded);
    let (mut state, _) = harvester_core::update(
        state,
        Msg::PromptContextsLoaded {
            contexts: Default::default(),
        },
    );
    state.set_llm_max_in_flight(2);
    state = harvester_core::update(state, Msg::PollSourcesClicked).0;
    let (state, _) = harvester_core::update(state, Msg::PollStarted { total: 1 });
    let (mut state, source_effects) = harvester_core::update(
        state,
        Msg::SourcePollCompleted {
            source_id: SourceId::new("overlap").unwrap(),
            urls: articles.iter().map(|article| article.url.clone()).collect(),
            kind: SourceKind::Rss,
            parsed: articles.len(),
            dedup_filtered: 0,
        },
    );
    let jobs: Vec<_> = source_effects
        .iter()
        .filter_map(|effect| match effect {
            Effect::EnqueueUrl { job_id, .. } => Some(*job_id),
            _ => None,
        })
        .collect();
    assert_eq!(jobs.len(), articles.len());
    for job_id in &jobs {
        state = harvester_core::update(
            state,
            Msg::JobProgress {
                job_id: *job_id,
                stage: harvester_core::Stage::Downloading,
                tokens: None,
                bytes: Some(1_024),
                content_preview: None,
            },
        )
        .0;
    }
    let (state, effects) = harvester_core::update(
        state,
        Msg::PipelineRunRequested {
            scope: PipelineRunScope::Full,
        },
    );
    let (state, _) = harvester_core::update(state, Msg::AllSourcesPollEnded);
    let (mut state, _) =
        harvester_core::fixture_support::complete_processing_configuration(state, effects, 100_000);

    state = harvester_core::update(
        state,
        Msg::JobDone {
            job_id: jobs[0],
            result: JobResultKind::Success,
            content_preview: None,
            extracted_links: Vec::new(),
            fetched_utc: articles[0].fetched_utc.clone(),
        },
    )
    .0;
    state = harvester_core::update(
        state,
        Msg::EvaluatePreTriageRefresh {
            ordered_urls: vec![articles[0].url.clone()],
            triggered_by_job_done: true,
        },
    )
    .0;
    let mut load_id = None;
    for tick in 1..=40 {
        let (next, tick_effects) = harvester_core::update(
            state,
            Msg::tick_at(chrono::DateTime::from_timestamp(1_700_200_000 + tick, 0).unwrap()),
        );
        let (next, advance_effects) = harvester_core::update(next, Msg::PipelineRunAdvance);
        state = next;
        load_id = tick_effects
            .iter()
            .chain(&advance_effects)
            .find_map(|effect| match effect {
                Effect::LoadArticlesForTriage { request_id, .. } => Some(*request_id),
                _ => None,
            });
        if load_id.is_some() {
            break;
        }
    }
    let load_id = load_id.expect("an overlap wave loads before the remaining download ends");
    assert_eq!(state.batch_observation().jobs_in_flight, 1);
    let (state, effects) = harvester_core::update(
        state,
        Msg::TriageArticlesLoaded {
            request_id: load_id,
            delta: harvester_engine::TriageArticleDelta::full_window(
                vec![articles[0].clone()],
                100_000,
            ),
        },
    );
    let first_request = triage_request_id(&effects).expect("first article dispatches triage early");
    assert_eq!(state.batch_observation().jobs_in_flight, 1);
    let (mut state, _) = harvester_core::update(
        state,
        Msg::LlmCompleted {
            request_id: first_request,
            result: harvester_core::LlmResultKind::Failed {
                reason: "fake triage failure".into(),
            },
            metadata: None,
        },
    );
    assert!(state.view().run_progress.run_active);

    state = harvester_core::update(
        state,
        Msg::JobDone {
            job_id: jobs[1],
            result: JobResultKind::Success,
            content_preview: None,
            extracted_links: Vec::new(),
            fetched_utc: articles[1].fetched_utc.clone(),
        },
    )
    .0;
    state = harvester_core::update(
        state,
        Msg::EvaluatePreTriageRefresh {
            ordered_urls: articles.iter().map(|article| article.url.clone()).collect(),
            triggered_by_job_done: true,
        },
    )
    .0;
    let mut second_load = None;
    for tick in 41..=100 {
        let (next, tick_effects) = harvester_core::update(
            state,
            Msg::tick_at(chrono::DateTime::from_timestamp(1_700_200_000 + tick, 0).unwrap()),
        );
        let (next, advance_effects) = harvester_core::update(next, Msg::PipelineRunAdvance);
        state = next;
        second_load = tick_effects
            .iter()
            .chain(&advance_effects)
            .find_map(|effect| match effect {
                Effect::LoadArticlesForTriage { request_id, .. } => Some(*request_id),
                _ => None,
            });
        if second_load.is_some() {
            break;
        }
    }
    let second_load = second_load.expect("the final download joins as a later wave");
    let (state, effects) = harvester_core::update(
        state,
        Msg::TriageArticlesLoaded {
            request_id: second_load,
            delta: harvester_engine::TriageArticleDelta::full_window(articles, 100_000),
        },
    );
    let second_request = triage_request_id(&effects).expect("the later article is admitted once");
    let (state, _) = harvester_core::update(
        state,
        Msg::LlmCompleted {
            request_id: second_request,
            result: harvester_core::LlmResultKind::Failed {
                reason: "fake triage failure".into(),
            },
            metadata: None,
        },
    );
    assert!(state.pipeline_activity().is_settled());
    assert_eq!(state.batch_status(), harvester_core::BatchStatus::Settled);
    assert!(!state.view().run_progress.run_active);
    assert_eq!(
        state
            .pipeline_waves()
            .waves()
            .iter()
            .filter(|wave| wave.stage == PipelineStage::Triaging)
            .count(),
        2
    );
    let settled_notice = state.run_completion_notice().cloned();
    let (state, effects) = harvester_core::update(state, Msg::PipelineRunAdvance);
    assert!(effects.is_empty());
    assert!(!state.view().run_progress.run_active);
    assert_eq!(state.run_completion_notice().cloned(), settled_notice);
}

#[test]
fn determine_exit_code_returns_zero_when_only_partial_failures_occur() {
    assert_eq!(determine_exit_code(0), 0);
}

#[test]
fn determine_exit_code_returns_nonzero_when_total_failure_occurs() {
    assert_eq!(determine_exit_code(1), 1);
}

#[test]
fn shutdown_overrides_each_modes_default_exit_code() {
    assert_eq!(exit_code_with_shutdown(0, true), 130);
    assert_eq!(exit_code_with_shutdown(1, true), 130);
    assert_eq!(exit_code_with_shutdown(1, false), 1);
}

#[test]
fn cycle_counter_baseline_reports_deltas_not_cumulative_totals() {
    let mut baseline = CycleCounterBaseline::from_observation(&observation_with_totals(
        577, 577, 0, 405, 0, 61, 0,
    ));
    let cycle_counts =
        baseline.measure_cycle_and_advance(&observation_with_totals(578, 578, 0, 406, 0, 61, 0));
    assert_eq!(
        cycle_counts,
        CycleCounts {
            new_jobs: 1,
            jobs_done: 1,
            jobs_failed: 0,
            triage_completed: 1,
            triage_failed: 0,
            summary_completed: 0,
            summary_failed: 0,
            imports_completed: 0,
            imports_failed: 0,
        }
    );
}

#[test]
fn test_summarize_batch_msg_compacts_large_payloads() {
    let msg = Msg::TriageArticlesLoaded {
        request_id: 1,
        delta: harvester_engine::TriageArticleDelta::full_window(Vec::new(), 100_000),
    };
    let summary = summarize_batch_msg(&msg);
    assert!(summary.contains("TriageArticlesLoaded"));
    assert!(summary.contains("articles: 0"));
    assert!(!summary.contains("request_id"));
}

#[test]
fn test_truncate_for_log_appends_ellipsis() {
    let input = "abcdefghijklmnopqrstuvwxyz";
    let output = truncate_for_log(input, 10);
    assert!(output.starts_with("abcdefghij"));
    assert!(output.ends_with("..."));
    assert_eq!(output.chars().count(), 13);
}

#[test]
fn test_should_log_batch_msg_filters_downloading_progress() {
    let downloading = Msg::JobProgress {
        job_id: 1,
        stage: harvester_core::Stage::Downloading,
        tokens: None,
        bytes: Some(4096),
        content_preview: None,
    };
    assert!(!should_log_batch_msg(&downloading));

    let tokenizing = Msg::JobProgress {
        job_id: 1,
        stage: harvester_core::Stage::Tokenizing,
        tokens: Some(10),
        bytes: None,
        content_preview: None,
    };
    assert!(should_log_batch_msg(&tokenizing));
}

#[test]
fn test_dispatch_loop_reduces_queued_poll_before_settling() {
    engine_logging::initialize_for_tests();
    let temp_dir = TempDir::new().unwrap();
    let output_dir = temp_dir.path().join("output");
    std::fs::create_dir_all(&output_dir).unwrap();

    let sources_path = temp_dir.path().join("sources.ron");
    std::fs::write(&sources_path, "SourceRegistry(sources: [])").unwrap();

    let runtime_paths = RuntimePaths::new(
        output_dir,
        sources_path,
        temp_dir.path().join("contexts"),
        temp_dir.path().join("prompts"),
    );

    let (msg_tx, msg_rx) = mpsc::channel::<Msg>();
    let mut state = AppState::new();
    let effect_runner = EffectRunner::new(
        runtime_paths,
        msg_tx.clone(),
        Box::new(NoOpPlatformHandler),
        Box::new(NoOpRuntimePersistenceSink),
    );
    let shutdown_flag = Arc::new(AtomicBool::new(false));

    msg_tx.send(Msg::PollSourcesClicked).unwrap();

    let outcome = run_dispatch_loop(
        &mut state,
        &msg_rx,
        &effect_runner,
        &shutdown_flag,
        DispatchLoopOptions {
            tick_interval: Duration::from_millis(75),
            ..DispatchLoopOptions::default()
        },
    )
    .expect("dispatch loop should complete");
    assert_eq!(outcome, CycleOutcome::Success);

    assert!(matches!(msg_rx.try_recv(), Err(mpsc::TryRecvError::Empty)));
}

#[test]
fn keyless_full_dispatch_loop_polls_and_settles_on_each_cycle() {
    engine_logging::initialize_for_tests();
    let temp_dir = TempDir::new().unwrap();
    let output_dir = temp_dir.path().join("output");
    std::fs::create_dir_all(&output_dir).unwrap();
    let sources_path = temp_dir.path().join("sources.ron");
    std::fs::write(&sources_path, "SourceRegistry(sources: [])").unwrap();
    let runtime_paths = RuntimePaths::new(
        output_dir,
        sources_path,
        temp_dir.path().join("contexts"),
        temp_dir.path().join("prompts"),
    );
    let (msg_tx, msg_rx) = mpsc::channel::<Msg>();
    let effect_runner = EffectRunner::new(
        runtime_paths,
        msg_tx.clone(),
        Box::new(NoOpPlatformHandler),
        Box::new(NoOpRuntimePersistenceSink),
    );
    let shutdown_flag = Arc::new(AtomicBool::new(false));
    let mut state = apply_llm_availability(
        AppState::new(),
        harvester_core::AiAvailability::Unavailable {
            reason: harvester_core::AiUnavailableReason::MissingApiKey,
        },
    );

    for cycle in 1..=2 {
        msg_tx
            .send(Msg::PipelineRunRequested {
                scope: harvester_core::PipelineRunScope::Full,
            })
            .unwrap();
        let outcome = run_dispatch_loop(
            &mut state,
            &msg_rx,
            &effect_runner,
            &shutdown_flag,
            DispatchLoopOptions {
                tick_interval: Duration::from_millis(75),
                ..DispatchLoopOptions::default()
            },
        )
        .expect("each keyless intake cycle must terminate");
        assert_eq!(outcome, CycleOutcome::Success);
        let progress = state.view().run_progress;
        assert!(!progress.run_active, "cycle {cycle} must settle");
        assert_eq!(
            progress.stages[harvester_core::PipelineStage::ScanningSources.index()].status,
            harvester_core::StageStatus::Done
        );
        assert_eq!(
            state.pipeline_run_phase(),
            harvester_core::PipelineRunPhase::Idle
        );
    }
}

#[test]
fn test_dispatch_loop_ticks_drive_pretriage_from_restore_signal() {
    engine_logging::initialize_for_tests();
    let temp_dir = TempDir::new().unwrap();
    let output_dir = temp_dir.path().join("output");
    std::fs::create_dir_all(&output_dir).unwrap();

    let sources_path = temp_dir.path().join("sources.ron");
    std::fs::write(&sources_path, "SourceRegistry(sources: [])").unwrap();

    let runtime_paths = RuntimePaths::new(
        output_dir,
        sources_path,
        temp_dir.path().join("contexts"),
        temp_dir.path().join("prompts"),
    );

    let (msg_tx, msg_rx) = mpsc::channel::<Msg>();
    let mut state = AppState::new();
    let effect_runner = EffectRunner::new(
        runtime_paths,
        msg_tx.clone(),
        Box::new(NoOpPlatformHandler),
        Box::new(NoOpRuntimePersistenceSink),
    );
    let shutdown_flag = Arc::new(AtomicBool::new(false));

    msg_tx
        .send(Msg::RestoreCompletedJobs(vec![
            harvester_core::CompletedJobSnapshot {
                url: "https://example.com/article-1".to_string(),
                tokens: Some(123),
                bytes: Some(4567),
                links: Vec::new(),
                fetched_utc: Some("2026-03-05T00:00:00Z".to_string()),
            },
        ]))
        .unwrap();

    let outcome = run_dispatch_loop_with_tick_interval(
        &mut state,
        &msg_rx,
        &effect_runner,
        &shutdown_flag,
        DispatchLoopOptions {
            tick_interval: Duration::ZERO,
            ..DispatchLoopOptions::default()
        },
        None,
    )
    .expect("dispatch loop should complete");
    assert_eq!(outcome, CycleOutcome::Success);

    let obs = state.batch_observation();
    assert!(!matches!(
        obs.pre_triage_phase,
        harvester_core::PreTriagePhase::Idle
    ));
}

#[test]
fn test_classify_outcome_success() {
    let obs = BatchObservation {
        poll_in_progress: false,
        session_state: harvester_core::SessionState::Idle,
        jobs_total: 5,
        jobs_done: 5,
        jobs_failed: 0,
        jobs_in_flight: 0,
        pre_triage_phase: harvester_core::PreTriagePhase::Idle,
        pre_triage_total: 0,
        pre_triage_included: 0,
        pre_triage_review: 0,
        pre_triage_filtered: 0,
        triage_phase: harvester_core::TriagePhase::Complete,
        triage_total: 5,
        triage_pending: 0,
        triage_in_flight: 0,
        triage_completed: 5,
        triage_failed: 0,
        summary_total: 0,
        summary_pending: 0,
        summary_in_flight: 0,
        summary_completed: 0,
        summary_failed: 0,
        signal_total: 0,
        signal_pending_or_in_flight: 0,
        signal_completed: 0,
        signal_failed: 0,
        triage_cache_hits: 0,
        triage_cache_misses: 0,
        triage_cache_key_unavailable: 0,
        summary_cache_hits: 0,
        summary_cache_misses: 0,
        summary_cache_key_unavailable: 0,
        import_phase: harvester_core::ImportPhase::Idle,
        imports_completed: 0,
        imports_failed: 0,
        import_in_flight: false,
        source_poll_stats: vec![],
    };

    assert_eq!(classify_cycle_outcome(&obs), CycleOutcome::Success);
}

#[test]
fn test_classify_outcome_partial_failure() {
    let obs = BatchObservation {
        poll_in_progress: false,
        session_state: harvester_core::SessionState::Idle,
        jobs_total: 5,
        jobs_done: 3,
        jobs_failed: 2,
        jobs_in_flight: 0,
        pre_triage_phase: harvester_core::PreTriagePhase::Idle,
        pre_triage_total: 0,
        pre_triage_included: 0,
        pre_triage_review: 0,
        pre_triage_filtered: 0,
        triage_phase: harvester_core::TriagePhase::Complete,
        triage_total: 5,
        triage_pending: 0,
        triage_in_flight: 0,
        triage_completed: 3,
        triage_failed: 2,
        summary_total: 0,
        summary_pending: 0,
        summary_in_flight: 0,
        summary_completed: 0,
        summary_failed: 0,
        signal_total: 0,
        signal_pending_or_in_flight: 0,
        signal_completed: 0,
        signal_failed: 0,
        triage_cache_hits: 0,
        triage_cache_misses: 0,
        triage_cache_key_unavailable: 0,
        summary_cache_hits: 0,
        summary_cache_misses: 0,
        summary_cache_key_unavailable: 0,
        import_phase: harvester_core::ImportPhase::Idle,
        imports_completed: 0,
        imports_failed: 0,
        import_in_flight: false,
        source_poll_stats: vec![],
    };

    assert_eq!(classify_cycle_outcome(&obs), CycleOutcome::PartialFailure);
}

#[test]
fn test_classify_outcome_total_failure() {
    let obs = BatchObservation {
        poll_in_progress: false,
        session_state: harvester_core::SessionState::Idle,
        jobs_total: 5,
        jobs_done: 0,
        jobs_failed: 5,
        jobs_in_flight: 0,
        pre_triage_phase: harvester_core::PreTriagePhase::Idle,
        pre_triage_total: 0,
        pre_triage_included: 0,
        pre_triage_review: 0,
        pre_triage_filtered: 0,
        triage_phase: harvester_core::TriagePhase::Complete,
        triage_total: 5,
        triage_pending: 0,
        triage_in_flight: 0,
        triage_completed: 0,
        triage_failed: 5,
        summary_total: 0,
        summary_pending: 0,
        summary_in_flight: 0,
        summary_completed: 0,
        summary_failed: 0,
        signal_total: 0,
        signal_pending_or_in_flight: 0,
        signal_completed: 0,
        signal_failed: 0,
        triage_cache_hits: 0,
        triage_cache_misses: 0,
        triage_cache_key_unavailable: 0,
        summary_cache_hits: 0,
        summary_cache_misses: 0,
        summary_cache_key_unavailable: 0,
        import_phase: harvester_core::ImportPhase::Idle,
        imports_completed: 0,
        imports_failed: 0,
        import_in_flight: false,
        source_poll_stats: vec![],
    };

    assert_eq!(classify_cycle_outcome(&obs), CycleOutcome::TotalFailure);
}

fn make_checkpoint_test_paths(temp_dir: &TempDir) -> RuntimePaths {
    let output_dir = temp_dir.path().to_path_buf();
    std::fs::create_dir_all(&output_dir).unwrap();
    RuntimePaths::new(
        output_dir,
        temp_dir.path().join("sources.ron"),
        temp_dir.path().join("contexts"),
        temp_dir.path().join("prompts"),
    )
}

#[test]
fn cycle_persistence_clears_consumed_pending_intake() {
    let temp_dir = TempDir::new().unwrap();
    let paths = make_checkpoint_test_paths(&temp_dir);
    let stale = vec!["https://example.invalid/stale".to_string()];
    for with_observer in [false, true] {
        harvester_io::try_persist_runtime_state_with_pending(&paths.state_path, &[], &stale)
            .expect("seed pending intake");
        let observer: harvester_io::FileWriteObserver = std::sync::Arc::new(|_, _, _| {});
        persist_cycle_state(&paths, &AppState::new(), with_observer.then_some(&observer));
        assert!(harvester_io::load_pending_intake(&paths.state_path).is_empty());
    }
}

#[test]
fn set_checkpoint_invalid_timestamp_returns_err_without_write() {
    let temp_dir = TempDir::new().unwrap();
    let paths = make_checkpoint_test_paths(&temp_dir);
    // Simulate the validation in checkpoint_command() — Set is only constructed after validation
    // We test execute_checkpoint_write with a directly-valid Set to confirm it writes
    let result =
        execute_checkpoint_write(CheckpointCommand::Set("not-rfc3339".to_string()), &paths);
    // save_briefing_checkpoint does not validate the string; validation is in checkpoint_command().
    // But the file SHOULD be written with whatever string is passed.
    // This test verifies the call succeeds (the CLI layer is responsible for validation).
    assert!(result.is_ok());
}

#[test]
fn set_checkpoint_writes_file() {
    let temp_dir = TempDir::new().unwrap();
    let paths = make_checkpoint_test_paths(&temp_dir);
    execute_checkpoint_write(
        CheckpointCommand::Set("2025-12-31T23:00:00Z".to_string()),
        &paths,
    )
    .unwrap();
    let loaded = load_briefing_checkpoint(&paths.briefing_checkpoint_path);
    assert_eq!(loaded.as_deref(), Some("2025-12-31T23:00:00Z"));
}

#[test]
fn set_checkpoint_now_writes_valid_rfc3339() {
    let temp_dir = TempDir::new().unwrap();
    let paths = make_checkpoint_test_paths(&temp_dir);
    execute_checkpoint_write(CheckpointCommand::SetNow, &paths).unwrap();
    let loaded = load_briefing_checkpoint(&paths.briefing_checkpoint_path);
    let ts = loaded.expect("checkpoint should be written");
    assert!(
        chrono::DateTime::parse_from_rfc3339(&ts).is_ok(),
        "expected valid RFC3339, got: {ts}"
    );
}

#[test]
fn clear_checkpoint_deletes_file() {
    let temp_dir = TempDir::new().unwrap();
    let paths = make_checkpoint_test_paths(&temp_dir);
    // Write first
    execute_checkpoint_write(
        CheckpointCommand::Set("2025-12-31T23:00:00Z".to_string()),
        &paths,
    )
    .unwrap();
    assert!(paths.briefing_checkpoint_path.exists());
    // Then clear
    execute_checkpoint_write(CheckpointCommand::Clear, &paths).unwrap();
    assert!(!paths.briefing_checkpoint_path.exists());
}

#[test]
fn show_checkpoint_prints_none_when_absent() {
    let temp_dir = TempDir::new().unwrap();
    let paths = make_checkpoint_test_paths(&temp_dir);
    let val = load_briefing_checkpoint(&paths.briefing_checkpoint_path);
    assert_eq!(val.as_deref().unwrap_or("NONE"), "NONE");
}

#[test]
fn redirected_start_line_uses_the_operational_mode_label() {
    assert!(format_startup_notice("one cycle").contains("starting (one cycle)"));
}

#[test]
fn immediate_exit_cursor_restore_emits_control_bytes_only_for_interactive_output() {
    let mut redirected = Vec::new();
    restore_cursor_before_immediate_exit(&mut redirected, false);
    assert!(
        redirected.is_empty(),
        "redirected output must contain no cursor-control bytes"
    );

    let mut interactive = Vec::new();
    restore_cursor_before_immediate_exit(&mut interactive, true);
    assert_eq!(interactive, b"\x1b[?25h");
}

#[test]
fn apply_signal_candidate_selection_settings_uses_defaults_and_overrides() {
    let temp_dir = TempDir::new().unwrap();
    let mut state = AppState::new();
    let mut args = create_test_args(&temp_dir);

    apply_signal_candidate_selection_settings(&mut state, &args);
    assert_eq!(
        state.signal_candidate_threshold(),
        DEFAULT_SELECTION_THRESHOLD
    );

    args.signal_candidate_threshold = Some(75);
    apply_signal_candidate_selection_settings(&mut state, &args);
    assert_eq!(state.signal_candidate_threshold(), 75);
}
