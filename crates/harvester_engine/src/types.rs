use crate::links::ExtractedLink;
use std::fmt;

pub type JobId = u64;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Stage {
    Queued,
    Downloading,
    Sanitizing,
    Converting,
    Tokenizing,
    Writing,
    Done,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct JobProgress {
    pub job_id: JobId,
    pub stage: Stage,
    pub bytes: Option<u64>,
    pub tokens: Option<u32>,
    pub content_preview: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EngineEvent {
    Progress(JobProgress),
    JobCompleted {
        job_id: JobId,
        result: Result<JobOutcome, FailureKind>,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FetchOutput {
    pub bytes: Vec<u8>,
    pub metadata: FetchMetadata,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct JobOutcome {
    pub final_url: String,
    pub tokens: Option<u32>,
    pub bytes_written: Option<u64>,
    pub content_preview: Option<String>,
    pub extracted_links: Vec<ExtractedLink>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FetchMetadata {
    pub original_url: String,
    pub final_url: String,
    pub redirect_count: usize,
    pub content_type: Option<String>,
    pub byte_len: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FetchError {
    pub kind: FailureKind,
    pub message: String,
}

impl FetchError {
    pub(crate) fn new(kind: FailureKind, message: impl Into<String>) -> Self {
        Self {
            kind,
            message: message.into(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FailureKind {
    InvalidUrl,
    HttpStatus(u16),
    Timeout,
    RedirectLimitExceeded,
    TooLarge { max_bytes: u64, actual: Option<u64> },
    UnsupportedContentType { content_type: String },
    ProcessingTimeout { stage: Stage },
    Cancelled,
    ProcessingError,
    Network,
}

impl fmt::Display for FailureKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            FailureKind::InvalidUrl => write!(f, "invalid url"),
            FailureKind::HttpStatus(code) => write!(f, "http status {code}"),
            FailureKind::Timeout => write!(f, "timeout"),
            FailureKind::RedirectLimitExceeded => write!(f, "redirect limit exceeded"),
            FailureKind::TooLarge { max_bytes, actual } => {
                write!(f, "response too large (max {max_bytes}, actual {actual:?})")
            }
            FailureKind::UnsupportedContentType { content_type } => {
                write!(f, "unsupported content type {content_type}")
            }
            FailureKind::ProcessingTimeout { stage } => {
                write!(f, "processing timeout at stage {stage:?}")
            }
            FailureKind::Cancelled => write!(f, "cancelled"),
            FailureKind::ProcessingError => write!(f, "processing error"),
            FailureKind::Network => write!(f, "network error"),
        }
    }
}
