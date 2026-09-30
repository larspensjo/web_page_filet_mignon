use super::batch_runtime::{divert_batch_effects, BatchRuntime};
use super::live_progress::LiveSystemBatchProgress;
use super::CycleStartWorkReporter;
use chrono::Utc;
use engine_logging::{engine_debug, engine_info, engine_warn};
use harvester_core::{update, AppState, BatchObservation, Msg};
use harvester_io::{host_bootstrap::pump_pre_triage_refresh, EffectRunner};
use std::collections::HashSet;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc;
use std::sync::Arc;
use std::time::{Duration, Instant};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum CycleOutcome {
    Success,
    PartialFailure,
    TotalFailure,
}

#[derive(Debug, Clone, Copy)]
pub(crate) struct DispatchLoopOptions {
    pub(crate) tick_interval: Duration,
}

pub(crate) const MAX_DISPATCH_INBOX_BATCH: usize = 32;
const MAX_BATCH_MSG_LOG_LEN: usize = 240;

/// Determines if the batch cycle should settle (all reducer-owned work quiesced).
pub(super) fn should_settle_cycle(status: harvester_core::BatchStatus) -> bool {
    matches!(status, harvester_core::BatchStatus::Settled)
}

pub(super) fn batch_buffer_is_quiescent(state: &AppState, buffered_ids: &HashSet<u64>) -> bool {
    !buffered_ids.is_empty()
        && state
            .pending_llm_request_ids()
            .all(|request_id| buffered_ids.contains(&request_id))
}

/// Classifies the outcome of a completed cycle based on observation metrics.
pub(super) fn classify_cycle_outcome(obs: &BatchObservation) -> CycleOutcome {
    let has_failures = obs.jobs_failed > 0 || obs.triage_failed > 0;
    let has_successes = obs.jobs_done > 0 || obs.triage_completed > 0;

    match (has_successes, has_failures) {
        (true, false) => CycleOutcome::Success,
        (true, true) => CycleOutcome::PartialFailure,
        (false, true) => CycleOutcome::TotalFailure,
        (false, false) => CycleOutcome::Success, // Nothing to do is success
    }
}

pub(super) fn truncate_for_log(input: &str, max_len: usize) -> String {
    if input.chars().count() <= max_len {
        return input.to_string();
    }
    let mut truncated: String = input.chars().take(max_len).collect();
    truncated.push_str("...");
    truncated
}

pub(crate) fn summarize_batch_msg(msg: &Msg) -> String {
    match msg {
        Msg::PollSourcesClicked => "PollSourcesClicked".to_string(),
        Msg::PollStarted { total } => format!("PollStarted(total={total})"),
        Msg::AllSourcesPollEnded => "AllSourcesPollEnded".to_string(),
        Msg::SourcePollCompleted {
            source_id, urls, ..
        } => {
            format!(
                "SourcePollCompleted {{ source_id: {}, urls: {} }}",
                source_id,
                urls.len()
            )
        }
        Msg::JobProgress {
            job_id,
            stage,
            tokens,
            bytes,
            ..
        } => format!(
            "JobProgress {{ job_id: {}, stage: {:?}, bytes: {:?}, tokens: {:?} }}",
            job_id, stage, bytes, tokens
        ),
        Msg::JobDone { job_id, result, .. } => {
            let result_label = match result {
                harvester_core::JobResultKind::Success => "Success".to_string(),
                harvester_core::JobResultKind::Failed { reason } => {
                    format!("Failed({})", truncate_for_log(reason, 80))
                }
            };
            format!("JobDone {{ job_id: {}, result: {} }}", job_id, result_label)
        }
        Msg::TriageArticlesLoaded { delta, .. } => {
            format!(
                "TriageArticlesLoaded {{ articles: {} }}",
                delta.members.len()
            )
        }
        Msg::ArticlesLoaded { articles, .. } => {
            format!("ArticlesLoaded {{ articles: {} }}", articles.len())
        }
        Msg::PromptContextsLoaded { contexts } => {
            format!("PromptContextsLoaded {{ prompts: {} }}", contexts.len())
        }
        Msg::LlmMetadataLoaded {
            active_versions,
            effective_models,
        } => format!(
            "LlmMetadataLoaded {{ active_versions: {}, effective_models: {} }}",
            active_versions.len(),
            effective_models.len()
        ),
        _ => truncate_for_log(&format!("{:?}", msg), MAX_BATCH_MSG_LOG_LEN),
    }
}

pub(crate) fn should_log_batch_msg(msg: &Msg) -> bool {
    !matches!(
        msg,
        Msg::JobProgress {
            stage: harvester_core::Stage::Downloading,
            ..
        }
    )
}

/// Reduce startup metadata and the restored article window before the first
/// cycle reports unfinished work or requests intake.
pub(super) fn prepare_startup_window(
    state: &mut AppState,
    msg_rx: &mpsc::Receiver<Msg>,
    effect_runner: &EffectRunner,
) -> Result<(), String> {
    let mut effect_sink = |effects| effect_runner.enqueue(effects);
    let mut reducer_observer = |_: &str, _: Duration| {};
    prepare_startup_window_with_sink(state, msg_rx, &mut effect_sink, &mut reducer_observer)
}

pub(super) fn prepare_startup_window_with_sink(
    state: &mut AppState,
    msg_rx: &mpsc::Receiver<Msg>,
    effect_sink: &mut dyn FnMut(Vec<harvester_core::Effect>),
    reducer_observer: &mut dyn FnMut(&str, Duration),
) -> Result<(), String> {
    let mut templates_loaded = false;
    let mut metadata_loaded = false;
    let mut contexts_loaded = false;
    for _ in 0..10_000 {
        match msg_rx.recv_timeout(Duration::from_millis(100)) {
            Ok(msg) => {
                templates_loaded |= matches!(msg, Msg::PromptTemplateFilesLoaded);
                metadata_loaded |= matches!(msg, Msg::LlmMetadataLoaded { .. });
                contexts_loaded |= matches!(
                    msg,
                    Msg::PromptContextsLoaded { .. } | Msg::PromptContextsLoadFailed { .. }
                );
                let (next, effects) =
                    reduce_timed_owned(std::mem::take(state), msg, reducer_observer);
                *state = next;
                if !effects.is_empty() {
                    effect_sink(effects);
                }
                let refresh_started = Instant::now();
                let (next, effects, refresh_triggered) =
                    pump_pre_triage_refresh(std::mem::take(state));
                if refresh_triggered {
                    reducer_observer("EvaluatePreTriageRefresh", refresh_started.elapsed());
                }
                *state = next;
                if !effects.is_empty() {
                    effect_sink(effects);
                }
            }
            Err(mpsc::RecvTimeoutError::Timeout) => {}
            Err(mpsc::RecvTimeoutError::Disconnected) => {
                return Err("Startup message channel disconnected unexpectedly".into());
            }
        }
        let (next, effects) = reduce_timed_owned(
            std::mem::take(state),
            Msg::tick_at(Utc::now()),
            reducer_observer,
        );
        *state = next;
        if !effects.is_empty() {
            effect_sink(effects);
        }
        if templates_loaded
            && metadata_loaded
            && contexts_loaded
            && !state.pipeline_activity().intake_refresh_pending
        {
            return Ok(());
        }
    }
    Err("Startup article window did not settle before the first cycle".into())
}

/// Runs the inner dispatch loop until settlement or error.
/// Processes messages, updates state, executes effects, and checks for settlement.
pub(super) fn run_dispatch_loop(
    state: &mut AppState,
    msg_tx: &mpsc::Sender<Msg>,
    msg_rx: &mpsc::Receiver<Msg>,
    effect_runner: &EffectRunner,
    shutdown_flag: &Arc<AtomicBool>,
    options: DispatchLoopOptions,
) -> Result<CycleOutcome, String> {
    run_dispatch_loop_with_tick_interval(
        state,
        msg_tx,
        msg_rx,
        effect_runner,
        shutdown_flag,
        options,
        None,
        None,
    )
}

#[allow(clippy::too_many_arguments)] // Batch runtime is optional at this runner boundary.
pub(super) fn run_dispatch_loop_with_tick_interval(
    state: &mut AppState,
    msg_tx: &mpsc::Sender<Msg>,
    msg_rx: &mpsc::Receiver<Msg>,
    effect_runner: &EffectRunner,
    shutdown_flag: &Arc<AtomicBool>,
    options: DispatchLoopOptions,
    progress: Option<&mut LiveSystemBatchProgress>,
    batch_runtime: Option<&mut BatchRuntime>,
) -> Result<CycleOutcome, String> {
    let mut effect_sink = |effects| effect_runner.enqueue(effects);
    let mut reducer_observer = |_: &str, _: Duration| {};
    run_dispatch_loop_with_sink(
        state,
        msg_tx,
        msg_rx,
        &mut effect_sink,
        &mut reducer_observer,
        shutdown_flag,
        options,
        progress,
        batch_runtime,
    )
}

#[allow(clippy::too_many_arguments)] // Keeps sink and reducer timing at the host boundary.
pub(super) fn run_dispatch_loop_with_sink(
    state: &mut AppState,
    msg_tx: &mpsc::Sender<Msg>,
    msg_rx: &mpsc::Receiver<Msg>,
    effect_sink: &mut dyn FnMut(Vec<harvester_core::Effect>),
    reducer_observer: &mut dyn FnMut(&str, Duration),
    shutdown_flag: &Arc<AtomicBool>,
    options: DispatchLoopOptions,
    progress: Option<&mut LiveSystemBatchProgress>,
    batch_runtime: Option<&mut BatchRuntime>,
) -> Result<CycleOutcome, String> {
    run_dispatch_loop_with_sink_inner(
        state,
        msg_tx,
        msg_rx,
        effect_sink,
        reducer_observer,
        shutdown_flag,
        options,
        progress,
        batch_runtime,
    )
}

#[allow(clippy::too_many_arguments)]
fn run_dispatch_loop_with_sink_inner(
    state: &mut AppState,
    msg_tx: &mpsc::Sender<Msg>,
    msg_rx: &mpsc::Receiver<Msg>,
    effect_sink: &mut dyn FnMut(Vec<harvester_core::Effect>),
    reducer_observer: &mut dyn FnMut(&str, Duration),
    shutdown_flag: &Arc<AtomicBool>,
    options: DispatchLoopOptions,
    mut progress: Option<&mut LiveSystemBatchProgress>,
    mut batch_runtime: Option<&mut BatchRuntime>,
) -> Result<CycleOutcome, String> {
    let timeout = Duration::from_millis(100);
    let mut iterations = 0;
    let mut last_tick = Instant::now();
    let mut cycle_start_work = CycleStartWorkReporter::default();
    const MAX_ITERATIONS: usize = 10_000; // Safety limit

    if let Some(line) = cycle_start_work.pending_count_line(state) {
        if let Some(p) = progress.as_deref_mut() {
            p.suspend_for_output();
        }
        println!("{line}");
        if let Some(p) = progress.as_deref_mut() {
            let cost = batch_runtime
                .as_ref()
                .map_or(0, |batch| batch.realized_cost_microdollars);
            p.resume(state, cost);
        }
    }

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

        // Receive at least one message with timeout, then drain a bounded batch.
        // Large restored states make reducer clones expensive; bounding the batch
        // keeps reducer-owned run advancement responsive under bursts.
        let mut recv_idle = false;
        let mut enqueued_effects = false;
        let mut queued_effects = Vec::new();
        match msg_rx.recv_timeout(timeout) {
            Ok(first_msg) => {
                let mut inbox = vec![first_msg];
                while inbox.len() < MAX_DISPATCH_INBOX_BATCH {
                    let Ok(next_msg) = msg_rx.try_recv() else {
                        break;
                    };
                    inbox.push(next_msg);
                }

                for msg in inbox {
                    if should_log_batch_msg(&msg) {
                        engine_debug!("[batch] Processing message: {}", summarize_batch_msg(&msg));
                    }
                    let (new_state, effects) =
                        reduce_timed_owned(std::mem::take(state), msg, reducer_observer);
                    *state = new_state;
                    queued_effects.extend(effects);
                    if let Some(p) = progress.as_deref_mut() {
                        let cost = batch_runtime
                            .as_ref()
                            .map_or(0, |batch| batch.realized_cost_microdollars);
                        p.clear_phase_override();
                        p.paint(state, cost, false);
                    }
                }

                let refresh_started = Instant::now();
                let current_state = std::mem::take(state);
                let (next_state, effects, refresh_triggered) =
                    pump_pre_triage_refresh(current_state);
                if refresh_triggered {
                    reducer_observer("EvaluatePreTriageRefresh", refresh_started.elapsed());
                }
                *state = next_state;
                queued_effects.extend(effects);
            }
            Err(mpsc::RecvTimeoutError::Timeout) => {
                recv_idle = true;
            }
            Err(mpsc::RecvTimeoutError::Disconnected) => {
                return Err("Message channel disconnected unexpectedly".to_string());
            }
        }

        if state.pipeline_run_phase() != harvester_core::PipelineRunPhase::Idle {
            let (new_state, advance_effects) = reduce_timed_owned(
                std::mem::take(state),
                Msg::PipelineRunAdvance,
                reducer_observer,
            );
            *state = new_state;
            queued_effects.extend(advance_effects);
        }

        if state.pipeline_wave_policy() != harvester_core::PipelineWavePolicy::Disabled
            && last_tick.elapsed() >= options.tick_interval
        {
            let tick_msg = Msg::tick_at(Utc::now());
            let (new_state, tick_effects) =
                reduce_timed_owned(std::mem::take(state), tick_msg, reducer_observer);
            *state = new_state;
            queued_effects.extend(tick_effects);
            last_tick = Instant::now();
        }

        if !queued_effects.is_empty() {
            engine_debug!("[batch] Enqueuing {} effects", queued_effects.len());
            let queued_effects = if let Some(batch) = batch_runtime.as_deref_mut() {
                divert_batch_effects(state, queued_effects, batch, msg_tx)
            } else {
                queued_effects
            };
            if !queued_effects.is_empty() {
                enqueued_effects = true;
                effect_sink(queued_effects);
            }
        }

        let lines = cycle_start_work.pending_lines(state);
        if !lines.is_empty() {
            if let Some(p) = progress.as_deref_mut() {
                p.suspend_for_output();
            }
            for line in lines {
                println!("{line}");
            }
            if let Some(p) = progress.as_deref_mut() {
                let cost = batch_runtime
                    .as_ref()
                    .map_or(0, |batch| batch.realized_cost_microdollars);
                p.resume(state, cost);
            }
        }

        // Check for settlement after processing available work.
        if let Some(p) = progress.as_deref_mut() {
            let cost = batch_runtime
                .as_ref()
                .map_or(0, |batch| batch.realized_cost_microdollars);
            p.clear_phase_override();
            p.paint(state, cost, false);
        }
        let obs = state.batch_observation();

        // This prevents an immediate idle-state exit before queued actions
        // (like PipelineRunRequested) have been reduced.
        if recv_idle && !enqueued_effects {
            if let Some(batch) = batch_runtime.as_deref_mut() {
                let buffered_ids = batch.coordinator.buffered_request_ids();
                if batch_buffer_is_quiescent(state, &buffered_ids) {
                    if let Err(err) = batch
                        .runtime
                        .block_on(batch.coordinator.flush(msg_tx, Utc::now().to_rfc3339()))
                    {
                        engine_warn!(
                            "[batch-submit] flush failed; manifest/buffer state retained where possible: {}",
                            err
                        );
                    }
                    continue;
                }
            }
        }

        if should_settle_cycle(state.batch_status()) {
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
    }
}

fn reduce_timed_owned(
    state: AppState,
    message: Msg,
    observer: &mut dyn FnMut(&str, Duration),
) -> (AppState, Vec<harvester_core::Effect>) {
    let kind = message.kind();
    let started = Instant::now();
    let (next_state, effects) = update(state, message);
    observer(kind, started.elapsed());
    (next_state, effects)
}
