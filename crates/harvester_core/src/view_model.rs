use crate::SessionState;

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct AppViewModel {
    pub session: SessionState,
    pub queued_urls: Vec<String>,
    pub job_count: usize,
    pub dirty: bool,
}
