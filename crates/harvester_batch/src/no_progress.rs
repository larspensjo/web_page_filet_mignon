use engine_logging::engine_warn;
use std::time::{Duration, Instant};

/// Quiet in-flight operations have their own deadlines. This watchdog detects
/// orchestration that has neither received a message nor dispatched active work.
pub(crate) const NO_PROGRESS_TIMEOUT: Duration = Duration::from_secs(60);

pub(crate) struct NoProgressWatchdog {
    idle_since: Instant,
}

impl NoProgressWatchdog {
    pub(crate) fn new(now: Instant) -> Self {
        Self { idle_since: now }
    }

    pub(crate) fn check(
        &mut self,
        now: Instant,
        received_message: bool,
        in_flight: bool,
        operation: impl FnOnce() -> String,
    ) -> Result<(), String> {
        if received_message || in_flight {
            self.idle_since = now;
        } else if now.saturating_duration_since(self.idle_since) >= NO_PROGRESS_TIMEOUT {
            let operation = operation();
            let reason = format!(
                "No progress for {} seconds while {operation}: no message and no in-flight work",
                NO_PROGRESS_TIMEOUT.as_secs()
            );
            engine_warn!("[no-progress-watchdog] {}", reason);
            return Err(reason);
        }
        Ok(())
    }
}

pub(crate) fn pipeline_operation(state: &harvester_core::AppState) -> String {
    let activity = state.pipeline_activity();
    format!(
        "advancing pipeline phase={:?} poll={} downloads={} refresh_pending={} triage={} summaries={} scoring={} import={}",
        state.pipeline_run_phase(), activity.poll_in_progress,
        activity.jobs_pending_or_in_flight, activity.intake_refresh_pending,
        activity.triage_pending_or_in_flight, activity.summary_pending_or_in_flight,
        activity.signal_pending_or_in_flight, activity.import_in_flight,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn quiet_downloads_can_outlast_the_old_iteration_cap() {
        let start = Instant::now();
        let mut watchdog = NoProgressWatchdog::new(start);
        for seconds in 1..=1_100 {
            watchdog
                .check(start + Duration::from_secs(seconds), false, true, || {
                    "downloading fixture URL".into()
                })
                .unwrap();
        }
    }

    #[test]
    fn messages_restart_the_no_progress_deadline() {
        let start = Instant::now();
        let mut watchdog = NoProgressWatchdog::new(start);
        watchdog
            .check(start + NO_PROGRESS_TIMEOUT, true, false, || {
                "triaging fixture article".into()
            })
            .unwrap();
        watchdog
            .check(
                start + NO_PROGRESS_TIMEOUT * 2 - Duration::from_millis(1),
                false,
                false,
                || "triaging fixture article".into(),
            )
            .unwrap();
        let error = watchdog
            .check(start + NO_PROGRESS_TIMEOUT * 2, false, false, || {
                "triaging fixture article".into()
            })
            .unwrap_err();
        assert!(error.contains("triaging fixture article"));
    }

    #[test]
    fn watchdog_fires_only_after_a_full_idle_duration_without_in_flight_work() {
        let start = Instant::now();
        let mut watchdog = NoProgressWatchdog::new(start);
        watchdog
            .check(start + NO_PROGRESS_TIMEOUT * 10, false, true, || {
                "importing fixture pages".into()
            })
            .unwrap();
        watchdog
            .check(
                start + NO_PROGRESS_TIMEOUT * 11 - Duration::from_millis(1),
                false,
                false,
                || "importing fixture pages".into(),
            )
            .unwrap();
        assert!(watchdog
            .check(start + NO_PROGRESS_TIMEOUT * 11, false, false, || {
                "importing fixture pages".into()
            })
            .is_err());
    }
}
