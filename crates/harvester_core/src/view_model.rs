use crate::{JobId, JobResultKind, SessionState, Stage};

pub const TOKEN_LIMIT: u64 = 200_000;

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct LastPasteStats {
    pub enqueued: usize,
    pub skipped: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AppViewModel {
    pub session: SessionState,
    pub queued_urls: Vec<String>,
    pub job_count: usize,
    pub jobs: Vec<JobRowView>,
    pub last_paste_stats: Option<LastPasteStats>,
    pub dirty: bool,
    pub total_tokens: u64,
    pub token_limit: u64,
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
}
