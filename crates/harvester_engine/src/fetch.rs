use std::sync::{
    atomic::{AtomicUsize, Ordering},
    Arc,
};
use std::time::Duration;

use futures_util::StreamExt;
use reqwest::header::CONTENT_TYPE;

use crate::{EngineEvent, FetchError, FetchMetadata, FetchOutput, FailureKind, JobId, JobProgress, Stage};

#[derive(Debug, Clone)]
pub struct FetchSettings {
    pub connect_timeout: Duration,
    pub request_timeout: Duration,
    pub redirect_limit: usize,
    pub max_bytes: u64,
    pub allowed_content_types: Vec<String>,
}

impl Default for FetchSettings {
    fn default() -> Self {
        Self {
            connect_timeout: Duration::from_secs(10),
            request_timeout: Duration::from_secs(30),
            redirect_limit: 5,
            max_bytes: 5 * 1024 * 1024,
            allowed_content_types: vec![
                "text/html".to_string(),
                "application/xhtml+xml".to_string(),
            ],
        }
    }
}

pub trait ProgressSink: Send + Sync {
    fn emit(&self, event: EngineEvent);
}

pub struct ChannelProgressSink {
    tx: std::sync::mpsc::Sender<EngineEvent>,
}

impl ChannelProgressSink {
    pub fn new(tx: std::sync::mpsc::Sender<EngineEvent>) -> Self {
        Self { tx }
    }
}

impl ProgressSink for ChannelProgressSink {
    fn emit(&self, event: EngineEvent) {
        let _ = self.tx.send(event);
    }
}

#[async_trait::async_trait]
pub trait Fetcher: Send + Sync {
    async fn fetch(
        &self,
        job_id: JobId,
        url: &str,
        sink: &dyn ProgressSink,
    ) -> Result<FetchOutput, FetchError>;
}

#[derive(Debug, Clone)]
pub struct ReqwestFetcher {
    settings: FetchSettings,
}

impl ReqwestFetcher {
    pub fn new(settings: FetchSettings) -> Self {
        Self { settings }
    }

    fn build_client(&self, redirect_counter: Arc<AtomicUsize>) -> Result<reqwest::Client, FetchError> {
        let redirect_limit = self.settings.redirect_limit;
        let policy = reqwest::redirect::Policy::custom(move |attempt| {
            let count = attempt.previous().len();
            redirect_counter.store(count, Ordering::Relaxed);
            if count >= redirect_limit {
                attempt.error("redirect limit exceeded")
            } else {
                attempt.follow()
            }
        });

        reqwest::Client::builder()
            .connect_timeout(self.settings.connect_timeout)
            .timeout(self.settings.request_timeout)
            .redirect(policy)
            .build()
            .map_err(|err| FetchError::new(FailureKind::Network, err.to_string()))
    }

    fn is_content_type_allowed(&self, content_type: &str) -> bool {
        let ct = content_type.split(';').next().unwrap_or(content_type).trim();
        self.settings
            .allowed_content_types
            .iter()
            .any(|allowed| allowed.eq_ignore_ascii_case(ct))
    }
}

#[async_trait::async_trait]
impl Fetcher for ReqwestFetcher {
    async fn fetch(
        &self,
        job_id: JobId,
        url: &str,
        sink: &dyn ProgressSink,
    ) -> Result<FetchOutput, FetchError> {
        let parsed = reqwest::Url::parse(url)
            .map_err(|err| FetchError::new(FailureKind::InvalidUrl, err.to_string()))?;
        let redirect_counter = Arc::new(AtomicUsize::new(0));
        let client = self.build_client(redirect_counter.clone())?;

        let response = client
            .get(parsed.clone())
            .send()
            .await
            .map_err(|err| map_reqwest_error(err))?;

        let status = response.status();
        if !status.is_success() {
            return Err(FetchError::new(
                FailureKind::HttpStatus(status.as_u16()),
                status.to_string(),
            ));
        }

        if let Some(content_len) = response.content_length() {
            if content_len > self.settings.max_bytes {
                return Err(FetchError::new(
                    FailureKind::TooLarge {
                        max_bytes: self.settings.max_bytes,
                        actual: Some(content_len),
                    },
                    "response too large",
                ));
            }
        }

        let final_url = response.url().to_string();
        let content_type = response
            .headers()
            .get(CONTENT_TYPE)
            .and_then(|value| value.to_str().ok())
            .map(|value| value.to_string());

        if let Some(ct) = content_type.as_deref() {
            if !self.is_content_type_allowed(ct) {
                return Err(FetchError::new(
                    FailureKind::UnsupportedContentType {
                        content_type: ct.to_string(),
                    },
                    "unsupported content type",
                ));
            }
        }

        sink.emit(EngineEvent::Progress(JobProgress {
            job_id,
            stage: Stage::Downloading,
            bytes: Some(0),
            tokens: None,
        }));

        let mut bytes = Vec::new();
        let mut stream = response.bytes_stream();
        while let Some(chunk) = stream.next().await {
            let chunk = chunk.map_err(|err| map_reqwest_error(err))?;
            let next_len = bytes.len() as u64 + chunk.len() as u64;
            if next_len > self.settings.max_bytes {
                return Err(FetchError::new(
                    FailureKind::TooLarge {
                        max_bytes: self.settings.max_bytes,
                        actual: Some(next_len),
                    },
                    "response too large",
                ));
            }
            bytes.extend_from_slice(&chunk);
            sink.emit(EngineEvent::Progress(JobProgress {
                job_id,
                stage: Stage::Downloading,
                bytes: Some(bytes.len() as u64),
                tokens: None,
            }));
        }

        let metadata = FetchMetadata {
            original_url: url.to_string(),
            final_url,
            redirect_count: redirect_counter.load(Ordering::Relaxed),
            content_type,
            byte_len: bytes.len() as u64,
        };

        Ok(FetchOutput { bytes, metadata })
    }
}

fn map_reqwest_error(err: reqwest::Error) -> FetchError {
    if err.is_timeout() {
        return FetchError::new(FailureKind::Timeout, err.to_string());
    }
    if err.is_redirect() {
        return FetchError::new(FailureKind::RedirectLimitExceeded, err.to_string());
    }
    FetchError::new(FailureKind::Network, err.to_string())
}
