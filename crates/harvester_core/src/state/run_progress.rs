use super::AppState;
use crate::{PipelineActivity, PipelineRunPhase, RunCompletionNotice, RunProgress, RunState};

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct PollPipelineJobSnapshot {
    pub url: Option<String>,
    pub total: u32,
    pub source_scan_done: bool,
}

impl AppState {
    pub fn pipeline_waves(&self) -> &crate::PipelineWaves {
        &self.pipeline_waves
    }

    pub fn pipeline_run_armed(&self) -> bool {
        self.run_progress_is_active() && self.pipeline_admission.as_ref().is_some_and(|r| r.armed)
    }

    pub(crate) fn pipeline_ready(&self) -> bool {
        self.pipeline_run_armed()
            && self
                .pipeline_admission
                .as_ref()
                .is_some_and(|r| r.configured)
    }
    pub fn pipeline_run_phase(&self) -> PipelineRunPhase {
        self.pipeline_run_phase
    }

    pub fn run_state(&self) -> RunState {
        if self.pipeline_run_phase == PipelineRunPhase::Stopping
            || self.session == crate::SessionState::Finishing
        {
            return RunState::Stopping {
                in_flight: self.stopping_in_flight_count(),
            };
        }
        if self.run_progress_is_active() {
            RunState::Active
        } else {
            RunState::Idle
        }
    }

    /// Export depends only on whether a pipeline run is working or draining.
    pub fn export_available(&self) -> bool {
        matches!(self.run_state(), RunState::Idle)
    }

    fn stopping_in_flight_count(&self) -> usize {
        let batch = self.batch_observation();
        let started_downloads = self.run_progress.as_ref().map_or(0, |run| {
            run.download_started_job_ids
                .difference(&run.download_finished_job_ids)
                .count()
        });
        started_downloads
            + self.article_model_requests_in_flight()
            + usize::from(batch.poll_in_progress)
            + usize::from(self.pipeline_activity().intake_refresh_pending)
            + usize::from(batch.import_in_flight)
    }

    pub fn run_progress(&self) -> Option<&RunProgress> {
        self.run_progress.as_ref()
    }

    /// Read-only progress snapshot, without building the rest of the desktop view.
    pub fn run_progress_view(&self) -> crate::RunProgressView {
        self.run_progress
            .as_ref()
            .map_or_else(Default::default, RunProgress::view)
    }

    pub fn run_completion_notice(&self) -> Option<&RunCompletionNotice> {
        self.run_completion_notice.as_ref()
    }

    /// Notice recorded when the current run first admits its prior window.
    pub fn reprocess_notice(&self) -> Option<(usize, u64)> {
        self.pipeline_admission
            .as_ref()
            .and_then(|run| run.reprocess_notice)
    }

    pub fn pipeline_activity(&self) -> PipelineActivity {
        let batch = self.batch_observation();
        PipelineActivity {
            intake_refresh_pending: self.pre_triage_refresh_eval_pending
                || self.processing_start.is_some()
                || self.pre_triage_coordinator.refresh_pending()
                || self.triage_in_flight_request_id().is_some(),
            poll_in_progress: usize::from(batch.poll_in_progress),
            jobs_pending_or_in_flight: batch.jobs_in_flight,
            pre_triage_loading: usize::from(matches!(
                batch.pre_triage_phase,
                crate::PreTriagePhase::LoadingArticles
            )),
            triage_pending_or_in_flight: (batch.triage_pending + batch.triage_in_flight)
                .max(usize::from(self.triage.is_active())),
            summary_pending_or_in_flight: batch.summary_pending + batch.summary_in_flight,
            signal_pending_or_in_flight: batch.signal_pending_or_in_flight,
            briefing_active: usize::from(self.briefing.is_active()),
            import_in_flight: usize::from(batch.import_in_flight),
        }
    }

    /// Outstanding side effects, excluding admitted work that has yet to dispatch.
    /// Hosts use this to distinguish quiet operations from stalled orchestration.
    pub fn pipeline_has_in_flight_work(&self) -> bool {
        self.source_states.is_poll_in_progress()
            || self.jobs.values().any(|job| job.outcome.is_none())
            || self.import_session.phase == crate::ImportPhase::Importing
            || self.triage_in_flight_request_id().is_some()
            || self
                .processing_start
                .as_ref()
                .is_some_and(|start| start.configuration_request.is_some())
            || self.pending_llm_request_ids().next().is_some()
            || matches!(self.briefing.phase(), crate::BriefingPhase::LoadingArticles)
    }

    pub(crate) fn run_progress_mut(&mut self) -> Option<&mut RunProgress> {
        self.run_progress.as_mut()
    }

    pub(crate) fn run_progress_is_active(&self) -> bool {
        self.run_progress.as_ref().is_some_and(|run| !run.terminal)
    }

    pub(crate) fn replace_run_progress(&mut self, progress: RunProgress) {
        self.run_progress = Some(progress);
    }

    pub(crate) fn allocate_run_id(&mut self) -> u64 {
        let run_id = self.next_run_id;
        self.next_run_id = self.next_run_id.wrapping_add(1);
        run_id
    }

    pub(crate) fn set_pipeline_run_phase(&mut self, phase: PipelineRunPhase) {
        self.pipeline_run_phase = phase;
    }

    pub(crate) fn pipeline_intake_open(&self) -> bool {
        self.pipeline_run_phase != PipelineRunPhase::Stopping
            && !matches!(
                self.session,
                crate::SessionState::Finishing | crate::SessionState::Finished
            )
            && (!self.run_progress_is_active()
                || self
                    .pipeline_admission
                    .as_ref()
                    .is_none_or(|run| run.intake_open))
    }

    pub(crate) fn set_run_completion_notice(&mut self, notice: RunCompletionNotice) {
        self.run_completion_notice = Some(notice);
    }

    pub(crate) fn clear_run_completion_notice(&mut self) -> bool {
        self.run_completion_notice.take().is_some()
    }

    pub(crate) fn poll_pipeline_job_snapshot(
        &self,
        job_id: crate::JobId,
        include_url: bool,
    ) -> Option<PollPipelineJobSnapshot> {
        let tracker = self.poll_pipeline.as_ref()?;
        if !tracker.job_ids.contains(&job_id) {
            return None;
        }
        Some(PollPipelineJobSnapshot {
            url: include_url
                .then(|| self.jobs.get(&job_id).map(|job| job.url.clone()))
                .flatten(),
            total: tracker.job_ids.len() as u32,
            source_scan_done: tracker.source_scan_done,
        })
    }

    pub(crate) fn poll_pipeline_job_total(&self) -> Option<u32> {
        self.poll_pipeline
            .as_ref()
            .map(|tracker| tracker.job_ids.len() as u32)
    }
}
