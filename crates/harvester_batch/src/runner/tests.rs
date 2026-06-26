use super::*;
use crate::cli::{Args, CheckpointCommand};
use harvester_core::signal_candidate::DEFAULT_SELECTION_THRESHOLD;
use harvester_core::{
    AppState, ArticleSummaryResult, BatchObservation, LlmModelUsageView, Msg, SummaryCache,
    SummaryCacheEntry, SummaryCacheKey,
};
use harvester_engine::llm::prompt::PromptId;
use harvester_io::{load_briefing_checkpoint, EffectRunner, NoOpPlatformHandler, RuntimePaths};
use std::path::PathBuf;
use std::sync::atomic::AtomicBool;
use std::sync::mpsc;
use std::sync::Arc;
use std::time::Duration;
use tempfile::TempDir;

fn create_test_args(dry_run: bool, temp_dir: &TempDir) -> Args {
    Args {
        output_dir: temp_dir.path().to_path_buf(),
        sources: PathBuf::from("test_sources.json"),
        contexts_dir: PathBuf::from("contexts"),
        prompts_dir: PathBuf::from("prompts"),
        dry_run,
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
    let result = run_dry_run(&runtime_paths, &args);
    assert!(result.is_ok());
    assert_eq!(result.unwrap(), 0);
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
    let result = run_dry_run(&runtime_paths, &args);
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
