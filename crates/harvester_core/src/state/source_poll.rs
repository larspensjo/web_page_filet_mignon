use super::{AppState, JobId, PollPipelineProgressState, SourceStateIndex};
use crate::SourceInstanceState;
use harvester_engine::SourceId;

impl AppState {
    /// Advance the logical tick counter by one. Called on every `Msg::Tick`.
    pub(crate) fn advance_tick(&mut self) {
        self.tick = self.tick.wrapping_add(1);
    }

    /// Return the current logical tick value.
    pub(crate) fn current_tick(&self) -> u64 {
        self.tick
    }

    #[allow(dead_code)]
    pub(crate) fn source_states(&self) -> &SourceStateIndex {
        &self.source_states
    }

    #[allow(dead_code)]
    pub(crate) fn source_state(&self, id: &SourceId) -> Option<&SourceInstanceState> {
        self.source_states.source_state(id)
    }

    #[allow(dead_code)]
    pub(crate) fn record_source_poll(&mut self, id: &SourceId, url_count: usize) {
        self.source_states.record_source_poll(id, url_count);
        self.dirty = true;
    }

    pub(crate) fn record_poll_stat(&mut self, stat: crate::SourcePollStat) {
        self.source_states.record_poll_stat(stat);
        self.dirty = true;
    }

    #[allow(dead_code)]
    pub(crate) fn record_source_error(&mut self, id: &SourceId, error: String) {
        self.source_states.record_source_error(id, error);
        self.dirty = true;
    }

    #[allow(dead_code)]
    pub(crate) fn start_poll(&mut self) -> bool {
        let started = self.source_states.start_poll();
        if started {
            self.poll_pipeline = Some(PollPipelineProgressState::default());
            self.dirty = true;
        }
        started
    }

    pub(crate) fn record_poll_pipeline_jobs(&mut self, job_ids: &[JobId]) {
        if job_ids.is_empty() {
            return;
        }
        if let Some(tracker) = &mut self.poll_pipeline {
            tracker.job_ids.extend(job_ids.iter().copied());
            self.dirty = true;
        }
    }

    pub(crate) fn set_poll_total(&mut self, total: usize) {
        self.source_states.set_poll_total(total);
        self.dirty = true;
    }

    #[allow(dead_code)]
    pub(crate) fn end_poll(&mut self) {
        self.source_states.end_poll();
        if let Some(tracker) = &mut self.poll_pipeline {
            tracker.source_scan_done = true;
        }
        self.clear_settled_poll_pipeline_if_complete();
        self.dirty = true;
    }

    #[allow(dead_code)]
    pub(crate) fn is_poll_in_progress(&self) -> bool {
        self.source_states.is_poll_in_progress()
    }

    pub(super) fn poll_pipeline_article_progress(&self) -> Option<(usize, usize)> {
        let tracker = self.poll_pipeline.as_ref()?;
        if !tracker.source_scan_done || tracker.job_ids.is_empty() {
            return None;
        }
        let total = tracker.job_ids.len();
        let settled = tracker
            .job_ids
            .iter()
            .filter(|job_id| {
                self.jobs
                    .get(job_id)
                    .and_then(|job| job.outcome.as_ref())
                    .is_some()
            })
            .count();
        (settled < total).then_some((settled, total))
    }

    pub(super) fn clear_settled_poll_pipeline_if_complete(&mut self) {
        let Some(tracker) = self.poll_pipeline.as_ref() else {
            return;
        };
        if !tracker.source_scan_done {
            return;
        }
        let all_settled = tracker.job_ids.iter().all(|job_id| {
            self.jobs
                .get(job_id)
                .and_then(|job| job.outcome.as_ref())
                .is_some()
        });
        if all_settled {
            self.poll_pipeline = None;
            self.dirty = true;
        }
    }

    pub(super) fn pre_triage_loading_operation_label(&self) -> String {
        match self.pre_triage_load_context.map(|context| context.reason) {
            Some(crate::pre_triage_coordinator::PreTriageRefreshReason::RestoreCompletedJobs) => {
                "Preparing triage list".to_string()
            }
            Some(crate::pre_triage_coordinator::PreTriageRefreshReason::JobDone) => {
                "Updating triage candidates".to_string()
            }
            None => "Preparing triage list".to_string(),
        }
    }
}
