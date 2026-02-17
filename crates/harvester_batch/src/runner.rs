use crate::cli::Args;
use crate::lock;
use engine_logging::{engine_info, engine_warn};
use harvester_core::{update, AppState, BatchObservation, Msg};
use harvester_io::{
    load_completed_jobs, load_sources, EffectRunner, NoOpPlatformHandler, RuntimePaths,
};
use std::sync::mpsc;
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
    engine_info!("[batch] Loading source registry from {:?}", paths.sources_path);
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

    // TODO: Implement outer cycle loop (D3)
    // For now, run a single dispatch cycle
    engine_info!("[batch] Running single dispatch cycle");
    let outcome = run_dispatch_loop(&mut state, &msg_rx, &effect_runner)?;
    engine_info!("[batch] Cycle complete with outcome: {:?}", outcome);

    // Clean shutdown
    drop(effect_runner);
    drop(msg_rx);

    Ok(0)
}

/// Runs the inner dispatch loop until settlement or error.
/// Processes messages, updates state, executes effects, and checks for settlement.
fn run_dispatch_loop(
    state: &mut AppState,
    msg_rx: &mpsc::Receiver<Msg>,
    effect_runner: &EffectRunner,
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

fn run_dry_run(_paths: &RuntimePaths, _args: &Args) -> Result<i32, String> {
    // TODO: Implement dry-run mode
    println!("[dry-run] Not yet implemented");
    Ok(0)
}
