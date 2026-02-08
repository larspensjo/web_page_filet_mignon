use crate::url_age::{guess_age_from_url, AgeEstimate};
use crate::view_model::{
    AppViewModel, JobRowView, LastPasteStats, LinkRowView, PreviewHeaderView, TOKEN_LIMIT,
};
use harvester_engine::{truncate_to_char_boundary, ExtractedLink, LinkKind};
use std::collections::{BTreeMap, HashSet};
use std::path::PathBuf;
use url::Url;

pub type JobId = u64;

const MAX_EXTRACTED_LINKS: usize = 5_000;
const LINK_ROW_LIMIT: usize = 200;
const LINK_LABEL_MAX: usize = 80;
const LINK_LABEL_TRUNCATE_MARKER: &str = "…";

/// Represents the download status for a specific link.
#[allow(dead_code)]
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LinkDownloadState {
    NotDownloaded,
    Downloading,
    Downloaded { path: PathBuf },
    Failed { error: String },
}

/// Canonical representation of a link extracted from a completed job.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LinkRecord {
    pub index: u32,
    pub url: String,
    pub anchor_text: Option<String>,
    pub kind: LinkKind,
    pub download_state: LinkDownloadState,
    pub age_estimate: Option<AgeEstimate>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LinkSnapshotRecord {
    pub url: String,
    pub downloaded_path: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CompletedJobSnapshot {
    pub url: String,
    pub tokens: Option<u32>,
    pub bytes: Option<u64>,
    pub links: Vec<LinkSnapshotRecord>,
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
                    outcome: job.outcome.clone(),
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
            left_panel_width: self.ui.left_panel_width(),
            window_width: self.ui.window_width(),
        }
    }

    /// Returns the current dirty flag and clears it in one step.
    pub fn consume_dirty(&mut self) -> bool {
        let was_dirty = self.dirty;
        self.dirty = false;
        was_dirty
    }

    /// Marks the state as dirty, signaling that a re-render is needed.
    pub(crate) fn mark_dirty(&mut self) {
        self.dirty = true;
    }

    pub fn completed_jobs_snapshot(&self) -> Vec<CompletedJobSnapshot> {
        self.jobs
            .values()
            .filter(|job| job.outcome == Some(JobResultKind::Success))
            .map(|job| CompletedJobSnapshot {
                url: job.url.clone(),
                tokens: job.tokens,
                bytes: job.bytes,
                links: job
                    .links
                    .iter()
                    .map(|link| LinkSnapshotRecord {
                        url: link.url.clone(),
                        downloaded_path: match &link.download_state {
                            LinkDownloadState::Downloaded { path } => {
                                Some(path.to_string_lossy().to_string())
                            }
                            _ => None,
                        },
                    })
                    .collect(),
            })
            .collect()
    }

    #[allow(dead_code)]
    pub fn job_links(&self, job_id: JobId) -> Option<&[LinkRecord]> {
        self.jobs.get(&job_id).map(|job| job.links())
    }

    pub(crate) fn restore_completed_jobs(&mut self, entries: Vec<CompletedJobSnapshot>) {
        if entries.is_empty() {
            return;
        }

        self.jobs.clear();
        self.seen_urls.clear();
        self.metrics = MetricsState::default();
        self.ui.urls.clear();
        self.ui.clear_preview();
        self.ui.clear_input_buffer();
        self.last_paste_stats = None;
        self.next_job_id = 1;

        for entry in entries {
            let CompletedJobSnapshot {
                url,
                tokens,
                bytes,
                links: link_snapshots,
            } = entry;
            let job_id = self.next_job_id;
            self.next_job_id += 1;
            self.jobs.insert(
                job_id,
                JobState {
                    url: url.clone(),
                    stage: Stage::Done,
                    outcome: Some(JobResultKind::Success),
                    tokens,
                    bytes,
                    content_preview: None,
                    preview_quality: None,
                    links: Vec::new(),
                },
            );
            let extracted_links: Vec<ExtractedLink> = link_snapshots
                .iter()
                .map(|record| ExtractedLink {
                    url: record.url.clone(),
                    text: None,
                    kind: LinkKind::Hyperlink,
                })
                .collect();
            if let Some(job) = self.jobs.get_mut(&job_id) {
                job.attach_extracted_links(extracted_links);
                job.apply_link_snapshots(&link_snapshots);
            }
            let normalized = normalize_url_for_dedupe(&url);
            self.seen_urls.insert(normalized);
            if let Some(tokens) = tokens {
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

    pub(crate) fn link_metadata(
        &self,
        job_id: JobId,
        link_index: u32,
    ) -> Option<(String, Option<PathBuf>)> {
        self.jobs.get(&job_id).and_then(|job| {
            job.links
                .iter()
                .find(|record| record.index == link_index)
                .map(|record| {
                    (
                        record.url.clone(),
                        match &record.download_state {
                            LinkDownloadState::Downloaded { path } => Some(path.clone()),
                            _ => None,
                        },
                    )
                })
        })
    }

    pub fn link_state(&self, job_id: JobId, link_index: u32) -> Option<(LinkDownloadState, bool)> {
        self.jobs.get(&job_id).and_then(|job| {
            job.links
                .iter()
                .find(|record| record.index == link_index)
                .map(|record| (record.download_state.clone(), record.age_estimate.is_some()))
        })
    }

    pub fn set_link_age_estimate(
        &mut self,
        job_id: JobId,
        link_index: u32,
        estimate: Option<AgeEstimate>,
    ) -> bool {
        if let Some(job) = self.jobs.get_mut(&job_id) {
            if let Some(record) = job
                .links
                .iter_mut()
                .find(|record| record.index == link_index)
            {
                record.age_estimate = estimate;
                self.dirty = true;
                return true;
            }
        }
        false
    }

    pub(crate) fn mark_link_download_requested(&mut self, job_id: JobId, link_index: u32) -> bool {
        if let Some(job) = self.jobs.get_mut(&job_id) {
            job.mark_link_download_requested(link_index);
            self.dirty = true;
            true
        } else {
            false
        }
    }

    pub(crate) fn mark_link_download_completed(
        &mut self,
        job_id: JobId,
        link_index: u32,
        path: PathBuf,
    ) -> bool {
        if let Some(job) = self.jobs.get_mut(&job_id) {
            job.mark_link_download_completed(link_index, path);
            self.dirty = true;
            true
        } else {
            false
        }
    }

    pub(crate) fn mark_link_download_failed(
        &mut self,
        job_id: JobId,
        link_index: u32,
        error: String,
    ) -> bool {
        if let Some(job) = self.jobs.get_mut(&job_id) {
            job.mark_link_download_failed(link_index, error);
            self.dirty = true;
            true
        } else {
            false
        }
    }

    pub(crate) fn mark_link_deleted(&mut self, job_id: JobId, link_index: u32) -> bool {
        if let Some(job) = self.jobs.get_mut(&job_id) {
            job.mark_link_deleted(link_index);
            self.dirty = true;
            true
        } else {
            false
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

    pub(crate) fn set_input_buffer(&mut self, text: String) {
        self.ui.set_input_buffer(text);
    }

    pub(crate) fn input_buffer(&self) -> &str {
        self.ui.input_buffer()
    }

    pub(crate) fn clear_input_buffer(&mut self) {
        self.ui.clear_input_buffer();
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
                    links: Vec::new(),
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
        extracted_links: Vec<ExtractedLink>,
    ) {
        let job_updated = if let Some(job) = self.jobs.get_mut(&job_id) {
            job.stage = Stage::Done;
            job.outcome = Some(result);
            if matches!(job.outcome.as_ref(), Some(JobResultKind::Success)) {
                if let Some(content) = content_preview {
                    job.set_preview_content(content);
                }
                job.attach_extracted_links(extracted_links);
            } else {
                job.clear_preview_content();
                job.clear_links();
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

    /// Returns the current left panel width (PANEL_INPUT + PANEL_JOBS combined).
    pub(crate) fn left_panel_width(&self) -> i32 {
        self.ui.left_panel_width()
    }

    /// Sets the left panel width.
    pub(crate) fn set_left_panel_width(&mut self, width: i32) {
        self.ui.set_left_panel_width(width);
    }

    /// Returns the current window width.
    pub(crate) fn window_width(&self) -> i32 {
        self.ui.window_width()
    }

    /// Sets the window width.
    pub(crate) fn set_window_width(&mut self, width: i32) {
        self.ui.set_window_width(width);
    }
}

/// Normalize URL for deduplication: trim whitespace, lowercase, strip trailing `/`.
pub fn normalize_url_for_dedupe(url: &str) -> String {
    let trimmed = url.trim();
    let lowercased = trimmed.to_lowercase();
    lowercased.trim_end_matches('/').to_owned()
}

fn normalize_extracted_link(link: &str) -> String {
    let trimmed = link.trim();
    if trimmed.is_empty() {
        return trimmed.to_string();
    }
    if let Ok(mut parsed) = Url::parse(trimmed) {
        parsed.set_fragment(None);
        if let Some(port) = parsed.port() {
            let normalized_port = match parsed.scheme() {
                "http" if port == 80 => None,
                "https" if port == 443 => None,
                _ => Some(port),
            };
            let _ = parsed.set_port(normalized_port);
        }
        parsed.into()
    } else {
        trimmed.to_string()
    }
}

fn domain_from_url(url: &str) -> String {
    let trimmed = url.trim();
    let without_scheme = trimmed
        .find("://")
        .map(|pos| &trimmed[pos + 3..])
        .unwrap_or(trimmed);
    let host = without_scheme
        .split(['/', '?', '#'])
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
    links: Vec<LinkRecord>,
}

impl JobState {
    fn to_view(&self, id: JobId) -> JobRowView {
        let links = build_link_rows(&self.links);
        let downloaded_link_count = self
            .links
            .iter()
            .filter(|link| matches!(link.download_state, LinkDownloadState::Downloaded { .. }))
            .count();
        JobRowView {
            job_id: id,
            url: self.url.clone(),
            stage: self.stage,
            outcome: self.outcome.clone(),
            tokens: self.tokens,
            bytes: self.bytes,
            link_count: self.links.len(),
            downloaded_link_count,
            links,
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
    #[allow(dead_code)]
    fn links(&self) -> &[LinkRecord] {
        &self.links
    }

    #[allow(dead_code)]
    fn clear_links(&mut self) {
        self.links.clear();
    }

    fn attach_extracted_links(&mut self, links: Vec<ExtractedLink>) {
        self.links.clear();
        let mut seen = HashSet::new();
        for (idx, link) in links.into_iter().enumerate() {
            if self.links.len() >= MAX_EXTRACTED_LINKS {
                break;
            }
            let canonical = normalize_extracted_link(&link.url);
            if canonical.is_empty() {
                continue;
            }
            if !seen.insert(canonical.clone()) {
                continue;
            }
            self.links.push(LinkRecord {
                index: idx as u32,
                url: canonical.clone(),
                anchor_text: link.text,
                kind: link.kind,
                download_state: LinkDownloadState::NotDownloaded,
                age_estimate: guess_age_from_url(&canonical),
            });
        }
    }

    fn apply_link_snapshots(&mut self, snapshots: &[LinkSnapshotRecord]) {
        for snapshot in snapshots {
            if let Some(path) = snapshot.downloaded_path.as_ref() {
                let canonical = normalize_extracted_link(&snapshot.url);
                if canonical.is_empty() {
                    continue;
                }
                if let Some(record) = self.links.iter_mut().find(|record| record.url == canonical) {
                    record.download_state = LinkDownloadState::Downloaded {
                        path: PathBuf::from(path),
                    };
                }
            }
        }
    }

    #[allow(dead_code)]
    fn find_link_mut(&mut self, link_index: u32) -> Option<&mut LinkRecord> {
        self.links
            .iter_mut()
            .find(|record| record.index == link_index)
    }

    #[allow(dead_code)]
    fn mark_link_download_requested(&mut self, link_index: u32) {
        if let Some(record) = self.find_link_mut(link_index) {
            record.download_state = LinkDownloadState::Downloading;
        }
    }

    #[allow(dead_code)]
    fn mark_link_download_completed(&mut self, link_index: u32, path: PathBuf) {
        if let Some(record) = self.find_link_mut(link_index) {
            record.download_state = LinkDownloadState::Downloaded { path };
        }
    }

    #[allow(dead_code)]
    fn mark_link_download_failed(&mut self, link_index: u32, error: String) {
        if let Some(record) = self.find_link_mut(link_index) {
            record.download_state = LinkDownloadState::Failed { error };
        }
    }

    #[allow(dead_code)]
    fn mark_link_deleted(&mut self, link_index: u32) {
        if let Some(record) = self.find_link_mut(link_index) {
            record.download_state = LinkDownloadState::NotDownloaded;
        }
    }
}

fn build_link_rows(records: &[LinkRecord]) -> Vec<LinkRowView> {
    records
        .iter()
        .take(LINK_ROW_LIMIT)
        .map(|record| LinkRowView {
            index: record.index,
            url: record.url.clone(),
            label: link_label_for_record(record),
            kind: record.kind.clone(),
            download_state: record.download_state.clone(),
            age_suspect: record.age_estimate.is_some(),
        })
        .collect()
}

fn link_label_for_record(record: &LinkRecord) -> String {
    if let Some(text) = record
        .anchor_text
        .as_deref()
        .map(str::trim)
        .filter(|text| !text.is_empty())
    {
        text.to_string()
    } else {
        truncate_link_url(&record.url)
    }
}

fn truncate_link_url(url: &str) -> String {
    if url.chars().count() <= LINK_LABEL_MAX {
        url.to_string()
    } else {
        let max_chars = LINK_LABEL_MAX
            .saturating_sub(LINK_LABEL_TRUNCATE_MARKER.len())
            .max(1);
        let truncated = truncate_to_char_boundary(url, max_chars);
        format!("{truncated}{LINK_LABEL_TRUNCATE_MARKER}")
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

#[derive(Debug, Clone, PartialEq, Eq, Default)]
enum PreviewState {
    #[default]
    Empty,
    Available {
        job_id: JobId,
        content: String,
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

// Default left panel width (PANEL_INPUT + PANEL_JOBS = 320 + 280)
const DEFAULT_LEFT_PANEL_WIDTH: i32 = 600;
// Default window width
const DEFAULT_WINDOW_WIDTH: i32 = 960;

#[derive(Debug, Clone, PartialEq, Eq)]
struct UiState {
    urls: Vec<String>,
    input_buffer: String,
    preview: PreviewState,
    left_panel_width: i32,
    window_width: i32,
}

impl Default for UiState {
    fn default() -> Self {
        Self {
            urls: Vec::new(),
            input_buffer: String::new(),
            preview: PreviewState::default(),
            left_panel_width: DEFAULT_LEFT_PANEL_WIDTH,
            window_width: DEFAULT_WINDOW_WIDTH,
        }
    }
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

    fn set_input_buffer(&mut self, text: String) {
        self.input_buffer = text;
    }

    fn input_buffer(&self) -> &str {
        &self.input_buffer
    }

    fn clear_input_buffer(&mut self) {
        self.input_buffer.clear();
    }

    fn left_panel_width(&self) -> i32 {
        self.left_panel_width
    }

    fn set_left_panel_width(&mut self, width: i32) {
        self.left_panel_width = width;
    }

    fn window_width(&self) -> i32 {
        self.window_width
    }

    fn set_window_width(&mut self, width: i32) {
        self.window_width = width;
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

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum JobResultKind {
    Success,
    Failed { reason: String },
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{update, Msg};
    use harvester_engine::{ExtractedLink, LinkKind};

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
            Vec::new(),
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
        state.apply_done(
            2,
            JobResultKind::Failed {
                reason: "ignored".to_string(),
            },
            Some("ignored".to_string()),
            Vec::new(),
        );
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
                extracted_links: Vec::new(),
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

    #[test]
    fn job_done_success_stores_normalized_links() {
        let mut state = AppState::new();
        state.jobs.insert(
            9,
            JobState {
                url: "https://link.example".to_string(),
                stage: Stage::Downloading,
                ..Default::default()
            },
        );

        let links = vec![
            ExtractedLink {
                url: "HTTP://EXAMPLE.com".to_string(),
                text: None,
                kind: LinkKind::Hyperlink,
            },
            ExtractedLink {
                url: "http://example.com/".to_string(),
                text: None,
                kind: LinkKind::Hyperlink,
            },
            ExtractedLink {
                url: "https://other.example:443/path".to_string(),
                text: None,
                kind: LinkKind::Hyperlink,
            },
        ];
        let (state, _) = update(
            state,
            Msg::JobDone {
                job_id: 9,
                result: JobResultKind::Success,
                content_preview: None,
                extracted_links: links,
            },
        );

        let job = state.jobs.get(&9).expect("job exists");
        let stored_links = job.links();
        assert_eq!(stored_links.len(), 2);
        assert_eq!(stored_links[0].url, "http://example.com/".to_string());
        assert_eq!(stored_links[0].index, 0);
        assert_eq!(
            stored_links[1].url,
            "https://other.example/path".to_string()
        );
        assert_eq!(stored_links[1].index, 2);
    }
}
