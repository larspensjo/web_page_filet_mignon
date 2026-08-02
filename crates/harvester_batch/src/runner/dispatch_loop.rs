use super::batch_runtime::{divert_batch_effects, BatchRuntime};
use super::live_progress::LiveSystemBatchProgress;
use chrono::Utc;
use engine_logging::{engine_debug, engine_info, engine_warn};
use harvester_core::{update, AppState, BatchObservation, Msg};
use harvester_io::EffectRunner;
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
    pub(crate) enable_ai_orchestration: bool,
    pub(crate) require_new_jobs_since: Option<usize>,
    pub(crate) tick_interval: Duration,
}

pub(crate) const MAX_DISPATCH_INBOX_BATCH: usize = 32;
const MAX_BATCH_MSG_LOG_LEN: usize = 240;

/// Determines if the batch cycle should settle (all reducer-owned work quiesced).
pub(super) fn should_settle_cycle(status: harvester_core::BatchStatus) -> bool {
    matches!(status, harvester_core::BatchStatus::Settled)
}

pub(super) fn should_check_settlement_this_iteration(orchestrated: bool) -> bool {
    !orchestrated
}

pub(super) fn batch_buffer_is_quiescent(state: &AppState, buffered_ids: &HashSet<u64>) -> bool {
    !buffered_ids.is_empty()
        && state
            .pending_llm_request_ids()
            .all(|request_id| buffered_ids.contains(&request_id))
}

pub(super) fn should_run_ai_orchestration(
    enable_ai_orchestration: bool,
    require_new_jobs_since: Option<usize>,
    obs: &BatchObservation,
) -> bool {
    if !enable_ai_orchestration {
        return false;
    }
    match require_new_jobs_since {
        Some(baseline_jobs_total) => obs.jobs_total > baseline_jobs_total,
        None => true,
    }
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
        Msg::TriageArticlesLoaded { articles, .. } => {
            format!("TriageArticlesLoaded {{ articles: {} }}", articles.len())
        }
        Msg::TriageArticlesLoadProgress {
            files_scanned,
            files_total,
            ..
        } => format!(
            "TriageArticlesLoadProgress {{ files_scanned: {}, files_total: {} }}",
            files_scanned, files_total
        ),
        Msg::ArticlesLoaded { articles, .. } => {
            format!("ArticlesLoaded {{ articles: {} }}", articles.len())
        }
        Msg::PromptContextsLoaded { contexts } => {
            format!("PromptContextsLoaded {{ prompts: {} }}", contexts.len())
        }
        Msg::LlmMetadataLoaded {
            active_versions,
            effective_models,
            templates,
        } => format!(
            "LlmMetadataLoaded {{ active_versions: {}, effective_models: {}, templates: {} }}",
            active_versions.len(),
            effective_models.len(),
            templates.len()
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
    mut progress: Option<&mut LiveSystemBatchProgress>,
    mut batch_runtime: Option<&mut BatchRuntime>,
) -> Result<CycleOutcome, String> {
    let timeout = Duration::from_millis(100);
    let mut iterations = 0;
    let mut last_tick = Instant::now();
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

        // Receive at least one message with timeout, then drain a bounded batch.
        // Large restored states make reducer clones expensive; bounding the batch
        // keeps progress and tick-driven orchestration responsive under bursts.
        let mut recv_idle = false;
        let mut enqueued_effects = false;
        match msg_rx.recv_timeout(timeout) {
            Ok(first_msg) => {
                let mut inbox = vec![first_msg];
                while inbox.len() < MAX_DISPATCH_INBOX_BATCH {
                    let Ok(next_msg) = msg_rx.try_recv() else {
                        break;
                    };
                    inbox.push(next_msg);
                }

                let mut queued_effects = Vec::new();
                for msg in inbox {
                    if should_log_batch_msg(&msg) {
                        engine_debug!("[batch] Processing message: {}", summarize_batch_msg(&msg));
                    }
                    let (new_state, effects) = update(state.clone(), msg);
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

                if let Some(triggered_by_job_done) =
                    state.take_pre_triage_refresh_evaluation_request()
                {
                    let ordered_urls = state.ordered_completed_job_urls_snapshot();
                    let (new_state, effects) = update(
                        state.clone(),
                        Msg::EvaluatePreTriageRefresh {
                            ordered_urls,
                            triggered_by_job_done,
                        },
                    );
                    *state = new_state;
                    queued_effects.extend(effects);
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
                        effect_runner.enqueue(queued_effects);
                    }
                }
            }
            Err(mpsc::RecvTimeoutError::Timeout) => {
                recv_idle = true;
            }
            Err(mpsc::RecvTimeoutError::Disconnected) => {
                return Err("Message channel disconnected unexpectedly".to_string());
            }
        }

        if options.enable_ai_orchestration && last_tick.elapsed() >= options.tick_interval {
            let (new_state, tick_effects) = update(state.clone(), Msg::Tick);
            *state = new_state;
            if !tick_effects.is_empty() {
                let tick_effects = if let Some(batch) = batch_runtime.as_deref_mut() {
                    divert_batch_effects(state, tick_effects, batch, msg_tx)
                } else {
                    tick_effects
                };
                if !tick_effects.is_empty() {
                    enqueued_effects = true;
                    effect_runner.enqueue(tick_effects);
                }
            }
            last_tick = Instant::now();
        }

        // Check for settlement after processing available work.
        let mut orchestrated = false;
        if let Some(p) = progress.as_deref_mut() {
            let cost = batch_runtime
                .as_ref()
                .map_or(0, |batch| batch.realized_cost_microdollars);
            p.clear_phase_override();
            p.paint(state, cost, false);
        }
        let obs = state.batch_observation();
        if should_run_ai_orchestration(
            options.enable_ai_orchestration,
            options.require_new_jobs_since,
            &obs,
        ) {
            if let Some(next_msg) = maybe_dispatch_batch_ai_orchestration(state) {
                msg_tx.send(next_msg.clone()).map_err(|e| {
                    format!(
                        "Failed to dispatch orchestration message {:?}: {}",
                        next_msg, e
                    )
                })?;
                orchestrated = true;
            }
        }

        // This prevents an immediate idle-state exit before queued actions
        // (like PollSourcesClicked) have been reduced.
        if !orchestrated && recv_idle && !enqueued_effects {
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

        if should_check_settlement_this_iteration(orchestrated)
            && should_settle_cycle(state.batch_status())
        {
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

pub(crate) fn maybe_dispatch_batch_ai_orchestration(state: &AppState) -> Option<Msg> {
    match state.batch_next_action() {
        harvester_core::BatchNextAction::DispatchTriage => Some(Msg::TriageClicked),
        harvester_core::BatchNextAction::DispatchSummaries => Some(Msg::PrepareSummariesClicked),
        harvester_core::BatchNextAction::None => None,
    }
}
