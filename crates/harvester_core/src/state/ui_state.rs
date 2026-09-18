use super::{AppState, JobId, SessionState};
use crate::entity_index::EntityIndex;
use crate::preview::PreviewContentKind;
use crate::tabs::TrendCategory;
use crate::trends::EntityTrendData;
use crate::view_model::LastPasteStats;

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub(super) struct MetricsState {
    pub(super) total_urls: usize,
    pub(super) total_tokens: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub(super) enum PreviewState {
    #[default]
    Empty,
    Available {
        job_id: JobId,
        content: String,
        kind: PreviewContentKind,
    },
    InProgress {
        job_id: JobId,
        content: String,
    },
    Unavailable {
        job_id: JobId,
    },
}

impl PreviewState {
    pub(super) fn job_id(&self) -> Option<JobId> {
        match self {
            PreviewState::Empty => None,
            PreviewState::Available { job_id, .. }
            | PreviewState::InProgress { job_id, .. }
            | PreviewState::Unavailable { job_id } => Some(*job_id),
        }
    }

    pub(super) fn content(&self) -> Option<&str> {
        match self {
            PreviewState::Available { content, .. } | PreviewState::InProgress { content, .. } => {
                Some(content.as_str())
            }
            PreviewState::Empty | PreviewState::Unavailable { .. } => None,
        }
    }

    pub(super) fn content_kind(&self) -> Option<PreviewContentKind> {
        match self {
            PreviewState::Available { kind, .. } => Some(*kind),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub(super) struct UiState {
    pub(super) urls: Vec<String>,
    input_buffer: String,
    jobs_search_query: String,
    pub(super) preview: PreviewState,
}

impl UiState {
    pub(super) fn preview_content(&self) -> Option<&str> {
        self.preview.content()
    }

    pub(super) fn selected_job_id(&self) -> Option<JobId> {
        self.preview.job_id()
    }

    pub(super) fn select_job(
        &mut self,
        job_id: JobId,
        content: Option<(&str, PreviewContentKind)>,
    ) -> bool {
        let next_state = match content {
            Some((text, kind)) => PreviewState::Available {
                job_id,
                content: text.to_owned(),
                kind,
            },
            None => PreviewState::Unavailable { job_id },
        };
        self.set_preview_state(next_state)
    }

    pub(super) fn clear_preview(&mut self) -> bool {
        self.set_preview_state(PreviewState::Empty)
    }

    pub(super) fn set_preview_state(&mut self, next: PreviewState) -> bool {
        if self.preview == next {
            false
        } else {
            self.preview = next;
            true
        }
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
    }

    pub fn workspace_view(&self) -> crate::WorkspaceView {
        self.workspace_view
    }
    pub fn job_list_mode(&self) -> crate::JobListMode {
        self.job_list_mode
    }
    pub(crate) fn set_workspace_view(&mut self, view: crate::WorkspaceView) {
        self.workspace_view = view;
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
            || matches!(
                batch.triage_phase,
                crate::TriagePhase::LoadingArticles | crate::TriagePhase::Triaging
            )
            || matches!(
                self.briefing.phase(),
                crate::BriefingPhase::LoadingArticles
                    | crate::BriefingPhase::Summarizing
                    | crate::BriefingPhase::GeneratingBriefing
            )
            || self.briefing.next_item_in_flight();

        if matches!(self.session, SessionState::Running) && has_active_work {
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

    pub(crate) fn set_last_paste_stats(&mut self, enqueued: usize, skipped: usize) {
        self.last_paste_stats = Some(LastPasteStats { enqueued, skipped });
        self.dirty = true;
    }

    pub(crate) fn set_active_trend_category(&mut self, category: TrendCategory) {
        self.active_trend_category = category;
        self.dirty = true;
    }

    pub fn active_trend_category(&self) -> TrendCategory {
        self.active_trend_category
    }

    pub(crate) fn set_entity_index(&mut self, index: EntityIndex, window_weeks: u32, top_n: usize) {
        self.entity_trend_data = Some(crate::trends::compute_trends(&index, window_weeks, top_n));
        self.entity_index = Some(index);
        self.dirty = true;
    }

    pub fn entity_trend_data(&self) -> Option<&EntityTrendData> {
        self.entity_trend_data.as_ref()
    }
}
