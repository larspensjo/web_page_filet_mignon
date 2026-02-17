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

/// Run the batch orchestration loop
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

    loop {
        cycle_count += 1;
        engine_info!("[batch] === Starting cycle {} ===", cycle_count);

        // Start the cycle by dispatching poll
        engine_info!("[batch] Dispatching poll sources");
        msg_tx
            .send(Msg::PollSourcesClicked)
            .map_err(|e| format!("Failed to dispatch poll: {}", e))?;

        // Run dispatch loop until settled
        let outcome = run_dispatch_loop(&mut state, &msg_rx, &effect_runner, &shutdown_flag)?;

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
    engine_info!("[batch] Shutdown complete");

    Ok(0)
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

fn run_dry_run(_paths: &RuntimePaths, _args: &Args) -> Result<i32, String> {
    // TODO: Implement dry-run mode
    println!("[dry-run] Not yet implemented");
    Ok(0)
}
