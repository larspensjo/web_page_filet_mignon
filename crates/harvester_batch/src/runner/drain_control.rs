use super::batch_runtime::BatchRuntime;
use super::live_progress::{
    run_provider_wait_loop, wait_with_local_heartbeat, LiveSystemBatchProgress,
    ProviderWaitOutcome, BATCH_WAIT_INTERVAL,
};
use super::reporting::write_no_progress_bailout;
use crate::batch_coordinator::BatchPeek;
use crate::progress::{BatchDisplayPhase, SystemProgressClock};
use engine_logging::{engine_info, engine_warn};
use harvester_core::{AppState, BatchObservation};
use std::sync::atomic::{AtomicBool, Ordering};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum BatchWaitDecision {
    KeepWaiting,
    RunCollectCycle,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct BatchDrainSnapshot {
    pub(super) pending_manifest_batches: Vec<(String, Option<String>)>,
    pub(super) triage_deferred: usize,
    pub(super) summary_deferred: usize,
    pub(super) signal_deferred: usize,
}

pub(super) const MAX_CONSECUTIVE_BATCH_COLLECT_NO_PROGRESS: usize = 2;

#[derive(Default)]
pub(super) struct DrainControlState {
    collect_cycle_baseline: Option<BatchDrainSnapshot>,
    consecutive_no_progress_collect_cycles: usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum DrainControl {
    ContinueCollectCycle,
    Break,
    Proceed,
}

pub(super) fn evaluate_batch_drain(
    batch_api_enabled: bool,
    obs: &BatchObservation,
    batch_runtime: &mut Option<BatchRuntime>,
    shutdown_flag: &AtomicBool,
    state: &AppState,
    progress: &mut LiveSystemBatchProgress,
    control_state: &mut DrainControlState,
) -> DrainControl {
    if !batch_api_enabled {
        return DrainControl::Proceed;
    }

    let deferred_total = obs.triage_deferred + obs.summary_deferred + obs.signal_deferred;
    if shutdown_flag.load(Ordering::Relaxed) {
        engine_info!("[batch] Shutdown signal received, exiting");
        return DrainControl::Break;
    }
    if deferred_total == 0 {
        engine_info!("[batch] Batch API drain settled; exiting");
        return DrainControl::Break;
    }

    let Some(batch) = batch_runtime.as_mut() else {
        engine_warn!(
            "[batch-wait] {} deferred requests cannot be drained because Batch API runtime is unavailable",
            deferred_total
        );
        return DrainControl::Break;
    };
    let drain_snapshot = BatchDrainSnapshot {
        pending_manifest_batches: batch.coordinator.pending_manifest_batches(),
        triage_deferred: obs.triage_deferred,
        summary_deferred: obs.summary_deferred,
        signal_deferred: obs.signal_deferred,
    };
    let delay_before_peek = if let Some(before) = control_state.collect_cycle_baseline.take() {
        if batch_drain_made_progress(&before, &drain_snapshot) {
            control_state.consecutive_no_progress_collect_cycles = 0;
            false
        } else {
            control_state.consecutive_no_progress_collect_cycles += 1;
            true
        }
    } else {
        false
    };
    if should_exit_batch_drain_after_no_progress(
        control_state.consecutive_no_progress_collect_cycles,
    ) {
        engine_warn!(
            "[batch-wait] collect-only operation made no progress for {} consecutive cycles; exiting with deferred triage={} summaries={} signal={} pending_manifest_batches={:?}",
            control_state.consecutive_no_progress_collect_cycles,
            obs.triage_deferred,
            obs.summary_deferred,
            obs.signal_deferred,
            drain_snapshot.pending_manifest_batches
        );
        progress.suspend_for_output();
        if let Err(err) = write_no_progress_bailout(&mut std::io::stdout(), &drain_snapshot) {
            engine_warn!("[batch-progress] failed to print bailout summary: {}", err);
        }
        progress.resume(
            state,
            batch_runtime
                .as_ref()
                .map_or(0, |runtime| runtime.realized_cost_microdollars),
        );
        return DrainControl::Break;
    }
    if delay_before_peek {
        engine_warn!(
            "[batch-wait] collect-only operation made no progress; waiting {} minutes before retrying pending_manifest_batches={:?}",
            BATCH_WAIT_INTERVAL.as_secs() / 60,
            drain_snapshot.pending_manifest_batches
        );
        progress.clear_phase_override();
        let delay_cost = batch.realized_cost_microdollars;
        let delay_clock = SystemProgressClock;
        if wait_with_local_heartbeat(&delay_clock, shutdown_flag, BATCH_WAIT_INTERVAL, || {
            progress.paint(state, delay_cost, false)
        }) {
            progress.set_phase(BatchDisplayPhase::Interrupted);
            progress.paint(state, delay_cost, true);
            engine_info!("[batch] Shutdown during batch retry wait, exiting");
            return DrainControl::Break;
        }
    }
    let wait_cost = batch.realized_cost_microdollars;
    let clock = SystemProgressClock;
    match run_provider_wait_loop(
        &clock,
        shutdown_flag,
        progress.peeks.clone(),
        || {
            batch
                .runtime
                .block_on(batch.coordinator.peek_pending_batches())
        },
        |render| {
            let force = render.phase == BatchDisplayPhase::CheckingProvider;
            progress.set_provider_wait_render(&render);
            progress.paint(state, wait_cost, force);
        },
    ) {
        ProviderWaitOutcome::Collect(peeks) => {
            progress.set_provider_check(peeks);
            progress.set_phase(BatchDisplayPhase::Collecting);
            progress.paint(state, wait_cost, true);
            control_state.collect_cycle_baseline = Some(drain_snapshot);
            engine_info!(
                "[batch-wait] collection or reconciliation is ready; starting collect-only cycle"
            );
            DrainControl::ContinueCollectCycle
        }
        ProviderWaitOutcome::Shutdown => {
            progress.set_phase(BatchDisplayPhase::Interrupted);
            progress.paint(state, wait_cost, true);
            engine_info!("[batch] Shutdown during batch wait, exiting");
            DrainControl::Break
        }
    }
}

pub(super) fn is_terminal_batch_status(status: &openai_provider_kit::BatchLifecycle) -> bool {
    matches!(
        status,
        openai_provider_kit::BatchLifecycle::Completed
            | openai_provider_kit::BatchLifecycle::Failed
            | openai_provider_kit::BatchLifecycle::Expired
            | openai_provider_kit::BatchLifecycle::Cancelled
    )
}

pub(super) fn decide_batch_wait(peeks: &[BatchPeek]) -> BatchWaitDecision {
    if peeks.is_empty()
        || peeks
            .iter()
            .filter_map(|peek| peek.status.as_ref())
            .any(is_terminal_batch_status)
    {
        BatchWaitDecision::RunCollectCycle
    } else {
        BatchWaitDecision::KeepWaiting
    }
}

pub(super) fn batch_drain_made_progress(
    before: &BatchDrainSnapshot,
    after: &BatchDrainSnapshot,
) -> bool {
    before != after
}

pub(super) fn should_exit_batch_drain_after_no_progress(consecutive_cycles: usize) -> bool {
    consecutive_cycles >= MAX_CONSECUTIVE_BATCH_COLLECT_NO_PROGRESS
}
