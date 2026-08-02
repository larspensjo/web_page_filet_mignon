use super::apply_signal_candidate_selection_settings;
use super::dispatch_loop::{run_dispatch_loop, DispatchLoopOptions};
use super::exit_code_with_shutdown;
use super::reporting::print_poll_stats;
use crate::cli::Args;
use engine_logging::engine_info;
use harvester_core::{update, AppState, Msg};
use harvester_io::{load_completed_jobs, EffectRunner, NoOpPlatformHandler, RuntimePaths};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc;
use std::sync::Arc;
use std::time::Duration;

pub(super) fn run_dry_run(
    paths: &RuntimePaths,
    args: &Args,
    shutdown_flag: &Arc<AtomicBool>,
) -> Result<i32, String> {
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
    apply_signal_candidate_selection_settings(&mut state, args);

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

    // Run dispatch loop until settlement or graceful shutdown.
    let outcome = run_dispatch_loop(
        &mut state,
        &msg_tx,
        &msg_rx,
        &effect_runner,
        shutdown_flag,
        DispatchLoopOptions {
            enable_ai_orchestration: false,
            require_new_jobs_since: None,
            tick_interval: Duration::from_millis(75),
        },
    )?;

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
    print_poll_stats(&obs.source_poll_stats);
    println!("======================\n");

    engine_info!("[dry-run] Dry-run complete (no state modifications)");
    Ok(exit_code_with_shutdown(
        0,
        shutdown_flag.load(Ordering::Relaxed),
    ))
}
