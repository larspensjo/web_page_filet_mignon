#[cfg(test)]
use crate::batch_coordinator::BatchPeek;
use crate::cli::{Args, CheckpointCommand};
use crate::lock;
use crate::progress::{BatchDisplayPhase, BatchRunBaseline};
use chrono::Utc;
use crossterm::{cursor::Show, QueueableCommand};
use engine_logging::{engine_info, engine_warn};
use harvester_core::{BatchObservation, Msg};
use harvester_io::{
    load_briefing_checkpoint, load_sources, persist_completed_jobs, save_blacklist,
    save_briefing_checkpoint, RuntimePaths,
};
use std::io::{IsTerminal, Write};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc;
use std::sync::Arc;
use std::time::{Duration, Instant};

mod batch_runtime;
mod bootstrap;
mod dispatch_loop;
mod drain_control;
mod dry_run;
mod live_progress;
mod reporting;

#[cfg(test)]
use dispatch_loop::run_dispatch_loop;
use dispatch_loop::run_dispatch_loop_with_tick_interval;
pub(crate) use dispatch_loop::{
    maybe_dispatch_batch_ai_orchestration, should_log_batch_msg, summarize_batch_msg, CycleOutcome,
    DispatchLoopOptions, MAX_DISPATCH_INBOX_BATCH,
};
use dry_run::run_dry_run;

use batch_runtime::collect_and_rearm_batch_cycle;
#[cfg(test)]
pub(crate) use batch_runtime::persist_batch_replay_records;
use batch_runtime::remove_collected_with_persisted_cache_confirmation;
pub(crate) use bootstrap::{
    apply_signal_candidate_selection_settings, build_effect_runner, is_ai_orchestration_enabled,
};
#[cfg(test)]
use drain_control::{
    batch_drain_made_progress, decide_batch_wait, should_exit_batch_drain_after_no_progress,
    BatchDrainSnapshot, BatchWaitDecision,
};
use drain_control::{evaluate_batch_drain, DrainControl, DrainControlState};

use live_progress::LiveBatchProgress;
pub(crate) use reporting::microdollars_to_display;
use reporting::{
    format_awaiting_batch_line, format_drain_summary, format_optional_cycle_diagnostics,
    print_final_summary, print_poll_stats, CycleCounts,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
struct CycleCounterBaseline {
    jobs_total: usize,
    jobs_done: usize,
    jobs_failed: usize,
    triage_completed: usize,
    triage_failed: usize,
    summary_completed: usize,
    summary_failed: usize,
    imports_completed: usize,
    imports_failed: usize,
}

fn batch_mode_label(batch_api_enabled: bool, drain: bool) -> &'static str {
    match (batch_api_enabled, drain) {
        (_, true) => "drain",
        (true, false) => "batch-api",
        (false, false) => "recurring",
    }
}

impl CycleCounterBaseline {
    fn from_observation(obs: &BatchObservation) -> Self {
        Self {
            jobs_total: obs.jobs_total,
            jobs_done: obs.jobs_done,
            jobs_failed: obs.jobs_failed,
            triage_completed: obs.triage_completed,
            triage_failed: obs.triage_failed,
            summary_completed: obs.summary_completed,
            summary_failed: obs.summary_failed,
            imports_completed: obs.imports_completed,
            imports_failed: obs.imports_failed,
        }
    }

    fn measure_cycle_and_advance(&mut self, obs: &BatchObservation) -> CycleCounts {
        let counts = CycleCounts {
            new_jobs: obs.jobs_total.saturating_sub(self.jobs_total),
            jobs_done: obs.jobs_done.saturating_sub(self.jobs_done),
            jobs_failed: obs.jobs_failed.saturating_sub(self.jobs_failed),
            triage_completed: obs.triage_completed.saturating_sub(self.triage_completed),
            triage_failed: obs.triage_failed.saturating_sub(self.triage_failed),
            summary_completed: obs.summary_completed.saturating_sub(self.summary_completed),
            summary_failed: obs.summary_failed.saturating_sub(self.summary_failed),
            imports_completed: obs.imports_completed.saturating_sub(self.imports_completed),
            imports_failed: obs.imports_failed.saturating_sub(self.imports_failed),
        };
        *self = Self::from_observation(obs);
        counts
    }
}

fn should_stop_after_cycle(single_shot: bool, shutdown_requested: bool) -> bool {
    shutdown_requested || single_shot
}

/// A collect-only cycle skips source polling and only advances work the batch
/// manifest already owns. Batch API mode polls once and then collects; drain
/// mode never polls, so its very first cycle is already collect-only.
fn is_collect_only_cycle(batch_api_enabled: bool, drain: bool, cycle_count: usize) -> bool {
    batch_api_enabled && (drain || cycle_count > 1)
}

fn require_new_jobs_since(
    single_shot: bool,
    batch_api: bool,
    cycle_jobs_total_baseline: usize,
) -> Option<usize> {
    (single_shot && !batch_api).then_some(cycle_jobs_total_baseline)
}

pub(crate) fn exit_code_with_shutdown(default_exit_code: i32, shutdown_requested: bool) -> i32 {
    if shutdown_requested {
        130
    } else {
        default_exit_code
    }
}

fn determine_exit_code(total_failure_cycles: usize) -> i32 {
    if total_failure_cycles > 0 {
        1
    } else {
        0
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

    let sources_path = args.sources_path();
    let paths = RuntimePaths::new(
        args.output_dir.clone(),
        sources_path,
        args.contexts_dir.clone(),
        args.prompts_dir.clone(),
    );

    // Handle checkpoint commands before entering the batch loop.
    match args.checkpoint_command()? {
        Some(CheckpointCommand::Show) => {
            let val = load_briefing_checkpoint(&paths.briefing_checkpoint_path);
            println!("{}", val.as_deref().unwrap_or("NONE"));
            return Ok(0);
        }
        Some(cmd) => {
            let _lock_guard = lock::acquire_lock(&paths.output_dir, args.force_unlock)?;
            execute_checkpoint_write(cmd, &paths)?;
            return Ok(0);
        }
        None => {}
    }

    engine_info!("[batch] Acquiring lock");
    let _lock_guard = lock::acquire_lock(&paths.output_dir, args.force_unlock)?;

    // Install signal handler immediately after lock acquisition so Ctrl-C always
    // reaches the shared graceful-shutdown path for every execution mode.
    let shutdown_flag = Arc::new(AtomicBool::new(false));
    let interactive = std::io::stdout().is_terminal() && std::io::stderr().is_terminal();
    install_signal_handler(Arc::clone(&shutdown_flag), interactive);

    if args.dry_run {
        engine_info!("[batch] Dry-run mode: single poll only");
        return run_dry_run(&paths, &args, &shutdown_flag);
    }

    // Import mode: branch before source loading
    if let Some(import_dir) = &args.import_saved_web_dir {
        engine_info!("[batch] Import mode: dir={}", import_dir.display());
        return crate::import_mode::run_import_mode(
            &paths,
            &args,
            import_dir.clone(),
            Arc::clone(&shutdown_flag),
        );
    }

    if args.refresh_stale_summaries_limit.is_some() {
        engine_info!("[batch] Summary refresh mode enabled");
        return crate::summary_refresh::run_refresh_stale_summaries_mode(
            &paths,
            &args,
            &shutdown_flag,
        );
    }

    // Validate source configuration
    engine_info!(
        "[batch] Loading source registry from {:?}",
        paths.sources_path
    );
    let source_registry = load_sources(&paths.sources_path);

    // Drain never polls, so an unsupported source must not be able to abort a
    // collection of work that has already been paid for.
    if !args.allow_unsupported_sources && !args.drain {
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

    let (mut state, effect_runner, mut batch_runtime, enable_ai_orchestration) =
        bootstrap::prepare_runtime(&paths, &args, msg_tx.clone())?;

    let run_baseline = BatchRunBaseline::from_observation(&state.batch_observation());
    let run_started_at = Instant::now();
    let mut progress = LiveBatchProgress::new(run_baseline, interactive, args.ascii_progress);
    if !interactive {
        println!(
            "[batch] started mode={}",
            batch_mode_label(args.batch_api_enabled(), args.drain)
        );
    }

    // Ordinary mode polls repeatedly. Batch API mode runs one intake cycle,
    // then drains exactly that cycle's deferred work without polling again.
    let poll_interval = Duration::from_secs((args.poll_interval * 60) as u64);
    let mut cycle_count = 0;
    let mut total_cycles = 0;
    let mut total_failure_cycles = 0;
    let mut cycle_baseline = CycleCounterBaseline::from_observation(&state.batch_observation());
    let mut total_new_articles = 0usize;
    let mut total_triaged = 0usize;
    let mut total_summarized = 0usize;
    let mut drain_control_state = DrainControlState::default();

    'cycles: loop {
        cycle_count += 1;
        total_cycles += 1;
        let collect_only_cycle =
            is_collect_only_cycle(args.batch_api_enabled(), args.drain, cycle_count);
        progress.record_pass(collect_only_cycle);
        if collect_only_cycle {
            engine_info!(
                "[batch] === Starting collect-only cycle {} ===",
                cycle_count
            );
        } else {
            engine_info!("[batch] === Starting cycle {} ===", cycle_count);
        }
        let cycle_jobs_total_baseline = state.batch_observation().jobs_total;

        if let Some(batch) = batch_runtime.as_mut() {
            state = collect_and_rearm_batch_cycle(
                state,
                batch,
                &paths,
                &effect_runner,
                &msg_tx,
                &mut progress,
            );
        }

        if !collect_only_cycle {
            progress.clear_phase_override();
            progress.paint(
                &state,
                batch_runtime
                    .as_ref()
                    .map_or(0, |batch| batch.realized_cost_microdollars),
                true,
            );
            engine_info!("[batch] Dispatching poll sources");
            msg_tx
                .send(Msg::PollSourcesClicked)
                .map_err(|e| format!("Failed to dispatch poll: {}", e))?;
        }

        // Run dispatch loop until settled
        let outcome = run_dispatch_loop_with_tick_interval(
            &mut state,
            &msg_tx,
            &msg_rx,
            &effect_runner,
            &shutdown_flag,
            DispatchLoopOptions {
                enable_ai_orchestration,
                require_new_jobs_since: require_new_jobs_since(
                    args.single_shot,
                    args.batch_api_enabled(),
                    cycle_jobs_total_baseline,
                ),
                tick_interval: Duration::from_millis(75),
            },
            Some(&mut progress),
            batch_runtime.as_mut(),
        )?;

        // Track outcome statistics
        match outcome {
            CycleOutcome::Success => {}
            CycleOutcome::PartialFailure => {}
            CycleOutcome::TotalFailure => total_failure_cycles += 1,
        }

        // Print cycle summary
        let obs = state.batch_observation();
        let cycle_counts = cycle_baseline.measure_cycle_and_advance(&obs);
        total_new_articles += cycle_counts.new_jobs;
        total_triaged += cycle_counts.triage_completed;
        total_summarized += cycle_counts.summary_completed;
        let current_cost = batch_runtime
            .as_ref()
            .map_or(0, |batch| batch.realized_cost_microdollars);
        let diagnostics = format_optional_cycle_diagnostics(
            args.verbose_progress,
            cycle_count == 1,
            !collect_only_cycle,
            cycle_count,
            &outcome,
            &cycle_counts,
            current_cost,
            &obs,
            &state.llm_usage_rows(),
            progress.last_provider_check_local,
        );
        if !diagnostics.is_empty() {
            progress.suspend_for_output();
            for line in diagnostics {
                println!("{line}");
            }
            progress.resume(&state, current_cost);
        }

        // Persist state
        engine_info!("[batch] Persisting state");
        progress.set_phase(BatchDisplayPhase::Persisting);
        progress.paint(&state, current_cost, true);
        let completed_jobs = state.completed_jobs_snapshot();
        persist_completed_jobs(&paths.state_path, &completed_jobs);
        if let Err(err) = save_blacklist(&paths.blacklist_path, state.blacklist()) {
            engine_warn!("[batch] failed to save blacklist: {}", err);
        }

        let shutdown_requested = shutdown_flag.load(Ordering::Relaxed);

        // Drain collects whatever the provider has already finished and exits.
        // It deliberately does not consult the reducer's deferred counters: a
        // fresh drain process has no deferred work to begin with, because that
        // state lives in memory rather than on disk. The manifest is the only
        // durable record of what is still outstanding.
        if args.drain {
            // A drain has no next cycle to run the confirmation, so snapshots
            // whose results already reached the caches are pruned here. Failure
            // only retains them for the next run, so it is not fatal.
            if let Some(batch) = batch_runtime.as_mut() {
                if let Err(err) = remove_collected_with_persisted_cache_confirmation(batch, &paths)
                {
                    engine_warn!(
                        "[batch-collect] persisted cache confirmation failed; retaining snapshots: {}",
                        err
                    );
                }
            }
            let summary = format_drain_summary(
                &batch_runtime
                    .as_ref()
                    .map(|batch| batch.coordinator.pending_manifest_batches())
                    .unwrap_or_default(),
            );
            engine_info!("{}", summary);
            progress.suspend_for_output();
            println!("{summary}");
            progress.resume(&state, current_cost);
            break;
        }

        match evaluate_batch_drain(
            args.batch_api_enabled(),
            &obs,
            &mut batch_runtime,
            shutdown_flag.as_ref(),
            &state,
            &mut progress,
            &mut drain_control_state,
        ) {
            DrainControl::ContinueCollectCycle => continue 'cycles,
            DrainControl::Break => break,
            DrainControl::Proceed => {}
        }

        // Check for shutdown signal or single-shot completion.
        if should_stop_after_cycle(args.single_shot, shutdown_requested) {
            if args.single_shot {
                engine_info!("[batch] Single-shot mode completed one cycle; exiting");
            }
            if shutdown_requested {
                engine_info!("[batch] Shutdown signal received, exiting");
            }
            break;
        }

        // Sleep interruptibly before the next ordinary polling cycle.
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
    if let Err(err) = save_blacklist(&paths.blacklist_path, state.blacklist()) {
        engine_warn!("[batch] failed to save blacklist on shutdown: {}", err);
    }

    let final_cost = batch_runtime
        .as_ref()
        .map_or(0, |batch| batch.realized_cost_microdollars);
    progress.set_phase(if shutdown_flag.load(Ordering::Relaxed) {
        BatchDisplayPhase::Interrupted
    } else {
        BatchDisplayPhase::Complete
    });
    progress.paint(&state, final_cost, true);
    progress.suspend_for_output();

    // Print final summary
    print_final_summary(
        args.batch_api_enabled(),
        total_cycles,
        &state.batch_observation(),
        total_new_articles,
        total_triaged,
        total_summarized,
        run_started_at.elapsed(),
        final_cost,
    );
    let final_obs = state.batch_observation();
    print_poll_stats(&final_obs.source_poll_stats);
    if args.verbose_progress {
        if let Some(line) = format_awaiting_batch_line(
            final_obs.triage_deferred,
            final_obs.summary_deferred,
            final_obs.signal_deferred,
        ) {
            println!("{line}");
            println!("  Run again after the batches complete to collect results.");
        }
    }
    progress.finish();

    engine_info!("[batch] Shutdown complete");

    Ok(exit_code_with_shutdown(
        determine_exit_code(total_failure_cycles),
        shutdown_flag.load(Ordering::Relaxed),
    ))
}

/// Writes or clears the briefing checkpoint file.
///
/// Called after the output lock is already held.
fn execute_checkpoint_write(cmd: CheckpointCommand, paths: &RuntimePaths) -> Result<(), String> {
    match cmd {
        CheckpointCommand::Set(ts) => {
            // ts was already validated by checkpoint_command()
            engine_info!("[briefing-checkpoint] set to {}", ts);
            save_briefing_checkpoint(&paths.briefing_checkpoint_path, Some(ts.as_str()))
        }
        CheckpointCommand::SetNow => {
            let ts = Utc::now().to_rfc3339();
            engine_info!("[briefing-checkpoint] set to {}", ts);
            save_briefing_checkpoint(&paths.briefing_checkpoint_path, Some(ts.as_str()))
        }
        CheckpointCommand::Clear => {
            engine_info!("[briefing-checkpoint] cleared");
            save_briefing_checkpoint(&paths.briefing_checkpoint_path, None)
        }
        CheckpointCommand::Show => unreachable!("Show is handled before lock acquisition"),
    }
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

/// Installs a signal handler for SIGINT/SIGTERM.
///
/// The first interrupt requests the runner's graceful shutdown path. The lock
/// remains held until `LockGuard` drops after the run returns. A second signal
/// hard-exits so a stuck network call cannot make the process unkillable.
fn install_signal_handler(shutdown_flag: Arc<AtomicBool>, interactive: bool) {
    let handler = move || {
        if shutdown_flag.swap(true, Ordering::Relaxed) {
            eprintln!("harvester_batch: interrupted again — exiting immediately");
            // The process exits without unwinding on the second interrupt, so
            // Drop cannot restore a cursor hidden by the dashboard.
            let mut stdout = std::io::stdout();
            restore_cursor_before_immediate_exit(&mut stdout, interactive);
            std::process::exit(130);
        }
        eprintln!("harvester_batch: interrupted — shutting down; lock remains held");
    };

    ctrlc::set_handler(handler).expect("Error setting signal handler");
}

fn restore_cursor_before_immediate_exit<W: Write>(stdout: &mut W, interactive: bool) {
    if interactive {
        let _ = stdout.queue(Show);
        let _ = stdout.flush();
    }
}

#[cfg(test)]
mod tests;
