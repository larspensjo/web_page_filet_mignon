use crate::{JobId, JobResultKind, SessionState, Stage};

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct LastPasteStats {
    pub enqueued: usize,
    pub skipped: usize,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct AppViewModel {
    pub session: SessionState,
    pub queued_urls: Vec<String>,
    pub job_count: usize,
    pub jobs: Vec<JobRowView>,
    pub last_paste_stats: Option<LastPasteStats>,
    pub dirty: bool,
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
