use super::AppState;
use crate::{PipelineActivity, PipelineRunPhase, RunCompletionNotice, RunProgress};

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct PollPipelineJobSnapshot {
    pub url: Option<String>,
    pub total: u32,
    pub source_scan_done: bool,
}

impl AppState {
    pub fn pipeline_run_phase(&self) -> PipelineRunPhase {
        self.pipeline_run_phase
    }

    pub fn run_progress(&self) -> Option<&RunProgress> {
        self.run_progress.as_ref()
    }

    pub fn run_completion_notice(&self) -> Option<&RunCompletionNotice> {
        self.run_completion_notice.as_ref()
    }

    pub fn pipeline_activity(&self) -> PipelineActivity {
        let batch = self.batch_observation();
        PipelineActivity {
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

    pub(crate) fn reduced_message_seq(&self) -> u64 {
        self.reduced_message_seq
    }

    pub(crate) fn note_reduced_work_message(&mut self) {
        self.reduced_message_seq = self.reduced_message_seq.wrapping_add(1);
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
