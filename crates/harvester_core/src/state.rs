use crate::view_model::{AppViewModel, JobRowView, LastPasteStats, PreviewHeaderView, TOKEN_LIMIT};
use std::collections::{BTreeMap, HashSet};

pub type JobId = u64;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CompletedJobSnapshot {
    pub url: String,
    pub tokens: Option<u32>,
    pub bytes: Option<u64>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct AppState {
    session: SessionState,
    jobs: BTreeMap<JobId, JobState>,
    metrics: MetricsState,
    ui: UiState,
    seen_urls: HashSet<String>,
    last_paste_stats: Option<LastPasteStats>,
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
            seen_urls: HashSet::new(),
            last_paste_stats: None,
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
        let preview_text = self.ui.preview_content().map(ToOwned::to_owned);
        let preview_header = self
            .ui
            .selected_job_id()
            .and_then(|job_id| self.jobs.get(&job_id))
            .map(|job| {
                let quality = job.preview_quality.unwrap_or_default();
                PreviewHeaderView {
                    domain: domain_from_url(&job.url),
                    tokens: job.tokens,
                    bytes: job.bytes,
                    stage: job.stage,
                    outcome: job.outcome,
                    heading_count: quality.heading_count,
                    link_density: quality.link_density,
                    nav_heavy: quality.nav_heavy(),
                }
            });
        AppViewModel {
            session: self.session,
            queued_urls: self.ui.urls.clone(),
            job_count: self.jobs.len(),
            jobs,
            last_paste_stats: self.last_paste_stats.clone(),
            dirty: self.dirty,
            total_tokens: self.metrics.total_tokens,
            token_limit: TOKEN_LIMIT,
            preview_text,
            preview_header,
        }
    }

    /// Returns the current dirty flag and clears it in one step.
    pub fn consume_dirty(&mut self) -> bool {
        let was_dirty = self.dirty;
        self.dirty = false;
        was_dirty
    }

    pub fn completed_jobs_snapshot(&self) -> Vec<CompletedJobSnapshot> {
        self.jobs
            .values()
            .filter(|job| job.outcome == Some(JobResultKind::Success))
            .map(|job| CompletedJobSnapshot {
                url: job.url.clone(),
                tokens: job.tokens,
                bytes: job.bytes,
            })
            .collect()
    }

    pub fn restore_completed_jobs(&mut self, entries: Vec<CompletedJobSnapshot>) {
        if entries.is_empty() {
            return;
        }

        self.jobs.clear();
        self.seen_urls.clear();
        self.metrics = MetricsState::default();
        self.ui.urls.clear();
        self.ui.clear_preview();
        self.last_paste_stats = None;
        self.next_job_id = 1;

        for entry in entries {
            let job_id = self.next_job_id;
            self.next_job_id += 1;
            self.jobs.insert(
                job_id,
                JobState {
                    url: entry.url.clone(),
                    stage: Stage::Done,
                    outcome: Some(JobResultKind::Success),
                    tokens: entry.tokens,
                    bytes: entry.bytes,
                    content_preview: None,
                    preview_quality: None,
                },
            );
            let normalized = normalize_url_for_dedupe(&entry.url);
            self.seen_urls.insert(normalized);
            if let Some(tokens) = entry.tokens {
                self.metrics.total_tokens = self.metrics.total_tokens.saturating_add(tokens as u64);
            }
        }

        self.metrics.total_urls = self.jobs.len();
        self.session = SessionState::Idle;
        self.dirty = true;
    }

    pub(crate) fn select_job(&mut self, job_id: JobId) {
        if let Some(job) = self.jobs.get(&job_id) {
            if self.ui.select_job(job_id, job.content_preview.as_deref()) {
                self.dirty = true;
            }
        }
    }

    pub(crate) fn session(&self) -> SessionState {
        self.session
    }

    pub(crate) fn set_urls(&mut self, urls: Vec<String>) {
        self.ui.urls = urls;
        self.metrics.total_urls = self.ui.urls.len();
        self.dirty = true;
    }

    pub(crate) fn enqueue_jobs_from_ui(&mut self) -> Vec<(JobId, String)> {
        let mut enqueued = Vec::new();
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
                    content_preview: None,
                    preview_quality: None,
                },
            );
            enqueued.push((job_id, url.clone()));
        }
        self.ui.urls.clear();
        self.dirty = true;
        enqueued
    }

    pub(crate) fn apply_progress(
        &mut self,
        job_id: JobId,
        stage: Stage,
        tokens: Option<u32>,
        bytes: Option<u64>,
        content_preview: Option<String>,
    ) {
        if let Some(job) = self.jobs.get_mut(&job_id) {
            job.stage = stage;
            if let Some(t) = tokens {
                if job.tokens != Some(t) {
                    let previous = job.tokens.unwrap_or(0) as u64;
                    self.metrics.total_tokens = self
                        .metrics
                        .total_tokens
                        .saturating_sub(previous)
                        .saturating_add(t as u64);
                    job.tokens = Some(t);
                }
            }
            if let Some(b) = bytes {
                job.bytes = Some(b);
            }
            if let Some(content) = content_preview {
                let selected = self.ui.selected_job_id() == Some(job_id);
                if selected {
                    self.ui.set_preview_state(PreviewState::InProgress {
                        job_id,
                        content: content.clone(),
                    });
                }
                job.set_preview_content(content);
            }
            self.dirty = true;
        }
    }

    pub(crate) fn apply_done(
        &mut self,
        job_id: JobId,
        result: JobResultKind,
        content_preview: Option<String>,
    ) {
        let job_updated = if let Some(job) = self.jobs.get_mut(&job_id) {
            job.stage = Stage::Done;
            job.outcome = Some(result);
            if matches!(result, JobResultKind::Success) {
                if let Some(content) = content_preview {
                    job.set_preview_content(content);
                }
            } else {
                job.clear_preview_content();
            }
            true
        } else {
            false
        };
        if job_updated && self.ui.selected_job_id() == Some(job_id) {
            let preview_content = self.jobs.get(&job_id).and_then(|job| job.content_preview());
            self.ui.select_job(job_id, preview_content);
        }
        if job_updated {
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

    /// Check if URL has been seen before. If not, insert it and return false.
    /// If yes, return true (indicating it should be skipped).
    pub(crate) fn is_url_seen(&mut self, normalized_url: &str) -> bool {
        !self.seen_urls.insert(normalized_url.to_owned())
    }
}

/// Normalize URL for deduplication: trim whitespace, lowercase, strip trailing `/`.
pub fn normalize_url_for_dedupe(url: &str) -> String {
    let trimmed = url.trim();
    let lowercased = trimmed.to_lowercase();
    lowercased.trim_end_matches('/').to_owned()
}

fn domain_from_url(url: &str) -> String {
    let trimmed = url.trim();
    let without_scheme = trimmed
        .find("://")
        .map(|pos| &trimmed[pos + 3..])
        .unwrap_or(trimmed);
    let host = without_scheme
        .split(|c: char| matches!(c, '/' | '?' | '#'))
        .next()
        .unwrap_or(without_scheme)
        .trim_end_matches('/');
    if host.is_empty() {
        trimmed.to_string()
    } else {
        host.to_string()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum SessionState {
    #[default]
    Idle,
    Running,
    /// Intake closed: ignore new URL ingestion while draining in-flight work.
    /// Do not auto-resume from this state unless a feature flag explicitly allows it.
    Finishing,
    Finished,
}

#[derive(Debug, Clone, PartialEq, Default)]
struct JobState {
    url: String,
    stage: Stage,
    outcome: Option<JobResultKind>,
    tokens: Option<u32>,
    bytes: Option<u64>,
    content_preview: Option<String>,
    preview_quality: Option<PreviewQuality>,
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

    #[allow(dead_code)]
    pub(crate) fn content_preview(&self) -> Option<&str> {
        self.content_preview.as_deref()
    }

    fn set_preview_content(&mut self, content: String) {
        self.preview_quality = Some(PreviewQuality::from_markdown(&content));
        self.content_preview = Some(content);
    }

    fn clear_preview_content(&mut self) {
        self.preview_quality = None;
        self.content_preview = None;
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
struct PreviewQuality {
    heading_count: usize,
    link_density: f64,
}

impl Default for PreviewQuality {
    fn default() -> Self {
        Self {
            heading_count: 0,
            link_density: 0.0,
        }
    }
}

impl PreviewQuality {
    const NAV_HEAVY_THRESHOLD: f64 = 0.3;

    fn from_markdown(content: &str) -> Self {
        let heading_count = content
            .lines()
            .filter(|line| line.trim_start().starts_with('#'))
            .count();
        let link_count = content
            .split('[')
            .skip(1)
            .filter(|segment| segment.contains("]("))
            .count();
        let word_count = content.split_whitespace().count();
        let link_density = if word_count > 0 {
            link_count as f64 / word_count as f64
        } else {
            0.0
        };
        Self {
            heading_count,
            link_density,
        }
    }

    fn nav_heavy(&self) -> bool {
        self.link_density > Self::NAV_HEAVY_THRESHOLD
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
struct MetricsState {
    total_urls: usize,
    total_tokens: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum PreviewState {
    Empty,
    Available { job_id: JobId, content: String },
    InProgress { job_id: JobId, content: String },
    Unavailable { job_id: JobId },
}

impl Default for PreviewState {
    fn default() -> Self {
        PreviewState::Empty
    }
}

impl PreviewState {
    fn job_id(&self) -> Option<JobId> {
        match self {
            PreviewState::Empty => None,
            PreviewState::Available { job_id, .. }
            | PreviewState::InProgress { job_id, .. }
            | PreviewState::Unavailable { job_id } => Some(*job_id),
        }
    }

    fn content(&self) -> Option<&str> {
        match self {
            PreviewState::Available { content, .. } | PreviewState::InProgress { content, .. } => {
                Some(content.as_str())
            }
            PreviewState::Empty | PreviewState::Unavailable { .. } => None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
struct UiState {
    urls: Vec<String>,
    preview: PreviewState,
}

impl UiState {
    fn preview_content(&self) -> Option<&str> {
        self.preview.content()
    }

    fn selected_job_id(&self) -> Option<JobId> {
        self.preview.job_id()
    }

    fn select_job(&mut self, job_id: JobId, content: Option<&str>) -> bool {
        let next_state = match content {
            Some(text) => PreviewState::Available {
                job_id,
                content: text.to_owned(),
            },
            None => PreviewState::Unavailable { job_id },
        };
        self.set_preview_state(next_state)
    }

    fn clear_preview(&mut self) -> bool {
        self.set_preview_state(PreviewState::Empty)
    }

    fn set_preview_state(&mut self, next: PreviewState) -> bool {
        if self.preview == next {
            false
        } else {
            self.preview = next;
            true
        }
    }
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{update, Msg};

    #[test]
    fn job_done_success_stores_preview() {
        let mut state = AppState::new();
        state.jobs.insert(
            1,
            JobState {
                url: "https://example.com".to_string(),
                stage: Stage::Queued,
                ..Default::default()
            },
        );
        state.apply_done(
            1,
            JobResultKind::Success,
            Some("preview content".to_string()),
        );
        let job = state.jobs.get(&1).expect("job exists");
        assert_eq!(job.content_preview(), Some("preview content"));
    }

    #[test]
    fn job_done_failure_clears_preview() {
        let mut state = AppState::new();
        state.jobs.insert(
            2,
            JobState {
                url: "https://example.com".to_string(),
                stage: Stage::Queued,
                content_preview: Some("old preview".to_string()),
                ..Default::default()
            },
        );
        state.apply_done(2, JobResultKind::Failed, Some("ignored".to_string()));
        let job = state.jobs.get(&2).expect("job exists");
        assert_eq!(job.content_preview(), None);
    }

    #[test]
    fn selecting_job_with_preview_updates_view_model() {
        let mut state = AppState::new();
        state.jobs.insert(
            3,
            JobState {
                url: "https://example.com/path".to_string(),
                stage: Stage::Done,
                content_preview: Some("preview content".to_string()),
                ..Default::default()
            },
        );
        let (state, _) = update(state, Msg::JobSelected { job_id: 3 });
        let view = state.view();
        assert_eq!(view.preview_text, Some("preview content".to_string()));
        assert_eq!(view.preview_header.as_ref().unwrap().domain, "example.com");
    }

    #[test]
    fn selecting_job_without_preview_only_sets_header() {
        let mut state = AppState::new();
        state.jobs.insert(
            4,
            JobState {
                url: "http://sub.example.net/a".to_string(),
                stage: Stage::Downloading,
                ..Default::default()
            },
        );
        let (state, _) = update(state, Msg::JobSelected { job_id: 4 });
        let view = state.view();
        assert_eq!(view.preview_text, None);
        let header = view.preview_header.expect("header should exist");
        assert_eq!(header.domain, "sub.example.net");
        assert_eq!(header.stage, Stage::Downloading);
    }

    #[test]
    fn selecting_same_job_twice_only_sets_dirty_once() {
        let mut state = AppState::new();
        state.jobs.insert(
            5,
            JobState {
                url: "https://repeat.example".to_string(),
                stage: Stage::Done,
                content_preview: Some("d".to_string()),
                ..Default::default()
            },
        );
        let (state, _) = update(state, Msg::JobSelected { job_id: 5 });
        let mut state = state;
        assert!(state.consume_dirty());
        let (state, _) = update(state, Msg::JobSelected { job_id: 5 });
        let mut state = state;
        assert!(!state.consume_dirty());
    }

    #[test]
    fn domain_from_url_handles_various_inputs() {
        assert_eq!(domain_from_url("https://example.com/"), "example.com");
        assert_eq!(domain_from_url("http://foo.bar/baz?qux"), "foo.bar");
        assert_eq!(domain_from_url("example.org/path"), "example.org");
        assert_eq!(domain_from_url(""), "");
    }

    #[test]
    fn job_progress_with_preview_updates_selected_preview() {
        let mut state = AppState::new();
        state.jobs.insert(
            6,
            JobState {
                url: "https://partial.example".to_string(),
                stage: Stage::Downloading,
                ..Default::default()
            },
        );

        let (state, _) = update(state, Msg::JobSelected { job_id: 6 });
        let (state, _) = update(
            state,
            Msg::JobProgress {
                job_id: 6,
                stage: Stage::Converting,
                tokens: None,
                bytes: None,
                content_preview: Some("live content".to_string()),
            },
        );

        let view = state.view();
        assert_eq!(view.preview_text, Some("live content".to_string()));
        let job = state.jobs.get(&6).expect("job exists");
        assert_eq!(job.content_preview(), Some("live content"));
    }

    #[test]
    fn job_progress_with_preview_stores_content_when_not_selected() {
        let mut state = AppState::new();
        state.jobs.insert(
            7,
            JobState {
                url: "https://unselected.example".to_string(),
                stage: Stage::Downloading,
                ..Default::default()
            },
        );

        let (state, _) = update(
            state,
            Msg::JobProgress {
                job_id: 7,
                stage: Stage::Converting,
                tokens: None,
                bytes: None,
                content_preview: Some("background content".to_string()),
            },
        );

        let view = state.view();
        assert_eq!(view.preview_text, None);
        let job = state.jobs.get(&7).expect("job exists");
        assert_eq!(job.content_preview(), Some("background content"));
    }

    #[test]
    fn job_done_after_inprogress_promotes_preview_to_available() {
        let mut state = AppState::new();
        state.jobs.insert(
            8,
            JobState {
                url: "https://final.example".to_string(),
                stage: Stage::Downloading,
                ..Default::default()
            },
        );

        let (state, _) = update(state, Msg::JobSelected { job_id: 8 });
        let (state, _) = update(
            state,
            Msg::JobProgress {
                job_id: 8,
                stage: Stage::Converting,
                tokens: None,
                bytes: None,
                content_preview: Some("partial".to_string()),
            },
        );
        let (state, _) = update(
            state,
            Msg::JobDone {
                job_id: 8,
                result: JobResultKind::Success,
                content_preview: Some("final".to_string()),
            },
        );

        let view = state.view();
        assert_eq!(view.preview_text, Some("final".to_string()));
        let header = view.preview_header.expect("header present");
        assert_eq!(header.stage, Stage::Done);
    }

    #[test]
    fn preview_quality_counts_headings_and_skips_nav_indicator_when_low_density() {
        let content =
            "# Title\n## Section\nBody text with a [link](http://example.com).\nMore words here.";
        let quality = PreviewQuality::from_markdown(content);
        assert_eq!(quality.heading_count, 2);
        assert!(!quality.nav_heavy());
    }

    #[test]
    fn preview_quality_marks_nav_heavy_when_link_density_high() {
        let content = "[a](x) [b](x) [c](x) [d](x) [e](x)";
        let quality = PreviewQuality::from_markdown(content);
        assert!(quality.nav_heavy());
    }
}
