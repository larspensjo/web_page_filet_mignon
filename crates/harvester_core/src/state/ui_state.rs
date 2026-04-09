use super::JobId;
use crate::preview::PreviewContentKind;
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