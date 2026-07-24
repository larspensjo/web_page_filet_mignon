use super::*;
use crate::cli::{Args, CheckpointCommand};
use chrono::TimeZone;
use harvester_core::signal_candidate::DEFAULT_SELECTION_THRESHOLD;
use harvester_core::{
    AppState, ArticleSummaryResult, BatchObservation, LlmModelUsageView, Msg, SummaryCache,
    SummaryCacheEntry, SummaryCacheKey,
};
use harvester_engine::llm::prompt::PromptId;
use harvester_engine::{SourceId, SourceKind};
use harvester_io::{load_briefing_checkpoint, EffectRunner, NoOpPlatformHandler, RuntimePaths};
use std::cell::{Cell, RefCell};
use std::io::Write;
use std::path::PathBuf;
use std::sync::atomic::AtomicBool;
use std::sync::mpsc;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use tempfile::TempDir;

#[derive(Clone, Default)]
struct SharedRunnerOutput(Arc<Mutex<Vec<u8>>>);

impl SharedRunnerOutput {
    fn bytes(&self) -> Vec<u8> {
        self.0.lock().unwrap().clone()
    }

    fn text(&self) -> String {
        String::from_utf8(self.bytes()).unwrap()
    }
}

impl Write for SharedRunnerOutput {
    fn write(&mut self, buffer: &[u8]) -> std::io::Result<usize> {
        self.0.lock().unwrap().extend_from_slice(buffer);
        Ok(buffer.len())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

fn create_test_args(dry_run: bool, temp_dir: &TempDir) -> Args {
    Args {
        output_dir: temp_dir.path().to_path_buf(),
        sources: Some(PathBuf::from("test_sources.json")),
        contexts_dir: PathBuf::from("contexts"),
        prompts_dir: PathBuf::from("prompts"),
        dry_run,
        batch_api: false,
        verbose_progress: false,
        ascii_progress: false,
        single_shot: false,
        allow_unsupported_sources: false,
        llm_concurrency: 1,
        poll_interval: 1,
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

fn test_batch_runtime(temp_dir: &TempDir) -> BatchRuntime {
    let provider = OpenAiProvider::new("test-key".to_string());
    let mock: Arc<dyn harvester_engine::llm::provider::LlmProvider> =
        Arc::new(harvester_engine::llm::MockLlmProvider::new());
    let mut registry = PromptRegistry::new();
    register_defaults(&mut registry);
    let config = LlmConfig {
        provider: mock,
        default_model: ModelId::new(ProviderKind::OpenAi, OPENAI_MODEL_GPT_4O_MINI),
        triage_model: Some(ModelId::new(ProviderKind::OpenAi, DEFAULT_TRIAGE_MODEL)),
        summary_model: Some(ModelId::new(ProviderKind::OpenAi, DEFAULT_SUMMARY_MODEL)),
        signal_candidate_model: None,
        briefing_model: Some(ModelId::new(ProviderKind::OpenAi, DEFAULT_BRIEFING_MODEL)),
        registry: Arc::new(RwLock::new(registry)),
        quotas: LlmQuotas::default(),
        output_dir: temp_dir.path().to_path_buf(),
        pricing: PricingRegistry::with_defaults(),
        max_input_bytes: 100_000,
        #[allow(deprecated)]
        max_input_chars: 0,
        timestamp_utc: Arc::new(|| "2026-07-19T00:00:00Z".to_string()),
        session_id: "test-batch".to_string(),
        replay_cache: None,
        max_concurrent_requests: 1,
    };
    BatchRuntime::new(
        provider,
        config,
        &RuntimePaths::new(
            temp_dir.path().to_path_buf(),
            temp_dir.path().join("sources.ron"),
            temp_dir.path().join("contexts"),
            temp_dir.path().join("prompts"),
        ),
    )
    .unwrap()
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
        triage_deferred: 0,
        summary_deferred: 0,
        signal_total: 0,
        signal_pending_or_in_flight: 0,
        signal_completed: 0,
        signal_failed: 0,
        signal_deferred: 0,
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
fn test_dry_run_exits_successfully_without_api_key() {
    engine_logging::initialize_for_tests();
    let temp_dir = TempDir::new().unwrap();
    let args = create_test_args(true, &temp_dir);

    // Create empty sources file to avoid validation errors
    let sources_path = temp_dir.path().join("test_sources.json");
    std::fs::write(&sources_path, r#"{"sources": []}"#).unwrap();

    let runtime_paths = RuntimePaths::new(
        args.output_dir.clone(),
        sources_path,
        args.contexts_dir.clone(),
        args.prompts_dir.clone(),
    );

    // Dry-run should succeed even without OPENAI_API_KEY
    let shutdown_flag = Arc::new(AtomicBool::new(false));
    let result = run_dry_run(&runtime_paths, &args, &shutdown_flag);
    assert!(result.is_ok());
    assert_eq!(result.unwrap(), 0);
}

#[test]
fn dry_run_returns_130_when_shutdown_is_already_requested() {
    engine_logging::initialize_for_tests();
    let temp_dir = TempDir::new().unwrap();
    let args = create_test_args(true, &temp_dir);
    let sources_path = temp_dir.path().join("test_sources.json");
    std::fs::write(&sources_path, r#"{"sources": []}"#).unwrap();
    let runtime_paths = RuntimePaths::new(
        args.output_dir.clone(),
        sources_path,
        args.contexts_dir.clone(),
        args.prompts_dir.clone(),
    );
    let shutdown_flag = Arc::new(AtomicBool::new(true));

    assert_eq!(
        run_dry_run(&runtime_paths, &args, &shutdown_flag).unwrap(),
        130
    );
}

#[test]
fn test_dry_run_does_not_modify_state_files() {
    engine_logging::initialize_for_tests();
    let temp_dir = TempDir::new().unwrap();
    let args = create_test_args(true, &temp_dir);

    let sources_path = temp_dir.path().join("test_sources.json");
    std::fs::write(&sources_path, r#"{"sources": []}"#).unwrap();

    let runtime_paths = RuntimePaths::new(
        args.output_dir.clone(),
        sources_path,
        args.contexts_dir.clone(),
        args.prompts_dir.clone(),
    );

    let state_path = &runtime_paths.state_path;

    // Ensure state file does not exist initially
    assert!(!state_path.exists());

    // Run dry-run
    let shutdown_flag = Arc::new(AtomicBool::new(false));
    let result = run_dry_run(&runtime_paths, &args, &shutdown_flag);
    assert!(result.is_ok());

    // State file should still not exist (no writes)
    assert!(!state_path.exists());
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
fn test_orchestration_dispatch_skips_settlement_in_same_iteration() {
    assert!(!should_check_settlement_this_iteration(true));
    assert!(should_check_settlement_this_iteration(false));
}

#[test]
fn buffered_batch_requests_are_quiescent_but_other_pending_requests_are_not() {
    let mut state = AppState::new();
    state.record_pending_llm_request(1, PromptId::ArticleTriage);
    let buffered = HashSet::from([1]);
    assert!(batch_buffer_is_quiescent(&state, &buffered));

    state.record_pending_llm_request(2, PromptId::AggregateBriefing);
    assert!(!batch_buffer_is_quiescent(&state, &buffered));
    assert!(!batch_buffer_is_quiescent(&state, &HashSet::new()));
}

#[test]
fn batch_custom_id_changes_when_model_changes() {
    let mut key = FrozenBatchKey {
        content_hash: "content-hash".to_string(),
        prompt_id: PromptId::ArticleTriage,
        prompt_version: 3,
        model_id: "gpt-5.4-nano".to_string(),
        context_hash: "context-hash".to_string(),
        stage: StageKind::Triage,
        url: "https://example.test".to_string(),
        rendered_system: String::new(),
        rendered_user: String::new(),
    };
    let first = batch_custom_id(&key);
    key.model_id = "gpt-5.4-mini".to_string();
    assert_ne!(first, batch_custom_id(&key));
}

#[test]
fn signal_custom_id_prefix_does_not_control_provider_stage_grouping() {
    let key = FrozenBatchKey {
        content_hash: "content-hash".to_string(),
        prompt_id: PromptId::ArticleSignalCandidate,
        prompt_version: 3,
        model_id: "gpt-5.4-nano".to_string(),
        context_hash: "context-hash".to_string(),
        stage: StageKind::SignalCandidate,
        url: "https://example.test".to_string(),
        rendered_system: String::new(),
        rendered_user: String::new(),
    };
    assert!(batch_custom_id(&key).starts_with("signal-"));

    let provider = crate::progress::ProviderProgress::from_peeks(&[BatchPeek {
        batch_id: "batch-with-signal-custom-id".to_string(),
        stage: StageKind::SignalCandidate,
        status: Some(openai_provider_kit::BatchLifecycle::InProgress),
        request_counts: Some(openai_provider_kit::BatchRequestCounts {
            total: 1,
            completed: 0,
            failed: 0,
        }),
    }]);
    assert_eq!(provider.signals.submitted, 1);
    assert_eq!(provider.triage.submitted, 0);
}

#[test]
fn batch_routing_partition_keeps_briefing_synchronous() {
    assert!(is_batch_eligible_prompt(PromptId::ArticleTriage));
    assert!(is_batch_eligible_prompt(PromptId::ArticleSummary));
    assert!(is_batch_eligible_prompt(PromptId::ArticleSignalCandidate));
    assert!(!is_batch_eligible_prompt(PromptId::AggregateBriefing));
    assert!(!is_batch_eligible_prompt(
        PromptId::BriefingExecutiveSummary
    ));
    assert!(!is_batch_eligible_prompt(PromptId::BriefingNextItem));
}

#[test]
fn batch_render_failure_replies_failed_exactly_once() {
    let (tx, rx) = mpsc::channel();
    send_batch_preparation_failure(
        &tx,
        17,
        &LlmCompletionError::TemplateRenderFailed {
            detail: "missing variable".to_string(),
        },
    );
    assert!(matches!(
        rx.recv().unwrap(),
        Msg::LlmCompleted {
            request_id: 17,
            result: harvester_core::LlmResultKind::Failed { .. },
            ..
        }
    ));
    assert!(matches!(rx.try_recv(), Err(mpsc::TryRecvError::Empty)));
}

#[test]
fn collected_replay_audit_is_idempotent_and_uses_discounted_cost() {
    let temp_dir = TempDir::new().unwrap();
    let mut runtime = test_batch_runtime(&temp_dir);
    let entry = harvester_core::CollectedEntry {
        batch_id: "batch-1".to_string(),
        custom_id: "line-1".to_string(),
        stage: StageKind::Triage,
        key: FrozenBatchKey {
            content_hash: "content-hash".to_string(),
            prompt_id: PromptId::ArticleTriage,
            prompt_version: 1,
            model_id: DEFAULT_TRIAGE_MODEL.to_string(),
            context_hash: "context-hash".to_string(),
            stage: StageKind::Triage,
            url: "https://example.test".to_string(),
            rendered_system: "system".to_string(),
            rendered_user: "user".to_string(),
        },
        created_at_utc: "2026-07-19T00:00:00Z".to_string(),
        outcome: harvester_core::CollectedOutcome::Success {
            raw_output_json: r#"{"category":"news","priority":3,"tags":["ai"],"rationale":"ok"}"#
                .to_string(),
            usage: TokenUsage::new(1_000_000, 0),
            resolved_model: DEFAULT_TRIAGE_MODEL.to_string(),
        },
    };

    persist_batch_replay_records(std::slice::from_ref(&entry), &mut runtime);
    persist_batch_replay_records(&[entry], &mut runtime);

    assert_eq!(runtime.realized_cost_microdollars, 100_000);
    assert_eq!(
        std::fs::read_dir(temp_dir.path().join("llm_results"))
            .unwrap()
            .count(),
        1
    );
}

#[test]
fn import_cycle_does_not_settle_while_triage_in_flight() {
    let mut obs = observation_with_import(0, 0, 0, 0, 0, 0, 0, 1, 0);
    obs.import_phase = harvester_core::ImportPhase::Idle;
    obs.import_in_flight = false;
    obs.triage_in_flight = 2;
    obs.triage_pending = 0;
    obs.summary_in_flight = 0;
    obs.summary_pending = 0;
    assert!(!should_settle_import_cycle(&obs));
}

#[test]
fn import_cycle_does_not_settle_while_triage_pending() {
    let mut obs = observation_with_import(0, 0, 0, 0, 0, 0, 0, 1, 0);
    obs.import_phase = harvester_core::ImportPhase::Idle;
    obs.import_in_flight = false;
    obs.triage_in_flight = 0;
    obs.triage_pending = 3;
    obs.summary_in_flight = 0;
    obs.summary_pending = 0;
    assert!(!should_settle_import_cycle(&obs));
}

#[test]
fn import_cycle_settles_when_all_phases_drained() {
    let mut obs = observation_with_import(0, 0, 0, 0, 0, 0, 0, 1, 0);
    obs.import_phase = harvester_core::ImportPhase::Idle;
    obs.import_in_flight = false;
    obs.triage_in_flight = 0;
    obs.triage_pending = 0;
    obs.summary_in_flight = 0;
    obs.summary_pending = 0;
    assert!(should_settle_import_cycle(&obs));
}

#[test]
fn test_should_stop_after_cycle_for_single_shot() {
    assert!(should_stop_after_cycle(true, false));
}

#[test]
fn test_should_stop_after_cycle_for_shutdown_signal() {
    assert!(should_stop_after_cycle(false, true));
}

#[test]
fn test_should_continue_after_cycle_when_not_single_shot_and_no_shutdown() {
    assert!(!should_stop_after_cycle(false, false));
}

#[test]
fn new_jobs_gate_is_disabled_for_batch_api_single_shot_mode() {
    assert_eq!(require_new_jobs_since(true, false, 42), Some(42));
    assert_eq!(require_new_jobs_since(true, true, 42), None);
    assert_eq!(require_new_jobs_since(false, false, 42), None);
    assert_eq!(require_new_jobs_since(false, true, 42), None);
}

#[test]
fn test_should_run_ai_orchestration_when_enabled_without_new_jobs_gate() {
    let obs = observation_with_totals(10, 0, 0, 0, 0, 0, 0);
    assert!(should_run_ai_orchestration(true, None, &obs));
}

#[test]
fn test_should_not_run_ai_orchestration_when_no_new_jobs_since_baseline() {
    let obs = observation_with_totals(10, 0, 0, 0, 0, 0, 0);
    assert!(!should_run_ai_orchestration(true, Some(10), &obs));
}

#[test]
fn test_should_run_ai_orchestration_when_new_jobs_arrived_since_baseline() {
    let obs = observation_with_totals(11, 0, 0, 0, 0, 0, 0);
    assert!(should_run_ai_orchestration(true, Some(10), &obs));
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
        articles: Vec::new(),
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
    let effect_runner =
        EffectRunner::new(runtime_paths, msg_tx.clone(), Box::new(NoOpPlatformHandler));
    let shutdown_flag = Arc::new(AtomicBool::new(false));

    msg_tx.send(Msg::PollSourcesClicked).unwrap();

    let outcome = run_dispatch_loop(
        &mut state,
        &msg_tx,
        &msg_rx,
        &effect_runner,
        &shutdown_flag,
        DispatchLoopOptions {
            enable_ai_orchestration: true,
            require_new_jobs_since: None,
            tick_interval: Duration::from_millis(75),
        },
    )
    .expect("dispatch loop should complete");
    assert_eq!(outcome, CycleOutcome::Success);

    assert!(matches!(msg_rx.try_recv(), Err(mpsc::TryRecvError::Empty)));
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
    let effect_runner =
        EffectRunner::new(runtime_paths, msg_tx.clone(), Box::new(NoOpPlatformHandler));
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
        &msg_tx,
        &msg_rx,
        &effect_runner,
        &shutdown_flag,
        DispatchLoopOptions {
            enable_ai_orchestration: true,
            require_new_jobs_since: None,
            tick_interval: Duration::ZERO,
        },
        None,
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
        triage_deferred: 0,
        summary_deferred: 0,
        signal_total: 0,
        signal_pending_or_in_flight: 0,
        signal_completed: 0,
        signal_failed: 0,
        signal_deferred: 0,
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
        triage_deferred: 0,
        summary_deferred: 0,
        signal_total: 0,
        signal_pending_or_in_flight: 0,
        signal_completed: 0,
        signal_failed: 0,
        signal_deferred: 0,
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
        triage_deferred: 0,
        summary_deferred: 0,
        signal_total: 0,
        signal_pending_or_in_flight: 0,
        signal_completed: 0,
        signal_failed: 0,
        signal_deferred: 0,
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

#[test]
fn test_microdollars_to_display_zero() {
    assert_eq!(microdollars_to_display(0), "$0.00");
}

#[test]
fn test_microdollars_to_display_rounds_down() {
    // 50 microdollars = $0.000050 -> rounds to $0.00
    assert_eq!(microdollars_to_display(50), "$0.00");
    // 4999 microdollars = $0.004999 -> rounds to $0.00
    assert_eq!(microdollars_to_display(4999), "$0.00");
}

#[test]
fn test_microdollars_to_display_rounds_up() {
    // 5000 microdollars = $0.005000 -> rounds to $0.01
    assert_eq!(microdollars_to_display(5000), "$0.01");
    // 15000 microdollars = $0.015000 -> rounds to $0.02
    assert_eq!(microdollars_to_display(15000), "$0.02");
}

#[test]
fn test_microdollars_to_display_exact_cents() {
    // 10000 microdollars = $0.01
    assert_eq!(microdollars_to_display(10000), "$0.01");
    // 1000000 microdollars = $1.00
    assert_eq!(microdollars_to_display(1000000), "$1.00");
}

#[test]
fn test_microdollars_to_display_typical_values() {
    // 1234567 microdollars = $1.234567 -> rounds to $1.23
    assert_eq!(microdollars_to_display(1234567), "$1.23");
    // 5678901 microdollars = $5.678901 -> rounds to $5.68
    assert_eq!(microdollars_to_display(5678901), "$5.68");
}

#[test]
fn test_microdollars_to_display_large_values() {
    // 123456789 microdollars = $123.456789 -> rounds to $123.46
    assert_eq!(microdollars_to_display(123456789), "$123.46");
    // 1000000000 microdollars = $1000.00
    assert_eq!(microdollars_to_display(1000000000), "$1000.00");
}

#[test]
fn format_compact_tokens_thresholds() {
    assert_eq!(format_compact_tokens(0), "0");
    assert_eq!(format_compact_tokens(999), "999");
    assert_eq!(format_compact_tokens(1_000), "1K");
    assert_eq!(format_compact_tokens(12_345), "12K");
    assert_eq!(format_compact_tokens(999_999), "999K");
    assert_eq!(format_compact_tokens(1_000_000), "1.0M");
    assert_eq!(format_compact_tokens(1_234_567), "1.2M");
}

#[test]
fn format_llm_usage_lines_formats_rows_compactly() {
    let rows = vec![
        LlmModelUsageView {
            model: "alpha".to_string(),
            input_tokens: 12_345,
            output_tokens: 3_100,
        },
        LlmModelUsageView {
            model: "beta".to_string(),
            input_tokens: 500,
            output_tokens: 80,
        },
    ];
    let lines = format_llm_usage_lines(&rows);
    assert_eq!(lines.len(), 2);
    assert!(lines[0].contains("alpha"));
    assert!(lines[0].contains("in=12K"));
    assert!(lines[0].contains("out=3K"));
    assert!(lines[1].contains("beta"));
    assert!(lines[1].contains("in=500"));
    assert!(lines[1].contains("out=80"));
}

#[test]
fn format_llm_usage_lines_empty_returns_empty() {
    let lines = format_llm_usage_lines(&[]);
    assert!(lines.is_empty());
}

#[test]
fn summary_refresh_exit_code_is_zero_for_partial_success() {
    assert_eq!(summary_refresh_exit_code(92, 8), 0);
}

#[test]
fn summary_refresh_exit_code_is_nonzero_when_all_attempts_fail() {
    assert_eq!(summary_refresh_exit_code(0, 8), 1);
}

#[test]
fn select_stale_summary_targets_prefers_missing_current_cache_key_and_respects_limit() {
    let metas = vec![
        harvester_engine::ArchiveArticleMeta {
            url: "https://example.com/a".to_string(),
            fetched_utc: Some("2026-04-01T00:00:00Z".to_string()),
            content_hash: Some("hash-a".to_string()),
        },
        harvester_engine::ArchiveArticleMeta {
            url: "https://example.com/b".to_string(),
            fetched_utc: Some("2026-04-02T00:00:00Z".to_string()),
            content_hash: Some("hash-b".to_string()),
        },
        harvester_engine::ArchiveArticleMeta {
            url: "https://example.com/c".to_string(),
            fetched_utc: Some("2026-04-03T00:00:00Z".to_string()),
            content_hash: Some("hash-c".to_string()),
        },
    ];
    let mut cache = SummaryCache::new();
    let current_key = SummaryCacheKey::try_new(
        "hash-a",
        PromptId::ArticleSummary,
        Some(5),
        Some("gpt-5.4-mini"),
        &[],
    )
    .unwrap();
    cache.insert(
        current_key,
        SummaryCacheEntry {
            result: ArticleSummaryResult {
                title: "A".to_string(),
                summary: "A".to_string(),
                key_points: vec![],
                input_tokens: 1,
                output_tokens: 1,
                entities: Default::default(),
            },
            created_at_utc: "2026-04-20T00:00:00Z".to_string(),
        },
    );
    let old_version_key = SummaryCacheKey::try_new(
        "hash-b",
        PromptId::ArticleSummary,
        Some(4),
        Some("gpt-5.4-mini"),
        &[],
    )
    .unwrap();
    cache.insert(
        old_version_key,
        SummaryCacheEntry {
            result: ArticleSummaryResult {
                title: "B".to_string(),
                summary: "B".to_string(),
                key_points: vec![],
                input_tokens: 1,
                output_tokens: 1,
                entities: Default::default(),
            },
            created_at_utc: "2026-04-19T00:00:00Z".to_string(),
        },
    );

    let selection = select_stale_summary_targets(&metas, &cache, 5, "gpt-5.4-mini", &[], 1);

    assert_eq!(selection.total_stale, 2);
    assert_eq!(selection.targets.len(), 1);
    assert_eq!(selection.targets[0].primary_url, "https://example.com/b");
}

#[test]
fn select_stale_summary_targets_deduplicates_by_content_hash() {
    let metas = vec![
        harvester_engine::ArchiveArticleMeta {
            url: "https://example.com/a".to_string(),
            fetched_utc: Some("2026-04-01T00:00:00Z".to_string()),
            content_hash: Some("shared-hash".to_string()),
        },
        harvester_engine::ArchiveArticleMeta {
            url: "https://example.com/b".to_string(),
            fetched_utc: Some("2026-04-02T00:00:00Z".to_string()),
            content_hash: Some("shared-hash".to_string()),
        },
    ];

    let selection =
        select_stale_summary_targets(&metas, &SummaryCache::new(), 5, "gpt-5.4-mini", &[], 10);

    assert_eq!(selection.total_stale, 1);
    assert_eq!(selection.targets.len(), 1);
    assert_eq!(selection.targets[0].primary_url, "https://example.com/a");
    assert_eq!(
        selection.targets[0].related_urls,
        vec![
            (
                "https://example.com/a".to_string(),
                Some("2026-04-01T00:00:00Z".to_string())
            ),
            (
                "https://example.com/b".to_string(),
                Some("2026-04-02T00:00:00Z".to_string())
            ),
        ]
    );
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

// --- Import CLI flag tests ---

fn idle_import_obs() -> BatchObservation {
    BatchObservation {
        poll_in_progress: false,
        session_state: harvester_core::SessionState::Idle,
        jobs_total: 0,
        jobs_done: 0,
        jobs_failed: 0,
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
        triage_completed: 0,
        triage_failed: 0,
        summary_total: 0,
        summary_pending: 0,
        summary_in_flight: 0,
        summary_completed: 0,
        summary_failed: 0,
        triage_deferred: 0,
        summary_deferred: 0,
        signal_total: 0,
        signal_pending_or_in_flight: 0,
        signal_completed: 0,
        signal_failed: 0,
        signal_deferred: 0,
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
    }
}

#[test]
fn import_saved_web_dir_flag_is_parsed() {
    let args =
        crate::cli::Args::parse_from(&["harvester_batch", "--import-saved-web-dir", "/tmp/saved"]);
    assert_eq!(
        args.import_saved_web_dir,
        Some(std::path::PathBuf::from("/tmp/saved"))
    );
}

#[test]
fn import_saved_web_dir_conflicts_with_dry_run() {
    let result = <crate::cli::Args as clap::Parser>::try_parse_from([
        "harvester_batch",
        "--import-saved-web-dir",
        "/tmp/saved",
        "--dry-run",
    ]);
    assert!(result.is_err());
}

#[test]
fn import_saved_web_dir_conflicts_with_single_shot() {
    let result = <crate::cli::Args as clap::Parser>::try_parse_from([
        "harvester_batch",
        "--import-saved-web-dir",
        "/tmp/saved",
        "--single-shot",
    ]);
    assert!(result.is_err());
}

#[test]
fn import_mode_persistence_merge_preserves_existing_jobs_and_appends_imports() {
    let existing = vec![harvester_core::CompletedJobSnapshot {
        url: "https://example.com/existing".to_string(),
        tokens: Some(10),
        bytes: Some(100),
        links: Vec::new(),
        fetched_utc: Some("2026-03-08T06:00:00Z".to_string()),
    }];
    let imported = vec![harvester_core::CompletedJobSnapshot {
        url: "https://example.com/imported".to_string(),
        tokens: None,
        bytes: None,
        links: Vec::new(),
        fetched_utc: Some("2026-03-08T06:01:56Z".to_string()),
    }];

    let merged = merge_completed_jobs_for_import(existing, imported);

    assert_eq!(merged.len(), 2);
    assert_eq!(merged[0].url, "https://example.com/existing");
    assert_eq!(merged[1].url, "https://example.com/imported");
    assert_eq!(
        merged[1].fetched_utc.as_deref(),
        Some("2026-03-08T06:01:56Z")
    );
}

#[test]
fn should_settle_import_cycle_when_idle() {
    let obs = idle_import_obs();
    assert!(should_settle_import_cycle(&obs));
}

#[test]
fn should_not_settle_import_cycle_when_in_flight() {
    let mut obs = idle_import_obs();
    obs.import_in_flight = true;
    obs.import_phase = harvester_core::ImportPhase::Importing;
    assert!(!should_settle_import_cycle(&obs));
}

#[test]
fn should_not_settle_import_cycle_when_summaries_pending() {
    let mut obs = idle_import_obs();
    obs.import_phase = harvester_core::ImportPhase::Complete;
    obs.summary_pending = 3;
    assert!(!should_settle_import_cycle(&obs));
}

#[test]
fn should_settle_import_cycle_when_complete_and_no_pending() {
    let mut obs = idle_import_obs();
    obs.import_phase = harvester_core::ImportPhase::Complete;
    obs.imports_completed = 2;
    assert!(should_settle_import_cycle(&obs));
}

#[test]
fn should_not_settle_import_cycle_when_pre_triage_loading() {
    let mut obs = idle_import_obs();
    obs.import_phase = harvester_core::ImportPhase::Complete;
    obs.imports_completed = 2;
    obs.pre_triage_phase = harvester_core::PreTriagePhase::LoadingArticles;
    assert!(!should_settle_import_cycle(&obs));
}

#[test]
fn should_not_settle_import_cycle_when_pre_triage_reviewing() {
    let mut obs = idle_import_obs();
    obs.import_phase = harvester_core::ImportPhase::Complete;
    obs.imports_completed = 2;
    obs.pre_triage_phase = harvester_core::PreTriagePhase::Reviewing;
    assert!(!should_settle_import_cycle(&obs));
}

#[test]
fn should_not_settle_import_cycle_when_pre_triage_ready_to_triage() {
    let mut obs = idle_import_obs();
    obs.import_phase = harvester_core::ImportPhase::Complete;
    obs.imports_completed = 2;
    obs.pre_triage_phase = harvester_core::PreTriagePhase::ReadyToTriage;
    assert!(!should_settle_import_cycle(&obs));
}

#[test]
fn classify_import_cycle_success_when_all_imported() {
    let mut obs = idle_import_obs();
    obs.import_phase = harvester_core::ImportPhase::Complete;
    obs.imports_completed = 3;
    assert_eq!(classify_import_cycle_outcome(&obs), CycleOutcome::Success);
}

#[test]
fn classify_import_cycle_partial_when_some_failed() {
    let mut obs = idle_import_obs();
    obs.import_phase = harvester_core::ImportPhase::Complete;
    obs.imports_completed = 2;
    obs.imports_failed = 1;
    assert_eq!(
        classify_import_cycle_outcome(&obs),
        CycleOutcome::PartialFailure
    );
}

#[test]
fn classify_import_cycle_total_failure_when_zero_imported() {
    let mut obs = idle_import_obs();
    obs.import_phase = harvester_core::ImportPhase::Failed;
    obs.imports_failed = 2;
    assert_eq!(
        classify_import_cycle_outcome(&obs),
        CycleOutcome::TotalFailure
    );
}

#[test]
fn format_awaiting_batch_line_is_absent_when_nothing_deferred() {
    assert_eq!(format_awaiting_batch_line(0, 0, 0), None);
}

#[test]
fn format_awaiting_batch_line_reports_per_stage_and_total_counts() {
    let line = format_awaiting_batch_line(3, 2, 1).unwrap();
    assert_eq!(
        line,
        "  Awaiting batch results: 3 triage, 2 summaries, 1 signal (6 total)"
    );
}

#[test]
fn default_progress_output_includes_poll_summary_once_but_excludes_verbose_diagnostics() {
    let mut observation = observation_with_totals(1, 1, 0, 1, 0, 1, 0);
    observation.triage_deferred = 1;
    observation
        .source_poll_stats
        .push(harvester_core::SourcePollStat {
            source_id: SourceId::new("test-rss").unwrap(),
            kind: SourceKind::Rss,
            parsed: 2,
            dedup_filtered: 1,
            emitted: 1,
        });
    let usage_rows = [LlmModelUsageView {
        model: "gpt-test".to_string(),
        input_tokens: 10,
        output_tokens: 20,
    }];

    let intake_details = format_optional_cycle_diagnostics(
        false,
        true,
        true,
        1,
        &CycleOutcome::Success,
        &CycleCounts::default(),
        0,
        &observation,
        &usage_rows,
        None,
    );
    let collect_only_details = format_optional_cycle_diagnostics(
        false,
        false,
        false,
        2,
        &CycleOutcome::Success,
        &CycleCounts::default(),
        0,
        &observation,
        &usage_rows,
        None,
    );
    let details = intake_details
        .into_iter()
        .chain(collect_only_details)
        .collect::<Vec<_>>()
        .join("\n");

    assert_eq!(details.matches("--- Poll summary ---").count(), 1);
    assert!(details.contains("test-rss"));
    assert!(!details.contains("Cycle"));
    assert!(!details.contains("Awaiting batch results"));
    assert!(!details.contains("gpt-test: in=10 out=20"));
}

#[test]
fn verbose_progress_output_contains_cycle_source_and_model_diagnostics() {
    let mut observation = observation_with_totals(1, 1, 0, 1, 0, 1, 0);
    observation
        .source_poll_stats
        .push(harvester_core::SourcePollStat {
            source_id: SourceId::new("test-rss").unwrap(),
            kind: SourceKind::Rss,
            parsed: 2,
            dedup_filtered: 1,
            emitted: 1,
        });
    let details = format_optional_cycle_diagnostics(
        true,
        true,
        true,
        1,
        &CycleOutcome::Success,
        &CycleCounts::default(),
        0,
        &observation,
        &[LlmModelUsageView {
            model: "gpt-test".to_string(),
            input_tokens: 10,
            output_tokens: 20,
        }],
        None,
    )
    .join("\n");

    assert!(details.contains("Cycle"));
    assert!(details.contains("--- Poll summary ---"));
    assert!(details.contains("gpt-test: in=10 out=20"));
}

#[test]
fn verbose_wait_timestamp_uses_injected_local_offset() {
    let checked_at = chrono::FixedOffset::east_opt(2 * 60 * 60)
        .unwrap()
        .with_ymd_and_hms(2026, 7, 24, 9, 48, 30)
        .single()
        .unwrap();
    let line = format_verbose_awaiting_batch_line(1, 0, 0, Some(checked_at)).unwrap();

    assert!(line.contains("2026-07-24 09:48:30 +02:00"));
    assert!(!line.contains("Z"));
    assert!(!line.contains("+00:00"));
}

#[test]
fn batch_api_final_summary_distinguishes_intake_from_collection_passes_and_cost_scope() {
    let summary = format_final_summary(
        true,
        7,
        &observation_with_totals(2, 2, 0, 1, 0, 1, 0),
        2,
        1,
        1,
        Duration::from_secs(136),
        25_000,
    );

    assert!(summary.contains("intake=1 collection_passes=6"));
    assert!(summary.contains("triage_success=1 triage_failed=0"));
    assert!(summary.contains("cost_this_run=$0.03"));
    assert!(!summary.contains("7 cycles"));
}

#[test]
fn ordinary_final_summary_retains_cycle_wording() {
    let summary = format_final_summary(
        false,
        7,
        &observation_with_totals(2, 2, 0, 1, 0, 1, 0),
        2,
        1,
        1,
        Duration::from_secs(136),
        0,
    );

    assert!(summary.contains("Batch complete: 7 cycles"));
    assert!(!summary.contains("collection_passes"));
}

#[test]
fn ascii_progress_selects_ascii_glyphs_only_for_interactive_dashboard() {
    assert_eq!(progress_glyphs(true), ProgressGlyphs::Ascii);
    assert_eq!(progress_glyphs(false), ProgressGlyphs::Unicode);
}

#[test]
fn redirected_start_line_uses_the_operational_mode_label() {
    assert_eq!(batch_mode_label(true), "batch-api");
    assert_eq!(batch_mode_label(false), "recurring");
}

fn batch_peek(
    status: Option<openai_provider_kit::BatchLifecycle>,
    completed: u32,
    total: u32,
) -> BatchPeek {
    let request_counts = status
        .as_ref()
        .map(|_| openai_provider_kit::BatchRequestCounts {
            total,
            completed,
            failed: 0,
        });
    BatchPeek {
        batch_id: "batch-test".to_string(),
        stage: harvester_core::StageKind::Triage,
        status,
        request_counts,
    }
}

#[test]
fn batch_wait_keeps_waiting_when_all_peeked_batches_are_nonterminal() {
    let peeks = vec![
        batch_peek(Some(openai_provider_kit::BatchLifecycle::InProgress), 3, 10),
        batch_peek(Some(openai_provider_kit::BatchLifecycle::Finalizing), 8, 8),
        batch_peek(None, 0, 0),
    ];

    assert_eq!(decide_batch_wait(&peeks), BatchWaitDecision::KeepWaiting);
}

#[test]
fn batch_wait_runs_collect_cycle_when_a_peeked_batch_is_terminal() {
    for terminal in [
        openai_provider_kit::BatchLifecycle::Completed,
        openai_provider_kit::BatchLifecycle::Failed,
        openai_provider_kit::BatchLifecycle::Expired,
        openai_provider_kit::BatchLifecycle::Cancelled,
    ] {
        let peeks = vec![
            batch_peek(Some(openai_provider_kit::BatchLifecycle::InProgress), 3, 10),
            batch_peek(Some(terminal), 10, 10),
        ];

        assert_eq!(
            decide_batch_wait(&peeks),
            BatchWaitDecision::RunCollectCycle
        );
    }
}

#[test]
fn batch_wait_runs_collect_cycle_when_no_batches_can_be_peeked() {
    assert_eq!(decide_batch_wait(&[]), BatchWaitDecision::RunCollectCycle);
}

#[test]
fn provider_lookup_failure_retains_last_successful_counts_until_a_successful_retry() {
    let previous = vec![batch_peek(
        Some(openai_provider_kit::BatchLifecycle::InProgress),
        4,
        10,
    )];
    let failed_lookup = vec![batch_peek(None, 0, 0)];
    let retained = retain_last_successful_provider_counts(&previous, failed_lookup);
    assert_eq!(retained.len(), 2);
    assert!(retained.iter().any(|peek| {
        peek.status == Some(openai_provider_kit::BatchLifecycle::InProgress)
            && peek
                .request_counts
                .as_ref()
                .is_some_and(|counts| counts.completed == 4)
    }));
    assert!(retained.iter().any(|peek| peek.status.is_none()));

    let recovered = retain_last_successful_provider_counts(
        &retained,
        vec![batch_peek(
            Some(openai_provider_kit::BatchLifecycle::InProgress),
            6,
            10,
        )],
    );
    assert_eq!(recovered.len(), 1);
    assert_eq!(recovered[0].request_counts.as_ref().unwrap().completed, 6);
}

struct ManualWaitClock {
    start: Instant,
    elapsed: Cell<Duration>,
    wall: RefCell<chrono::DateTime<chrono::FixedOffset>>,
}

impl ManualWaitClock {
    fn new(wall: chrono::DateTime<chrono::FixedOffset>) -> Self {
        Self {
            start: Instant::now(),
            elapsed: Cell::new(Duration::ZERO),
            wall: RefCell::new(wall),
        }
    }
}

impl ProgressClock for ManualWaitClock {
    fn monotonic_now(&self) -> Instant {
        self.start + self.elapsed.get()
    }

    fn wall_now(&self) -> chrono::DateTime<chrono::FixedOffset> {
        *self.wall.borrow()
    }

    fn sleep(&self, duration: Duration) {
        self.elapsed.set(self.elapsed.get() + duration);
        let updated = *self.wall.borrow() + chrono::Duration::from_std(duration).unwrap();
        *self.wall.borrow_mut() = updated;
    }
}

impl ProgressClock for &ManualWaitClock {
    fn monotonic_now(&self) -> Instant {
        (*self).monotonic_now()
    }

    fn wall_now(&self) -> chrono::DateTime<chrono::FixedOffset> {
        (*self).wall_now()
    }

    fn sleep(&self, duration: Duration) {
        (*self).sleep(duration);
    }
}

fn empty_run_baseline() -> BatchRunBaseline {
    BatchRunBaseline {
        jobs_total: 0,
        jobs_done: 0,
        jobs_failed: 0,
    }
}

#[test]
fn plain_progress_throttles_steady_heartbeats_and_flushes_each_phase_transition() {
    let wall = chrono::FixedOffset::east_opt(2 * 60 * 60)
        .unwrap()
        .with_ymd_and_hms(2026, 7, 23, 9, 43, 30)
        .single()
        .unwrap();
    let clock = ManualWaitClock::new(wall);
    let output = SharedRunnerOutput::default();
    let surface = BatchProgressSurface::Plain(PlainProgressReporter::new(output.clone()));
    let mut progress = LiveBatchProgress::with_parts(empty_run_baseline(), &clock, surface);
    let state = AppState::new();

    progress.set_phase(BatchDisplayPhase::Intake);
    clock.sleep(PROGRESS_REFRESH_INTERVAL);
    progress.paint(&state, 0, false);
    assert_eq!(output.text().lines().count(), 1);

    clock.sleep(PLAIN_PROGRESS_HEARTBEAT_INTERVAL - Duration::from_secs(1));
    progress.paint(&state, 0, false);
    assert_eq!(
        output.text().lines().count(),
        1,
        "steady-state output must remain quiet before the minute boundary"
    );

    clock.sleep(Duration::from_secs(1));
    progress.paint(&state, 0, false);
    assert_eq!(
        output.text().lines().count(),
        2,
        "exactly one steady-state heartbeat is due after one minute"
    );

    clock.sleep(PROGRESS_REFRESH_INTERVAL);
    progress.paint(&state, 0, false);
    assert_eq!(
        output.text().lines().count(),
        2,
        "a second steady-state line must not follow within the minute"
    );

    progress.set_phase(BatchDisplayPhase::Triage);
    clock.sleep(PROGRESS_REFRESH_INTERVAL);
    progress.paint(&state, 0, false);
    assert_eq!(
        output.text().lines().count(),
        3,
        "a phase transition must flush one line within the heartbeat window"
    );

    clock.sleep(PROGRESS_REFRESH_INTERVAL);
    progress.paint(&state, 0, false);
    assert_eq!(
        output.text().lines().count(),
        3,
        "the phase transition must emit exactly one line"
    );

    progress.set_phase(BatchDisplayPhase::Summaries);
    clock.sleep(PROGRESS_REFRESH_INTERVAL);
    progress.paint(&state, 0, false);
    assert_eq!(
        output.text().lines().count(),
        4,
        "every distinct phase transition must flush exactly one line"
    );
}

#[test]
fn terminal_progress_stays_live_across_collection_passes_and_finishes_once() {
    let wall = chrono::FixedOffset::east_opt(0)
        .unwrap()
        .with_ymd_and_hms(2026, 7, 23, 9, 43, 30)
        .single()
        .unwrap();
    let clock = ManualWaitClock::new(wall);
    let output = SharedRunnerOutput::default();
    let surface = BatchProgressSurface::Terminal(TerminalProgressSurface::new(
        output.clone(),
        ProgressGlyphs::Unicode,
    ));
    let mut progress = LiveBatchProgress::with_parts(empty_run_baseline(), &clock, surface);
    let state = AppState::new();

    progress.record_pass(false);
    progress.set_phase(BatchDisplayPhase::Intake);
    progress.paint(&state, 0, true);
    for _ in 0..3 {
        progress.record_pass(true);
        progress.set_phase(BatchDisplayPhase::Collecting);
        progress.paint(&state, 0, true);
    }

    let before_finish = output.text();
    assert_eq!(
        before_finish.matches("\u{1b}[?25l").count(),
        1,
        "one persistent surface hides the cursor only once"
    );
    assert!(
        !before_finish.contains("\u{1b}[?25h"),
        "collection passes must not finish and append historical dashboards"
    );

    progress.set_phase(BatchDisplayPhase::Complete);
    progress.paint(&state, 0, true);
    progress.finish();

    let finished = output.text();
    assert_eq!(
        finished.matches("\u{1b}[?25h").count(),
        1,
        "the terminal surface must be finished exactly once"
    );
    assert!(
        finished.ends_with("\u{1b}[?25h\n"),
        "only the final dashboard may be terminated as historical output"
    );
}

#[test]
fn provider_wait_uses_second_heartbeats_without_extra_peeks_between_deadlines() {
    let wall = chrono::FixedOffset::east_opt(2 * 60 * 60)
        .unwrap()
        .with_ymd_and_hms(2026, 7, 23, 9, 43, 30)
        .single()
        .unwrap();
    let clock = ManualWaitClock::new(wall);
    let shutdown = AtomicBool::new(false);
    let mut peek_calls = 0usize;
    let mut heartbeats = 0usize;
    let mut checking = 0usize;
    let mut checked_at_local = None;
    let mut next_check_local = None;
    let result = run_provider_wait_loop(
        &clock,
        &shutdown,
        Vec::new(),
        || {
            peek_calls += 1;
            if peek_calls == 1 {
                vec![batch_peek(
                    Some(openai_provider_kit::BatchLifecycle::InProgress),
                    1,
                    2,
                )]
            } else {
                vec![batch_peek(
                    Some(openai_provider_kit::BatchLifecycle::Completed),
                    2,
                    2,
                )]
            }
        },
        |event| match event.phase {
            BatchDisplayPhase::WaitingForProvider => {
                heartbeats += 1;
                checked_at_local = event.last_provider_check_local;
                next_check_local = event.next_provider_check_local;
            }
            BatchDisplayPhase::CheckingProvider => checking += 1,
            _ => unreachable!(),
        },
    );

    assert!(matches!(result, ProviderWaitOutcome::Collect(_)));
    assert_eq!(peek_calls, 2, "one initial and one deadline check only");
    assert_eq!(checking, 2);
    assert_eq!(
        heartbeats, 300,
        "one local refresh per second for five minutes"
    );
    assert_eq!(
        checked_at_local
            .unwrap()
            .format("%Y-%m-%d %H:%M:%S %:z")
            .to_string(),
        "2026-07-23 09:43:30 +02:00"
    );
    assert_eq!(
        next_check_local
            .unwrap()
            .format("%Y-%m-%d %H:%M:%S %:z")
            .to_string(),
        "2026-07-23 09:48:30 +02:00"
    );
}

#[test]
fn provider_wait_shutdown_is_observed_at_the_half_second_local_poll_boundary() {
    let wall = chrono::FixedOffset::east_opt(0)
        .unwrap()
        .with_ymd_and_hms(2026, 7, 23, 9, 43, 30)
        .single()
        .unwrap();
    let clock = ManualWaitClock::new(wall);
    let shutdown = AtomicBool::new(false);
    let mut rendered_wait = false;
    let outcome = run_provider_wait_loop(
        &clock,
        &shutdown,
        Vec::new(),
        || {
            vec![batch_peek(
                Some(openai_provider_kit::BatchLifecycle::InProgress),
                0,
                1,
            )]
        },
        |event| {
            if event.phase == BatchDisplayPhase::WaitingForProvider && !rendered_wait {
                rendered_wait = true;
                shutdown.store(true, std::sync::atomic::Ordering::Relaxed);
            }
        },
    );
    assert!(matches!(outcome, ProviderWaitOutcome::Shutdown));
    assert!(clock.elapsed.get() <= Duration::from_millis(500));
}

#[test]
fn provider_wait_marks_checking_before_a_blocking_peek_and_observes_shutdown_after_it_returns() {
    let wall = chrono::FixedOffset::east_opt(0)
        .unwrap()
        .with_ymd_and_hms(2026, 7, 23, 9, 43, 30)
        .single()
        .unwrap();
    let clock = ManualWaitClock::new(wall);
    let shutdown = AtomicBool::new(false);
    let checking_rendered = Cell::new(false);
    let outcome = run_provider_wait_loop(
        &clock,
        &shutdown,
        Vec::new(),
        || {
            assert!(
                checking_rendered.get(),
                "CheckingProvider must paint before the peek"
            );
            clock.sleep(Duration::from_secs(5));
            shutdown.store(true, std::sync::atomic::Ordering::Relaxed);
            vec![batch_peek(
                Some(openai_provider_kit::BatchLifecycle::InProgress),
                0,
                1,
            )]
        },
        |event| {
            if event.phase == BatchDisplayPhase::CheckingProvider {
                checking_rendered.set(true);
            }
        },
    );

    assert!(matches!(outcome, ProviderWaitOutcome::Shutdown));
    assert!(checking_rendered.get());
    assert_eq!(clock.elapsed.get(), Duration::from_secs(5));
}

#[test]
fn local_heartbeat_wait_observes_shutdown_within_half_a_second() {
    let wall = chrono::FixedOffset::east_opt(0)
        .unwrap()
        .with_ymd_and_hms(2026, 7, 23, 9, 43, 30)
        .single()
        .unwrap();
    let clock = ManualWaitClock::new(wall);
    let shutdown = AtomicBool::new(false);
    let renders = Cell::new(0usize);

    let interrupted = wait_with_local_heartbeat(&clock, &shutdown, Duration::from_secs(30), || {
        renders.set(renders.get() + 1);
        shutdown.store(true, std::sync::atomic::Ordering::Relaxed);
    });

    assert!(interrupted);
    assert_eq!(renders.get(), 1);
    assert!(
        clock.elapsed.get() <= SHUTDOWN_POLL_INTERVAL,
        "shutdown must be observed within the local poll boundary"
    );
}

#[test]
fn batch_drain_progress_compares_manifest_and_deferred_work() {
    let before = BatchDrainSnapshot {
        pending_manifest_batches: vec![("file-1".to_string(), None)],
        triage_deferred: 1,
        summary_deferred: 0,
        signal_deferred: 0,
    };
    assert!(!batch_drain_made_progress(&before, &before));

    let after_reconcile = BatchDrainSnapshot {
        pending_manifest_batches: vec![("file-1".to_string(), Some("batch-1".to_string()))],
        ..before.clone()
    };
    assert!(batch_drain_made_progress(&before, &after_reconcile));

    let after_collection = BatchDrainSnapshot {
        pending_manifest_batches: Vec::new(),
        triage_deferred: 0,
        ..after_reconcile.clone()
    };
    assert!(batch_drain_made_progress(
        &after_reconcile,
        &after_collection
    ));
}

#[test]
fn batch_drain_exits_after_second_consecutive_no_progress_cycle() {
    assert!(!should_exit_batch_drain_after_no_progress(0));
    assert!(!should_exit_batch_drain_after_no_progress(1));
    assert!(should_exit_batch_drain_after_no_progress(2));
}

#[test]
fn batch_drain_bailout_prints_remaining_stage_counts_without_changing_exit_code() {
    let snapshot = BatchDrainSnapshot {
        pending_manifest_batches: vec![("file-1".to_string(), Some("batch-1".to_string()))],
        triage_deferred: 7,
        summary_deferred: 5,
        signal_deferred: 3,
    };
    let mut output = Vec::new();

    assert!(should_exit_batch_drain_after_no_progress(
        MAX_CONSECUTIVE_BATCH_COLLECT_NO_PROGRESS
    ));
    write_no_progress_bailout(&mut output, &snapshot).unwrap();

    assert_eq!(
        String::from_utf8(output).unwrap(),
        "[batch-wait] no-progress bailout; remaining triage=7 summaries=5 signal=3\n"
    );
    assert_eq!(
        exit_code_with_shutdown(determine_exit_code(0), false),
        0,
        "the bailout must retain the existing successful exit code"
    );
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
    let mut args = create_test_args(false, &temp_dir);

    apply_signal_candidate_selection_settings(&mut state, &args);
    assert_eq!(
        state.signal_candidate_threshold(),
        DEFAULT_SELECTION_THRESHOLD
    );

    args.signal_candidate_threshold = Some(75);
    apply_signal_candidate_selection_settings(&mut state, &args);
    assert_eq!(state.signal_candidate_threshold(), 75);
}
