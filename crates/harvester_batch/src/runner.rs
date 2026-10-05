use crate::cli::{Args, CheckpointCommand};
use chrono::Utc;
use crossterm::{cursor::Show, QueueableCommand};
use engine_logging::{engine_info, engine_warn};
use harvester_core::{AppState, BatchObservation, Msg};
use harvester_engine::llm::{ModelId, ProviderKind, OPENAI_MODEL_GPT_4O_MINI};
use harvester_io::{
    acquire_lock, host_bootstrap::HostLlmDefaults, load_briefing_checkpoint, load_sources,
    save_blacklist, save_briefing_checkpoint, RuntimePaths, COMMAND_LINE_LOCK_IDENTITY,
};
use std::io::{IsTerminal, Write};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc;
use std::sync::Arc;
use std::time::{Duration, Instant};

mod bootstrap;
mod dispatch_loop;
mod live_progress;
mod reporting;

pub(crate) const BATCH_MISSING_API_KEY_WARNING: &str =
    "[batch] OPENAI_API_KEY not set; AI triage/summary features disabled";
pub(crate) const BATCH_EMPTY_API_KEY_WARNING: &str =
    "[batch] OPENAI_API_KEY is empty; AI triage/summary features disabled";

pub(crate) fn batch_host_llm_defaults() -> HostLlmDefaults {
    HostLlmDefaults {
        default_model: ModelId::new(ProviderKind::OpenAi, OPENAI_MODEL_GPT_4O_MINI),
        session_id_prefix: "batch-",
    }
}

pub(crate) use bootstrap::{apply_llm_availability, apply_signal_candidate_selection_settings};
use dispatch_loop::prepare_startup_window;
#[cfg(test)]
use dispatch_loop::run_dispatch_loop;
#[cfg(test)]
use dispatch_loop::run_dispatch_loop_with_tick_interval;
pub(crate) use dispatch_loop::{
    should_log_batch_msg, summarize_batch_msg, CycleOutcome, DispatchLoopOptions,
    MAX_DISPATCH_INBOX_BATCH,
};

use live_progress::LiveBatchProgress;
pub(crate) use reporting::CycleStartWorkReporter;
use reporting::{
    format_startup_notice, print_final_summary, print_poll_stats, CycleCounts, PollSummaryReporter,
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

/// Run one poll, download and synchronous processing cycle, then exit.
pub fn run(args: Args) -> Result<i32, String> {
    engine_info!("[batch] Starting harvester_batch");
    engine_info!("[batch] output_dir: {:?}", args.output_dir);
    engine_info!("[batch] sources: {:?}", args.sources_path());
    engine_info!(
        "[batch] signal_candidate_threshold: {:?}",
        args.signal_candidate_threshold
    );
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
            let _lock_guard = acquire_lock(
                &paths.output_dir,
                COMMAND_LINE_LOCK_IDENTITY,
                args.force_unlock,
            )?;
            execute_checkpoint_write(cmd, &paths)?;
            return Ok(0);
        }
        None => {}
    }

    engine_info!("[batch] Acquiring lock");
    let _lock_guard = acquire_lock(
        &paths.output_dir,
        COMMAND_LINE_LOCK_IDENTITY,
        args.force_unlock,
    )?;

    // Install signal handler immediately after lock acquisition so Ctrl-C always
    // reaches the shared graceful-shutdown path for every execution mode.
    let shutdown_flag = Arc::new(AtomicBool::new(false));
    let interactive = std::io::stdout().is_terminal() && std::io::stderr().is_terminal();
    install_signal_handler(Arc::clone(&shutdown_flag), interactive);

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

    // Validate source configuration
    engine_info!(
        "[batch] Loading source registry from {:?}",
        paths.sources_path
    );
    let source_registry = load_sources(&paths.sources_path);
    engine_info!(
        "[batch] Source registry entries loaded={}",
        source_registry.sources.len()
    );

    let (msg_tx, msg_rx) = mpsc::channel::<Msg>();
    if interactive {
        println!("{}", format_startup_notice("one cycle"));
    }
    let (mut state, effect_runner) = bootstrap::prepare_runtime(&paths, &args, msg_tx.clone())?;
    prepare_startup_window(&mut state, &msg_rx, &effect_runner)?;
    let run_started_at = Instant::now();
    let mut cycle_baseline = CycleCounterBaseline::from_observation(&state.batch_observation());
    let mut progress = LiveBatchProgress::new(interactive);
    if !interactive {
        println!("[batch] started mode=one-cycle");
    }
    progress.paint(&state, state.llm_quota().usage.cost_microdollars, true);
    let mut effect_sink = |effects| effect_runner.enqueue(effects);
    let mut reducer_observer = |_: &str, _: Duration| {};
    let mut cycle_counts = CycleCounts::default();
    let mut cycle_outcome = CycleOutcome::Success;
    let mut poll_summary = PollSummaryReporter::default();
    execute_cycle_with_sink(
        &mut state,
        &paths,
        &msg_tx,
        &msg_rx,
        &mut effect_sink,
        &mut reducer_observer,
        &shutdown_flag,
        &mut progress,
        |outcome, state, progress| {
            cycle_outcome = outcome;
            let obs = state.batch_observation();
            cycle_counts = cycle_baseline.measure_cycle_and_advance(&obs);
            if let Some(summary) = poll_summary.take(&obs.source_poll_stats) {
                progress.suspend_for_output();
                println!("{summary}");
                progress.resume(state, state.llm_quota().usage.cost_microdollars);
            }
        },
        None,
    )?;
    // Flush and stop the ordered persistence sinks before the final synchronous save.
    engine_info!("[batch] Graceful shutdown: draining effects and persisting final state");
    let result_save_error = effect_runner.flush_results().err();
    drop(effect_runner);
    drop(msg_rx);
    persist_final_cycle_state(&paths, &state, None);
    progress.set_stopping(shutdown_flag.load(Ordering::Relaxed));
    progress.paint(&state, state.llm_quota().usage.cost_microdollars, true);
    progress.suspend_for_output();
    print_final_summary(
        1,
        &state.batch_observation(),
        cycle_counts.new_jobs,
        cycle_counts.triage_completed,
        cycle_counts.summary_completed,
        run_started_at.elapsed(),
    );
    progress.finish();
    engine_info!("[batch] Shutdown complete");
    Ok(exit_code_with_shutdown(
        if let Some(reason) = state
            .result_store_failure()
            .map(str::to_owned)
            .or_else(|| result_save_error.map(|e| e.to_string()))
        {
            eprintln!("Final summary: AI features unavailable: {reason}");
            1
        } else {
            determine_exit_code(usize::from(cycle_outcome == CycleOutcome::TotalFailure))
        },
        shutdown_flag.load(Ordering::Relaxed),
    ))
}

/// Prepare the same hydrated batch state and startup window for a host that
/// supplies its own effect sink. The sink runs on the host/effect boundary;
/// reducer policy remains in `harvester_core`.
pub fn prepare_cycle_state_with_effect_sink(
    paths: &RuntimePaths,
    args: &Args,
    msg_rx: &mpsc::Receiver<Msg>,
    effect_sink: &mut dyn FnMut(Vec<harvester_core::Effect>),
    reducer_observer: &mut dyn FnMut(&str, Duration),
) -> Result<AppState, String> {
    let (hydrated_state, startup_effects) = bootstrap::hydrate_batch_state(paths, args);
    let mut state = bootstrap::apply_llm_availability(
        hydrated_state,
        harvester_core::AiAvailability::Available,
    );
    if !startup_effects.is_empty() {
        effect_sink(startup_effects);
    }
    dispatch_loop::prepare_startup_window_with_sink(
        &mut state,
        msg_rx,
        effect_sink,
        reducer_observer,
    )?;
    Ok(state)
}

/// Request and run one normal Full batch cycle through the production reducer
/// dispatch loop, using the host-provided effect sink.
#[allow(clippy::too_many_arguments)]
pub fn run_single_cycle_with_effect_sink(
    state: &mut AppState,
    paths: &RuntimePaths,
    msg_tx: &mpsc::Sender<Msg>,
    msg_rx: &mpsc::Receiver<Msg>,
    effect_sink: &mut dyn FnMut(Vec<harvester_core::Effect>),
    reducer_observer: &mut dyn FnMut(&str, Duration),
    file_write_observer: &harvester_io::FileWriteObserver,
) -> Result<(), String> {
    let shutdown_flag = Arc::new(AtomicBool::new(false));
    let started = Instant::now();
    let mut progress = LiveBatchProgress::new(false);
    println!("[batch] started mode=one-cycle");
    progress.paint(state, state.llm_quota().usage.cost_microdollars, true);
    execute_cycle_with_sink(
        state,
        paths,
        msg_tx,
        msg_rx,
        effect_sink,
        reducer_observer,
        &shutdown_flag,
        &mut progress,
        |_, _, _| {},
        Some(file_write_observer),
    )?;
    progress.paint(state, state.llm_quota().usage.cost_microdollars, true);
    progress.suspend_for_output();
    let obs = state.batch_observation();
    print_final_summary(
        1,
        &obs,
        obs.jobs_total,
        obs.triage_completed,
        obs.summary_completed,
        started.elapsed(),
    );
    print_poll_stats(&obs.source_poll_stats);
    progress.finish();
    Ok(())
}

pub fn persist_final_cycle_state(
    paths: &RuntimePaths,
    state: &AppState,
    observer: Option<&harvester_io::FileWriteObserver>,
) {
    persist_cycle_state(paths, state, observer);
}

#[allow(clippy::too_many_arguments)]
fn execute_cycle_with_sink<F>(
    state: &mut AppState,
    paths: &RuntimePaths,
    msg_tx: &mpsc::Sender<Msg>,
    msg_rx: &mpsc::Receiver<Msg>,
    effect_sink: &mut dyn FnMut(Vec<harvester_core::Effect>),
    reducer_observer: &mut dyn FnMut(&str, Duration),
    shutdown_flag: &Arc<AtomicBool>,
    progress: &mut live_progress::LiveSystemBatchProgress,
    after_dispatch: F,
    file_write_observer: Option<&harvester_io::FileWriteObserver>,
) -> Result<(), String>
where
    F: FnOnce(CycleOutcome, &AppState, &mut live_progress::LiveSystemBatchProgress),
{
    let outcome = dispatch_cycle_with_sink(
        state,
        msg_tx,
        msg_rx,
        effect_sink,
        reducer_observer,
        shutdown_flag,
        Some(progress),
    )?;
    after_dispatch(outcome, state, progress);
    // The reducer loop is the sole state owner. This checkpoint can race the
    // debounced writer; final shutdown writes again after the runner stops.
    engine_info!("[batch] Persisting state");
    progress.paint(state, state.llm_quota().usage.cost_microdollars, true);
    persist_cycle_state(paths, state, file_write_observer);
    Ok(())
}

#[allow(clippy::too_many_arguments)]
fn dispatch_cycle_with_sink(
    state: &mut AppState,
    msg_tx: &mpsc::Sender<Msg>,
    msg_rx: &mpsc::Receiver<Msg>,
    effect_sink: &mut dyn FnMut(Vec<harvester_core::Effect>),
    reducer_observer: &mut dyn FnMut(&str, Duration),
    shutdown_flag: &Arc<AtomicBool>,
    progress: Option<&mut live_progress::LiveSystemBatchProgress>,
) -> Result<CycleOutcome, String> {
    {
        msg_tx
            .send(Msg::PipelineRunRequested {
                scope: harvester_core::PipelineRunScope::Full,
            })
            .map_err(|error| format!("Failed to request full pipeline run: {error}"))?;
    }
    dispatch_loop::run_dispatch_loop_with_sink(
        state,
        msg_rx,
        effect_sink,
        reducer_observer,
        shutdown_flag,
        DispatchLoopOptions {
            tick_interval: Duration::from_millis(75),
            ..DispatchLoopOptions::default()
        },
        progress,
    )
}

fn persist_cycle_state(
    paths: &RuntimePaths,
    state: &AppState,
    observer: Option<&harvester_io::FileWriteObserver>,
) {
    let started = Instant::now();
    let snapshot = harvester_core::PersistenceSnapshot::capture(state);
    if let Err(error) = harvester_io::try_persist_runtime_state_with_pending(
        &paths.state_path,
        &snapshot.completed,
        &snapshot.pending_intake,
    ) {
        engine_warn!(
            "[batch] failed to save runtime state {}: {}",
            paths.state_path.display(),
            error
        );
    } else if let Some(observer) = observer {
        if let Ok(metadata) = std::fs::metadata(&paths.state_path) {
            observer(&paths.state_path, metadata.len(), started.elapsed());
        }
    }
    let started = Instant::now();
    if let Err(error) = save_blacklist(&paths.blacklist_path, state.blacklist()) {
        engine_warn!("[batch] failed to save blacklist: {}", error);
    } else if let (Some(observer), Ok(metadata)) =
        (observer, std::fs::metadata(&paths.blacklist_path))
    {
        observer(&paths.blacklist_path, metadata.len(), started.elapsed());
    }
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
            // Drop cannot restore a cursor hidden by the progress block.
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
