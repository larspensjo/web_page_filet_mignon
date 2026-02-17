use crate::cli::Args;
use crate::lock;
use engine_logging::{engine_info, engine_warn};
use harvester_core::{update, AppState, BatchObservation, Msg};
use harvester_io::{
    load_completed_jobs, load_sources, persist_completed_jobs, EffectRunner, NoOpPlatformHandler,
    RuntimePaths,
};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc;
use std::sync::Arc;
use std::time::Duration;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum CycleOutcome {
    Success,
    PartialFailure,
    TotalFailure,
}

/// Determines if the batch cycle should settle (all work done or failed).
fn should_settle_cycle(obs: &BatchObservation) -> bool {
    // Settled when:
    // 1. No poll in progress
    // 2. Triage is either idle, complete, or failed (not active)
    // 3. No jobs in flight
    // 4. No triage work in flight
    !obs.poll_in_progress
        && !matches!(
            obs.triage_phase,
            harvester_core::TriagePhase::LoadingArticles | harvester_core::TriagePhase::Triaging
        )
        && obs.jobs_in_flight == 0
        && obs.triage_in_flight == 0
}

/// Classifies the outcome of a completed cycle based on observation metrics.
fn classify_cycle_outcome(obs: &BatchObservation) -> CycleOutcome {
    let has_failures = obs.jobs_failed > 0 || obs.triage_failed > 0;
    let has_successes = obs.jobs_done > 0 || obs.triage_completed > 0;

    match (has_successes, has_failures) {
        (true, false) => CycleOutcome::Success,
        (true, true) => CycleOutcome::PartialFailure,
        (false, true) => CycleOutcome::TotalFailure,
        (false, false) => CycleOutcome::Success, // Nothing to do is success
    }
}

/// Run the batch orchestration loop.
///
/// Executes repeated poll cycles until shutdown signal received or error occurs.
/// Returns exit code: 0 (success), 1 (partial failure), or 2 (fatal error via Err).
///
/// # Arguments
/// * `args` - Parsed command-line arguments specifying paths, intervals, and flags
///
/// # Behavior
/// - Acquires exclusive lock on output directory
/// - Polls sources at configured intervals
/// - Persists state after each cycle
/// - Handles SIGINT/SIGTERM gracefully
/// - Dry-run mode: single poll, read-only, no persistence
pub fn run(args: Args) -> Result<i32, String> {
    engine_info!("[batch] Initializing runtime paths");

    let paths = RuntimePaths::new(
        args.output_dir.clone(),
        args.sources.clone(),
        args.contexts_dir.clone(),
        args.prompts_dir.clone(),
    );

    engine_info!("[batch] Acquiring lock");
    let _lock_guard = lock::acquire_lock(&paths.output_dir, args.force_unlock)?;

    if args.dry_run {
        engine_info!("[batch] Dry-run mode: single poll only");
        return run_dry_run(&paths, &args);
    }

    // Validate source configuration
    engine_info!(
        "[batch] Loading source registry from {:?}",
        paths.sources_path
    );
    let source_registry = load_sources(&paths.sources_path);

    if !args.allow_unsupported_sources {
        let unsupported: Vec<_> = source_registry
            .sources
            .iter()
            .filter_map(|s| match &s.source_type {
                harvester_engine::SourceType::Script { .. } => Some(s.id.to_string()),
                _ => None,
            })
            .collect();

        if !unsupported.is_empty() {
            return Err(format!(
                "Unsupported source types detected: {:?}. Use --allow-unsupported-sources to override.",
                unsupported
            ));
        }
    } else {
        let unsupported_count = source_registry
            .sources
            .iter()
            .filter(|s| matches!(&s.source_type, harvester_engine::SourceType::Script { .. }))
            .count();
        if unsupported_count > 0 {
            engine_warn!(
                "[batch] Running with {} unsupported source(s) (Script type)",
                unsupported_count
            );
        }
    }

    // Create message channel
    let (msg_tx, msg_rx) = mpsc::channel::<Msg>();

    // Hydrate state
    engine_info!("[batch] Hydrating state from disk");
    let mut state = AppState::new();
    state.set_triage_max_in_flight(args.llm_concurrency);
    state.set_summary_max_in_flight(args.llm_concurrency);

    // Restore completed jobs
    let completed_jobs = load_completed_jobs(&paths.state_path);
    if !completed_jobs.is_empty() {
        engine_info!("[batch] Restoring {} completed jobs", completed_jobs.len());
        let (new_state, effects) = update(state, Msg::RestoreCompletedJobs(completed_jobs));
        state = new_state;
        // Effects from restore are cache loads which will be executed shortly
        for effect in effects {
            engine_info!("[batch] Restore effect queued: {:?}", effect);
        }
    } else {
        engine_info!("[batch] No previous state found, starting fresh");
    }

    // Build EffectRunner (without LLM for now - TODO: add LLM support)
    engine_info!("[batch] Building EffectRunner");
    let platform_handler = Box::new(NoOpPlatformHandler);
    let effect_runner = EffectRunner::new(paths.clone(), msg_tx.clone(), platform_handler);

    // Install signal handler for graceful shutdown
    let shutdown_flag = Arc::new(AtomicBool::new(false));
    install_signal_handler(Arc::clone(&shutdown_flag));

    // Outer cycle loop - poll repeatedly until shutdown
    let poll_interval = Duration::from_secs((args.poll_interval * 60) as u64);
    let mut cycle_count = 0;
    let mut total_cycles = 0;
    let mut successful_cycles = 0;
    let mut partial_failure_cycles = 0;
    let mut total_failure_cycles = 0;

    loop {
        cycle_count += 1;
        total_cycles += 1;
        engine_info!("[batch] === Starting cycle {} ===", cycle_count);

        // Start the cycle by dispatching poll
        engine_info!("[batch] Dispatching poll sources");
        msg_tx
            .send(Msg::PollSourcesClicked)
            .map_err(|e| format!("Failed to dispatch poll: {}", e))?;

        // Run dispatch loop until settled
        let outcome = run_dispatch_loop(&mut state, &msg_rx, &effect_runner, &shutdown_flag)?;

        // Track outcome statistics
        match outcome {
            CycleOutcome::Success => successful_cycles += 1,
            CycleOutcome::PartialFailure => partial_failure_cycles += 1,
            CycleOutcome::TotalFailure => total_failure_cycles += 1,
        }

        // Print cycle summary
        let obs = state.batch_observation();
        print_cycle_summary(cycle_count, &outcome, &obs);

        // Persist state
        engine_info!("[batch] Persisting state");
        let completed_jobs = state.completed_jobs_snapshot();
        persist_completed_jobs(&paths.state_path, &completed_jobs);

        // Check for shutdown signal
        if shutdown_flag.load(Ordering::Relaxed) {
            engine_info!("[batch] Shutdown signal received, exiting");
            break;
        }

        // Sleep interruptibly before next cycle
        engine_info!(
            "[batch] Sleeping for {} minutes before next cycle",
            args.poll_interval
        );
        if sleep_interruptible(poll_interval, &shutdown_flag) {
            engine_info!("[batch] Shutdown during sleep, exiting");
            break;
        }
    }

    // Graceful shutdown
    engine_info!("[batch] Graceful shutdown: draining effects and persisting final state");
    drop(effect_runner);
    drop(msg_rx);

    let completed_jobs = state.completed_jobs_snapshot();
    persist_completed_jobs(&paths.state_path, &completed_jobs);

    // Print final summary
    print_final_summary(
        total_cycles,
        successful_cycles,
        partial_failure_cycles,
        total_failure_cycles,
    );

    engine_info!("[batch] Shutdown complete");

    // Determine exit code based on outcomes
    let exit_code = if partial_failure_cycles > 0 || total_failure_cycles > 0 {
        1 // Partial: work completed with some failures
    } else {
        0 // Success: all cycles successful
    };

    Ok(exit_code)
}

/// Runs the inner dispatch loop until settlement or error.
/// Processes messages, updates state, executes effects, and checks for settlement.
fn run_dispatch_loop(
    state: &mut AppState,
    msg_rx: &mpsc::Receiver<Msg>,
    effect_runner: &EffectRunner,
    shutdown_flag: &Arc<AtomicBool>,
) -> Result<CycleOutcome, String> {
    let timeout = Duration::from_millis(100);
    let mut iterations = 0;
    const MAX_ITERATIONS: usize = 10_000; // Safety limit

    loop {
        iterations += 1;
        if iterations > MAX_ITERATIONS {
            return Err(format!(
                "Dispatch loop exceeded maximum iterations ({})",
                MAX_ITERATIONS
            ));
        }

        // Check for shutdown signal
        if shutdown_flag.load(Ordering::Relaxed) {
            engine_info!("[batch] Shutdown signal detected in dispatch loop");
            let obs = state.batch_observation();
            return Ok(classify_cycle_outcome(&obs));
        }

        // Check for settlement
        let obs = state.batch_observation();
        if should_settle_cycle(&obs) {
            engine_info!(
                "[batch] Cycle settled after {} iterations: jobs={}/{}, triage={}/{}",
                iterations,
                obs.jobs_done,
                obs.jobs_total,
                obs.triage_completed,
                obs.triage_total
            );
            return Ok(classify_cycle_outcome(&obs));
        }

        // Receive message with timeout
        match msg_rx.recv_timeout(timeout) {
            Ok(msg) => {
                engine_info!("[batch] Processing message: {:?}", msg);

                // Update state
                let (new_state, effects) = update(state.clone(), msg);
                *state = new_state;

                // Execute effects
                if !effects.is_empty() {
                    engine_info!("[batch] Enqueuing {} effects", effects.len());
                    effect_runner.enqueue(effects);
                }
            }
            Err(mpsc::RecvTimeoutError::Timeout) => {
                // No message available, continue loop
                continue;
            }
            Err(mpsc::RecvTimeoutError::Disconnected) => {
                return Err("Message channel disconnected unexpectedly".to_string());
            }
        }
    }
}

/// Prints a summary of the completed cycle.
fn print_cycle_summary(cycle: usize, outcome: &CycleOutcome, obs: &BatchObservation) {
    println!("\n=== Cycle {} Summary ===", cycle);
    println!("Outcome: {:?}", outcome);
    println!(
        "Jobs: {} total, {} done, {} failed, {} in-flight",
        obs.jobs_total, obs.jobs_done, obs.jobs_failed, obs.jobs_in_flight
    );
    println!(
        "Triage: {} total, {} completed, {} failed, {} pending",
        obs.triage_total, obs.triage_completed, obs.triage_failed, obs.triage_pending
    );
    println!("========================\n");
}

/// Prints the final summary when batch runner exits.
fn print_final_summary(
    total_cycles: usize,
    successful: usize,
    partial_failures: usize,
    total_failures: usize,
) {
    println!("\n╔═══════════════════════════════════════╗");
    println!("║        BATCH RUN FINAL SUMMARY        ║");
    println!("╚═══════════════════════════════════════╝");
    println!("Total cycles:      {}", total_cycles);
    println!("  Successful:      {}", successful);
    println!("  Partial failure: {}", partial_failures);
    println!("  Total failure:   {}", total_failures);
    println!("═══════════════════════════════════════\n");
}

/// Sleeps for the specified duration, checking shutdown flag periodically.
/// Returns true if shutdown was requested during sleep.
fn sleep_interruptible(duration: Duration, shutdown_flag: &Arc<AtomicBool>) -> bool {
    let check_interval = Duration::from_millis(500);
    let mut remaining = duration;

    while remaining > Duration::ZERO {
        if shutdown_flag.load(Ordering::Relaxed) {
            return true;
        }

        let sleep_time = remaining.min(check_interval);
        std::thread::sleep(sleep_time);
        remaining = remaining.saturating_sub(sleep_time);
    }

    false
}

/// Installs a signal handler for SIGINT/SIGTERM to set the shutdown flag.
fn install_signal_handler(shutdown_flag: Arc<AtomicBool>) {
    #[cfg(unix)]
    {
        use std::sync::Mutex;
        static HANDLER_INSTALLED: Mutex<bool> = Mutex::new(false);

        let mut installed = HANDLER_INSTALLED.lock().unwrap();
        if *installed {
            return;
        }

        ctrlc::set_handler(move || {
            engine_info!("[batch] Received shutdown signal (SIGINT/SIGTERM)");
            shutdown_flag.store(true, Ordering::Relaxed);
        })
        .expect("Error setting signal handler");

        *installed = true;
    }

    #[cfg(windows)]
    {
        ctrlc::set_handler(move || {
            engine_info!("[batch] Received shutdown signal (Ctrl-C)");
            shutdown_flag.store(true, Ordering::Relaxed);
        })
        .expect("Error setting signal handler");
    }
}

/// Converts microdollars to a human-readable dollar string with exact rounding.
/// Examples: 0 -> "$0.00", 1234567 -> "$1.23", 50 -> "$0.00", 5000 -> "$0.01"
#[cfg(test)]
fn microdollars_to_display(microdollars: u64) -> String {
    let cents = (microdollars + 5000) / 10000; // Round to nearest cent
    let dollars = cents / 100;
    let remaining_cents = cents % 100;
    format!("${}.{:02}", dollars, remaining_cents)
}

fn run_dry_run(paths: &RuntimePaths, args: &Args) -> Result<i32, String> {
    engine_info!("[dry-run] Starting dry-run mode: single poll, no downloads/triage");

    // Hydrate state (read-only)
    engine_info!(
        "[dry-run] Loading completed jobs from {:?}",
        paths.state_path
    );
    let completed_jobs = load_completed_jobs(&paths.state_path);
    engine_info!("[dry-run] Loaded {} completed jobs", completed_jobs.len());

    // Initialize state
    let (msg_tx, msg_rx) = mpsc::channel();
    let mut state = AppState::new();
    state.set_triage_max_in_flight(args.llm_concurrency);
    state.set_summary_max_in_flight(args.llm_concurrency);

    // Restore completed jobs
    if !completed_jobs.is_empty() {
        let restore_msg = Msg::RestoreCompletedJobs(completed_jobs);
        let (new_state, _effects) = update(state, restore_msg);
        state = new_state;
    }

    // Create effect runner
    let platform_handler = Box::new(NoOpPlatformHandler);
    let effect_runner = EffectRunner::new(paths.clone(), msg_tx.clone(), platform_handler);

    // Dispatch poll
    engine_info!("[dry-run] Dispatching poll");
    msg_tx
        .send(Msg::PollSourcesClicked)
        .map_err(|e| format!("Failed to send poll message: {}", e))?;

    // Run dispatch loop until settlement (read-only, no signal handling needed)
    let shutdown_flag = Arc::new(AtomicBool::new(false));
    let outcome = run_dispatch_loop(&mut state, &msg_rx, &effect_runner, &shutdown_flag)?;

    // Print summary
    let obs = state.batch_observation();
    println!("\n=== Dry-Run Summary ===");
    println!("Outcome: {:?}", outcome);
    println!(
        "Jobs: {} total, {} done, {} failed",
        obs.jobs_total, obs.jobs_done, obs.jobs_failed
    );
    println!(
        "Triage: {} total, {} completed, {} failed, {} pending",
        obs.triage_total, obs.triage_completed, obs.triage_failed, obs.triage_pending
    );
    println!("Session state: {:?}", obs.session_state);
    println!("======================\n");

    engine_info!("[dry-run] Dry-run complete (no state modifications)");
    Ok(0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;
    use tempfile::TempDir;

    fn create_test_args(dry_run: bool, temp_dir: &TempDir) -> Args {
        Args {
            output_dir: temp_dir.path().to_path_buf(),
            sources: PathBuf::from("test_sources.json"),
            contexts_dir: PathBuf::from("contexts"),
            prompts_dir: PathBuf::from("prompts"),
            dry_run,
            allow_unsupported_sources: false,
            llm_concurrency: 1,
            poll_interval: 1,
            force_unlock: false,
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
    fn test_should_settle_cycle_when_idle() {
        let obs = BatchObservation {
            poll_in_progress: false,
            session_state: harvester_core::SessionState::Idle,
            jobs_total: 0,
            jobs_done: 0,
            jobs_failed: 0,
            jobs_in_flight: 0,
            pre_triage_phase: harvester_core::PreTriagePhase::Idle,
            triage_phase: harvester_core::TriagePhase::Idle,
            triage_total: 0,
            triage_pending: 0,
            triage_in_flight: 0,
            triage_completed: 0,
            triage_failed: 0,
        };

        assert!(should_settle_cycle(&obs));
    }

    #[test]
    fn test_should_not_settle_when_poll_in_progress() {
        let obs = BatchObservation {
            poll_in_progress: true,
            session_state: harvester_core::SessionState::Running,
            jobs_total: 0,
            jobs_done: 0,
            jobs_failed: 0,
            jobs_in_flight: 0,
            pre_triage_phase: harvester_core::PreTriagePhase::Idle,
            triage_phase: harvester_core::TriagePhase::Idle,
            triage_total: 0,
            triage_pending: 0,
            triage_in_flight: 0,
            triage_completed: 0,
            triage_failed: 0,
        };

        assert!(!should_settle_cycle(&obs));
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
            triage_phase: harvester_core::TriagePhase::Complete,
            triage_total: 5,
            triage_pending: 0,
            triage_in_flight: 0,
            triage_completed: 5,
            triage_failed: 0,
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
            triage_phase: harvester_core::TriagePhase::Complete,
            triage_total: 5,
            triage_pending: 0,
            triage_in_flight: 0,
            triage_completed: 3,
            triage_failed: 2,
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
            triage_phase: harvester_core::TriagePhase::Complete,
            triage_total: 5,
            triage_pending: 0,
            triage_in_flight: 0,
            triage_completed: 0,
            triage_failed: 5,
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
}
