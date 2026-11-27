use super::dispatch_loop::reduce_and_dispatch;
use crate::{
    archive_result::{ArchiveResult, Status},
    no_progress::NoProgressWatchdog,
};
use chrono::{DateTime, Utc};
use engine_logging::engine_warn;
use harvester_core::{AppState, Effect, Msg};
use harvester_engine::ArchiveWindow;
use harvester_io::{archive_observation::published_article_count, EffectRunner};
use std::{
    path::Path,
    sync::{
        atomic::{AtomicBool, Ordering},
        mpsc,
    },
    time::{Duration, Instant},
};

pub(super) fn cycle_failure(state: &AppState, reason: &'static str) -> ArchiveResult {
    let mut result = ArchiveResult::new(Status::CycleFailed, Some(reason));
    result.since_utc = state.batch_archive_since_utc();
    result.failed_sources = state.batch_archive_failed_sources();
    result
}

fn submission_time(previous_until: Option<DateTime<Utc>>, shutdown: &AtomicBool) -> DateTime<Utc> {
    loop {
        let now = Utc::now();
        if previous_until != Some(ArchiveWindow::from_submission(None, now).until_utc())
            || shutdown.load(Ordering::Relaxed)
        {
            return now;
        }
        // A previous file with the same bound could masquerade as the new publication.
        std::thread::sleep(Duration::from_millis(50));
    }
}

#[allow(clippy::too_many_arguments)]
pub(super) fn finish_archive(
    state: &mut AppState,
    target: &Path,
    rx: &mpsc::Receiver<Msg>,
    runner: &EffectRunner,
    sink: &mut dyn FnMut(Vec<Effect>),
    observer: &mut dyn FnMut(&str, Duration, &AppState),
    shutdown: &AtomicBool,
    timeout: Duration,
) -> ArchiveResult {
    let submitted_at = submission_time(state.batch_archive_previous_until_utc(), shutdown);
    let mut interrupted = shutdown.load(Ordering::Relaxed);
    if interrupted {
        reduce_and_dispatch(state, Msg::BatchArchiveInterrupted, sink, observer);
    }
    reduce_and_dispatch(
        state,
        Msg::BatchArchiveRequested {
            request_id: 1,
            submitted_at,
        },
        sink,
        observer,
    );

    let mut acknowledgement_lost = false;
    let mut publication_started = false;
    let mut last_publication_step_at = None;
    let mut channel_closed = false;
    let receive_interval = Duration::from_millis(100).min(timeout);
    let mut watchdog = NoProgressWatchdog::with_timeout(Instant::now(), timeout);
    loop {
        if let Some(outcome) = state.batch_archive_outcome() {
            let mut result = ArchiveResult::from_outcome(outcome);
            result.since_utc = state.batch_archive_since_utc();
            if acknowledgement_lost {
                result.reason = Some("export_ack_lost");
            }
            return result;
        }
        publication_started |= state.batch_archive_publication().is_some();
        let reply = if channel_closed {
            std::thread::sleep(receive_interval);
            None
        } else {
            Some(rx.recv_timeout(receive_interval))
        };
        // Sample the signal before reducing a reply that could release the intent.
        if !publication_started && !interrupted && shutdown.load(Ordering::Relaxed) {
            interrupted = true;
            reduce_and_dispatch(state, Msg::BatchArchiveInterrupted, sink, observer);
        }
        let received_progress = match reply {
            Some(Ok(message)) => {
                let progressed = matches!(
                    message,
                    Msg::ArchiveExportProgress { .. }
                        | Msg::BatchArchivePublished { .. }
                        | Msg::BatchArchivePublicationFailed { .. }
                        | Msg::PortfolioRecordSaved { .. }
                );
                reduce_and_dispatch(state, message, sink, observer);
                progressed
            }
            Some(Err(mpsc::RecvTimeoutError::Disconnected)) => {
                engine_warn!(
                    "[morning-archive] target={} reply channel closed",
                    target.display()
                );
                channel_closed = true;
                false
            }
            Some(Err(mpsc::RecvTimeoutError::Timeout)) | None => false,
        };
        let publication = state.batch_archive_publication();
        let (step_at, can_still_publish) = publication
            .and_then(|(request_id, _)| runner.archive_publication_progress(request_id))
            .map_or((None, false), |(step_at, active)| (Some(step_at), active));
        let step_progressed = step_at.is_some() && step_at != last_publication_step_at;
        last_publication_step_at = step_at;
        // Recovery must wait until the worker can no longer replace the target.
        if (!channel_closed || can_still_publish)
            && watchdog
                .check(
                    Instant::now(),
                    received_progress || step_progressed,
                    can_still_publish,
                    || {
                        format!(
                            "waiting for archive or record acknowledgement target={}",
                            target.display()
                        )
                    },
                )
                .is_ok()
        {
            continue;
        }
        if let Some((request_id, window)) = publication {
            acknowledgement_lost = true;
            let message = recover_publication(runner, target, request_id, window);
            reduce_and_dispatch(state, message, sink, observer);
            watchdog = NoProgressWatchdog::with_timeout(Instant::now(), timeout);
        } else if let Some(save_id) = state.batch_archive_pending_record_save() {
            let receipt = if channel_closed && publication_started {
                wait_for_save_receipt(runner, save_id, timeout)
            } else {
                runner.portfolio_save_receipt(save_id)
            };
            reduce_and_dispatch(
                state,
                Msg::PortfolioRecordSaved {
                    save_id,
                    result: receipt.unwrap_or_else(|| {
                        Err("the record save acknowledgement did not arrive".into())
                    }),
                },
                sink,
                observer,
            );
        } else if state.batch_archive_outcome().is_none() {
            return cycle_failure(state, "cycle_stalled");
        }
    }
}

fn wait_for_save_receipt(
    runner: &EffectRunner,
    save_id: u64,
    timeout: Duration,
) -> Option<Result<(), String>> {
    let started = Instant::now();
    loop {
        let receipt = runner.portfolio_save_receipt(save_id);
        let remaining = timeout.saturating_sub(started.elapsed());
        if receipt.is_some() || remaining.is_zero() {
            return receipt;
        }
        std::thread::sleep(Duration::from_millis(50).min(remaining));
    }
}

fn recover_publication(
    runner: &EffectRunner,
    target: &Path,
    request_id: u64,
    window: ArchiveWindow,
) -> Msg {
    let doc_count = published_article_count(target, window.until_utc());
    let (previous_kept, previous_since) = runner
        .archive_publication_previous(request_id)
        .map_or((None, None), |receipt| {
            (receipt.previous_kept, receipt.previous_since_utc)
        });
    let previous_superseded = previous_kept.is_some()
        && window
            .since_utc()
            .zip(previous_since)
            .is_some_and(|(since, old)| since <= old);
    engine_warn!(
        "[morning-archive] target={} reason=export_ack_lost request_id={request_id} pending_until={} published={}",
        target.display(), window.until_utc(), doc_count.is_some()
    );
    if let Some(doc_count) = doc_count {
        Msg::BatchArchivePublished {
            request_id,
            doc_count,
            previous_kept,
            previous_superseded,
        }
    } else {
        Msg::BatchArchivePublicationFailed {
            request_id,
            reason: "export acknowledgement lost and the target index did not match the intent"
                .into(),
            previous_kept,
            previous_superseded,
        }
    }
}
