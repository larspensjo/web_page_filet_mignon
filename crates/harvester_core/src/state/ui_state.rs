use super::{AppState, JobId, SessionState};
use crate::view_model::LastPasteStats;

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub(super) struct MetricsState {
    pub(super) total_urls: usize,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub(super) struct UiState {
    pub(super) urls: Vec<String>,
    input_buffer: String,
    jobs_search_query: String,
    selected_job_id: Option<JobId>,
}

impl UiState {
    pub(super) fn selected_job_id(&self) -> Option<JobId> {
        self.selected_job_id
    }
    pub(super) fn select_job(&mut self, job_id: JobId) -> bool {
        let changed = self.selected_job_id != Some(job_id);
        self.selected_job_id = Some(job_id);
        changed
    }
    pub(super) fn clear_selection(&mut self) {
        self.selected_job_id = None;
    }

    pub(super) fn set_input_buffer(&mut self, text: String) {
        self.input_buffer = text;
    }

    pub(super) fn input_buffer(&self) -> &str {
        &self.input_buffer
    }

    pub(super) fn clear_input_buffer(&mut self) {
        self.input_buffer.clear();
    }

    pub(super) fn set_jobs_search_query(&mut self, text: String) {
        self.jobs_search_query = text;
    }

    pub(super) fn jobs_search_query(&self) -> &str {
        &self.jobs_search_query
    }

    pub(super) fn clear_jobs_search_query(&mut self) {
        self.jobs_search_query.clear();
    }
}

impl AppState {
    pub fn last_observed_utc(&self) -> Option<chrono::DateTime<chrono::Utc>> {
        self.last_observed_utc
    }

    pub(crate) fn observe_utc(&mut self, now: chrono::DateTime<chrono::Utc>) {
        self.last_observed_utc = Some(now);
        if self
            .saved_scope_clock
            .is_none_or(|previous| (now - previous).num_seconds().abs() >= 60)
        {
            self.rebuild_saved_results();
        }
    }

    pub fn job_list_mode(&self) -> crate::JobListMode {
        self.job_list_mode
    }
    pub(crate) fn set_job_list_mode(&mut self, mode: crate::JobListMode) {
        self.job_list_mode = mode;
    }
    pub fn consume_dirty(&mut self) -> bool {
        let was_dirty = self.dirty;
        self.dirty = false;
        was_dirty
    }

    pub fn briefing_session_can_start(&self) -> bool {
        self.briefing.can_start()
    }

    pub(crate) fn mark_dirty(&mut self) {
        self.dirty = true;
    }

    pub(crate) fn session(&self) -> SessionState {
        self.session
    }

    pub(crate) fn stop_finish_button_state(&self) -> crate::StopFinishButtonState {
        let batch = self.batch_observation();
        let has_active_work = batch.jobs_in_flight > 0
            || batch.poll_in_progress
            || batch.import_in_flight
            || self
                .signal_candidate()
                .observation_counts()
                .pending_or_in_flight
                > 0
            || matches!(
                batch.triage_phase,
                crate::TriagePhase::LoadingArticles | crate::TriagePhase::Triaging
            )
            || matches!(
                self.briefing.phase(),
                crate::BriefingPhase::LoadingArticles | crate::BriefingPhase::Summarizing
            );

        let pipeline_active = self.run_progress_is_active();
        let already_stopping = self.pipeline_run_phase() == crate::PipelineRunPhase::Stopping
            || matches!(
                self.session,
                SessionState::Finishing | SessionState::Finished
            );

        if (matches!(self.session, SessionState::Running) || pipeline_active)
            && has_active_work
            && !already_stopping
        {
            crate::StopFinishButtonState::Enabled {
                policy: crate::StopPolicy::Finish,
            }
        } else {
            crate::StopFinishButtonState::Disabled
        }
    }

    pub(crate) fn set_urls(&mut self, urls: Vec<String>) {
        self.ui.urls = urls;
        self.metrics.total_urls = self.ui.urls.len();
        self.dirty = true;
    }

    pub(crate) fn set_input_buffer(&mut self, text: String) {
        self.ui.set_input_buffer(text);
    }

    pub(crate) fn input_buffer(&self) -> &str {
        self.ui.input_buffer()
    }

    pub(crate) fn clear_input_buffer(&mut self) {
        self.ui.clear_input_buffer();
    }

    pub(crate) fn jobs_search_query(&self) -> &str {
        self.ui.jobs_search_query()
    }

    pub(crate) fn set_jobs_search_query(&mut self, text: String) {
        if self.ui.jobs_search_query() != text {
            self.ui.set_jobs_search_query(text);
            self.dirty = true;
        }
    }

    pub(crate) fn clear_jobs_search_query(&mut self) {
        if !self.ui.jobs_search_query().is_empty() {
            self.ui.clear_jobs_search_query();
            self.dirty = true;
        }
    }

    pub(crate) fn start_session(&mut self) {
        self.session = SessionState::Running;
        self.dirty = true;
    }

    pub(crate) fn finish_session(&mut self) {
        self.session = SessionState::Finishing;
        self.dirty = true;
    }

    pub(crate) fn reset_session_to_idle(&mut self) {
        if self.session != SessionState::Idle {
            self.session = SessionState::Idle;
            self.dirty = true;
        }
    }

    pub(crate) fn set_last_paste_stats(&mut self, enqueued: usize, skipped: usize) {
        self.last_paste_stats = Some(LastPasteStats { enqueued, skipped });
        self.dirty = true;
    }
}
