use super::{AppState, JobId, SessionState};
use crate::entity_index::EntityIndex;
use crate::preview::PreviewContentKind;
use crate::tabs::{AppTab, JobListScope, LeftTab, TrendCategory};
use crate::trends::EntityTrendData;
use crate::view_model::LastPasteStats;
use crate::view_model::{DEFAULT_JOBS_PANEL_WIDTH, DEFAULT_WINDOW_WIDTH};

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

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub(super) enum PreviewMode {
    #[default]
    Briefing,
    SelectedJob,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct UiState {
    pub(super) urls: Vec<String>,
    input_buffer: String,
    jobs_search_query: String,
    pub(super) preview: PreviewState,
    preview_mode: PreviewMode,
    left_panel_width: i32,
    input_panel_visible: bool,
    window_width: i32,
}

impl Default for UiState {
    fn default() -> Self {
        Self {
            urls: Vec::new(),
            input_buffer: String::new(),
            jobs_search_query: String::new(),
            preview: PreviewState::default(),
            preview_mode: PreviewMode::default(),
            left_panel_width: DEFAULT_JOBS_PANEL_WIDTH,
            input_panel_visible: false,
            window_width: DEFAULT_WINDOW_WIDTH,
        }
    }
}

impl UiState {
    pub(super) fn preview_content(&self) -> Option<&str> {
        self.preview.content()
    }

    pub(super) fn preview_mode(&self) -> PreviewMode {
        self.preview_mode
    }

    pub(super) fn set_preview_mode(&mut self, mode: PreviewMode) {
        self.preview_mode = mode;
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

    pub(super) fn left_panel_width(&self) -> i32 {
        self.left_panel_width
    }

    pub(super) fn input_panel_visible(&self) -> bool {
        self.input_panel_visible
    }

    pub(super) fn set_left_panel_width(&mut self, width: i32) {
        self.left_panel_width = width;
    }

    pub(super) fn set_input_panel_visible(&mut self, visible: bool) {
        self.input_panel_visible = visible;
    }

    pub(super) fn window_width(&self) -> i32 {
        self.window_width
    }

    pub(super) fn set_window_width(&mut self, width: i32) {
        self.window_width = width;
    }
}

impl AppState {
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
            );

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

    pub(crate) fn left_panel_width(&self) -> i32 {
        self.ui.left_panel_width()
    }

    pub(crate) fn input_panel_visible(&self) -> bool {
        self.ui.input_panel_visible()
    }

    pub(crate) fn set_left_panel_width(&mut self, width: i32) {
        self.ui.set_left_panel_width(width);
    }

    pub(crate) fn set_input_panel_visible(&mut self, visible: bool) {
        self.ui.set_input_panel_visible(visible);
    }

    pub(crate) fn window_width(&self) -> i32 {
        self.ui.window_width()
    }

    pub(crate) fn set_window_width(&mut self, width: i32) {
        self.ui.set_window_width(width);
    }

    pub(crate) fn select_tab(&mut self, tab: AppTab) {
        if self.active_tab != tab {
            self.active_tab = tab;
            self.dirty = true;
        }
    }

    pub fn active_tab(&self) -> AppTab {
        self.active_tab
    }

    pub(crate) fn select_left_tab(&mut self, tab: LeftTab) {
        if self.left_tab != tab {
            self.left_tab = tab;
            self.dirty = true;
        }
    }

    pub(crate) fn set_left_tab(&mut self, tab: LeftTab) {
        self.select_left_tab(tab);
    }

    pub fn left_tab(&self) -> LeftTab {
        self.left_tab
    }

    pub fn job_list_scope(&self) -> JobListScope {
        self.job_list_scope
    }

    pub(crate) fn set_job_list_scope(&mut self, scope: JobListScope) {
        if self.job_list_scope != scope {
            self.job_list_scope = scope;
            self.dirty = true;
        }
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
