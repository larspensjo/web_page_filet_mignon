use crate::view_model::AppViewModel;
use std::collections::BTreeMap;

pub type JobId = u64;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AppState {
    session: SessionState,
    jobs: BTreeMap<JobId, JobState>,
    metrics: MetricsState,
    ui: UiState,
    dirty: bool,
    next_job_id: JobId,
}

impl Default for AppState {
    fn default() -> Self {
        Self {
            session: SessionState::Idle,
            jobs: BTreeMap::new(),
            metrics: MetricsState::default(),
            ui: UiState::default(),
            dirty: false,
            next_job_id: 1,
        }
    }
}

impl AppState {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn view(&self) -> AppViewModel {
        AppViewModel {
            session: self.session,
            queued_urls: self.ui.urls.clone(),
            job_count: self.jobs.len(),
            dirty: self.dirty,
        }
    }

    /// Returns the current dirty flag and clears it in one step.
    pub fn consume_dirty(&mut self) -> bool {
        let was_dirty = self.dirty;
        self.dirty = false;
        was_dirty
    }

    pub(crate) fn session(&self) -> SessionState {
        self.session
    }

    pub(crate) fn set_urls(&mut self, urls: Vec<String>) {
        self.ui.urls = urls;
        self.metrics.total_urls = self.ui.urls.len();
        self.dirty = true;
    }

    pub(crate) fn start_session(&mut self) {
        self.session = SessionState::Running;
        self.dirty = true;
    }

    pub(crate) fn finish_session(&mut self) {
        self.session = SessionState::Finishing;
        self.dirty = true;
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum SessionState {
    #[default]
    Idle,
    Running,
    Finishing,
    Finished,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
struct JobState {
    url: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
struct MetricsState {
    total_urls: usize,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
struct UiState {
    urls: Vec<String>,
}
