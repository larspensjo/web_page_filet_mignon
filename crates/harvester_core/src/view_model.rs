use crate::state::LinkDownloadState;
use crate::{JobId, JobResultKind, SessionState, Stage};
use harvester_engine::LinkKind;

pub const TOKEN_LIMIT: u64 = 200_000;
pub const INPUT_PANEL_FIXED_WIDTH: i32 = 160;
pub const MIN_JOBS_PANEL_WIDTH: i32 = 200;

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct LastPasteStats {
    pub enqueued: usize,
    pub skipped: usize,
}

#[derive(Debug, Clone, PartialEq)]
pub struct PreviewHeaderView {
    pub domain: String,
    pub tokens: Option<u32>,
    pub bytes: Option<u64>,
    pub stage: Stage,
    pub outcome: Option<JobResultKind>,
    pub heading_count: usize,
    pub link_density: f64,
    pub nav_heavy: bool,
}

// Default left panel width when the input panel is shown (PANEL_INPUT + PANEL_JOBS = 160 + 440)
pub const DEFAULT_LEFT_PANEL_WIDTH: i32 = 600;
pub const DEFAULT_JOBS_PANEL_WIDTH: i32 = DEFAULT_LEFT_PANEL_WIDTH - INPUT_PANEL_FIXED_WIDTH;
// Default window width
pub const DEFAULT_WINDOW_WIDTH: i32 = 960;

#[derive(Debug, Clone, PartialEq)]
pub struct AppViewModel {
    pub session: SessionState,
    pub queued_urls: Vec<String>,
    pub job_count: usize,
    pub jobs: Vec<JobRowView>,
    pub last_paste_stats: Option<LastPasteStats>,
    pub dirty: bool,
    pub total_tokens: u64,
    pub token_limit: u64,
    pub preview_text: Option<String>,
    pub preview_header: Option<PreviewHeaderView>,
    pub briefing_can_start: bool,
    pub briefing_progress: Option<String>,
    pub briefing_preview: Option<String>,
    pub triage_can_start: bool,
    pub triage_progress: Option<String>,
    /// Width of the left panels region (PANEL_INPUT + PANEL_JOBS).
    pub left_panel_width: i32,
    /// Whether the dropbox/input panel is currently visible.
    pub input_panel_visible: bool,
    /// Current window width.
    pub window_width: i32,
}

impl Default for AppViewModel {
    fn default() -> Self {
        Self {
            session: SessionState::Idle,
            queued_urls: Vec::new(),
            job_count: 0,
            jobs: Vec::new(),
            last_paste_stats: None,
            dirty: false,
            total_tokens: 0,
            token_limit: TOKEN_LIMIT,
            preview_text: None,
            preview_header: None,
            briefing_can_start: true,
            briefing_progress: None,
            briefing_preview: None,
            triage_can_start: false,
            triage_progress: None,
            left_panel_width: DEFAULT_LEFT_PANEL_WIDTH,
            input_panel_visible: false,
            window_width: DEFAULT_WINDOW_WIDTH,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct JobRowView {
    pub job_id: JobId,
    pub url: String,
    pub stage: Stage,
    pub outcome: Option<JobResultKind>,
    pub tokens: Option<u32>,
    pub bytes: Option<u64>,
    pub link_count: usize,
    pub downloaded_link_count: usize,
    pub links: Vec<LinkRowView>,
    pub triage_annotation: Option<TriageAnnotationView>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LinkRowView {
    pub index: u32,
    pub url: String,
    pub label: String,
    pub kind: LinkKind,
    pub download_state: LinkDownloadState,
    pub age_suspect: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TriageAnnotationView {
    pub priority: u8,
    pub category: String,
    pub tags: Vec<String>,
}
