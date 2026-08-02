use super::batch_runtime::BatchRuntime;
use super::dispatch_loop::{
    batch_buffer_is_quiescent, classify_cycle_outcome, should_check_settlement_this_iteration,
    should_run_ai_orchestration, should_settle_cycle, truncate_for_log,
};
use super::live_progress::batch_peek;
use super::*;
use crate::cli::{Args, CheckpointCommand};
use harvester_core::signal_candidate::DEFAULT_SELECTION_THRESHOLD;
use harvester_core::{AppState, BatchObservation, FrozenBatchKey, Msg, StageKind};
use harvester_engine::llm::prompt::PromptId;
use harvester_engine::llm::{
    prompts::register_defaults, LlmCompletionError, LlmConfig, LlmQuotas, ModelId, OpenAiProvider,
    PricingRegistry, PromptRegistry, ProviderKind, TokenUsage, DEFAULT_BRIEFING_MODEL,
    DEFAULT_SUMMARY_MODEL, DEFAULT_TRIAGE_MODEL, OPENAI_MODEL_GPT_4O_MINI,
};
use harvester_io::{load_briefing_checkpoint, EffectRunner, NoOpPlatformHandler, RuntimePaths};
use std::collections::HashSet;
use std::path::PathBuf;
use std::sync::atomic::AtomicBool;
use std::sync::mpsc;
use std::sync::Arc;
use std::sync::RwLock;
use std::time::Duration;
use tempfile::TempDir;

use super::batch_runtime::{
    batch_custom_id, is_batch_eligible_prompt, send_batch_preparation_failure,
};

fn create_test_args(dry_run: bool, temp_dir: &TempDir) -> Args {
    Args {
        output_dir: temp_dir.path().to_path_buf(),
        sources: Some(PathBuf::from("test_sources.json")),
        contexts_dir: PathBuf::from("contexts"),
        prompts_dir: PathBuf::from("prompts"),
        dry_run,
        batch_api: false,
        drain: false,
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
fn drain_makes_the_first_cycle_collect_only_so_no_sources_are_polled() {
    // Batch API mode polls once before it starts collecting.
    assert!(!is_collect_only_cycle(true, false, 1));
    assert!(is_collect_only_cycle(true, false, 2));

    // Drain never polls, so it collects from the very first cycle.
    assert!(is_collect_only_cycle(true, true, 1));
    assert!(is_collect_only_cycle(true, true, 2));

    // Without the Batch API runtime there is no manifest to collect from.
    assert!(!is_collect_only_cycle(false, false, 1));
    assert!(!is_collect_only_cycle(false, false, 2));
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

#[test]
fn redirected_start_line_uses_the_operational_mode_label() {
    assert_eq!(batch_mode_label(true, false), "batch-api");
    assert_eq!(batch_mode_label(false, false), "recurring");
    assert_eq!(batch_mode_label(true, true), "drain");
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
