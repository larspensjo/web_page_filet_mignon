use crate::view_model::{AppViewModel, JobRowView};
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
        let jobs: Vec<JobRowView> = self.jobs.iter().map(|(id, job)| job.to_view(*id)).collect();
        AppViewModel {
            session: self.session,
            queued_urls: self.ui.urls.clone(),
            job_count: self.jobs.len(),
            jobs,
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

    pub(crate) fn enqueue_jobs_from_ui(&mut self) {
        for url in self.ui.urls.iter() {
            let job_id = self.next_job_id;
            self.next_job_id += 1;
            self.jobs.insert(
                job_id,
                JobState {
                    url: url.clone(),
                    stage: Stage::Queued,
                    outcome: None,
                    tokens: None,
                    bytes: None,
                },
            );
        }
        self.ui.urls.clear();
        self.dirty = true;
    }

    pub(crate) fn apply_progress(
        &mut self,
        job_id: JobId,
        stage: Stage,
        tokens: Option<u32>,
        bytes: Option<u64>,
    ) {
        if let Some(job) = self.jobs.get_mut(&job_id) {
            job.stage = stage;
            if let Some(t) = tokens {
                job.tokens = Some(t);
            }
            if let Some(b) = bytes {
                job.bytes = Some(b);
            }
            self.dirty = true;
        }
    }

    pub(crate) fn apply_done(&mut self, job_id: JobId, result: JobResultKind) {
        if let Some(job) = self.jobs.get_mut(&job_id) {
            job.stage = Stage::Done;
            job.outcome = Some(result);
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
    stage: Stage,
    outcome: Option<JobResultKind>,
    tokens: Option<u32>,
    bytes: Option<u64>,
}

impl JobState {
    fn to_view(&self, id: JobId) -> JobRowView {
        JobRowView {
            job_id: id,
            url: self.url.clone(),
            stage: self.stage,
            outcome: self.outcome,
            tokens: self.tokens,
            bytes: self.bytes,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
struct MetricsState {
    total_urls: usize,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
struct UiState {
    urls: Vec<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Stage {
    #[default]
    Queued,
    Downloading,
    Sanitizing,
    Converting,
    Tokenizing,
    Writing,
    Done,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum JobResultKind {
    Success,
    Failed,
}
