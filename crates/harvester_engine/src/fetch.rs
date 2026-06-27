use std::error::Error as StdError;
use std::fmt;
use std::sync::{
    atomic::{AtomicUsize, Ordering},
    Arc,
};
use std::time::Duration;

use engine_logging::{engine_info, engine_warn};
use futures_util::StreamExt;
use reqwest::{
    header::{HeaderMap, HeaderValue, ACCEPT, ACCEPT_LANGUAGE, CONTENT_TYPE, RETRY_AFTER},
    StatusCode,
};
use tokio_util::sync::CancellationToken;

use crate::{
    EngineEvent, FailureKind, FetchError, FetchMetadata, FetchOutput, JobId, JobProgress, Stage,
    UrlPolicy,
};

const MAX_LOG_URL_LEN: usize = 96;

fn truncate_url_for_log(url: &str) -> String {
    if url.chars().count() <= MAX_LOG_URL_LEN {
        return url.to_string();
    }
    let mut short: String = url.chars().take(MAX_LOG_URL_LEN).collect();
    short.push_str("...");
    short
}

#[derive(Debug, Clone)]
pub struct RetrySettings {
    pub max_retries: usize,
    pub initial_backoff: Duration,
    pub max_backoff: Duration,
    pub backoff_multiplier: f64,
}

impl Default for RetrySettings {
    fn default() -> Self {
        Self {
            max_retries: 2,
            initial_backoff: Duration::from_secs(1),
            max_backoff: Duration::from_secs(8),
            backoff_multiplier: 2.0,
        }
    }
}

#[derive(Debug, Clone)]
pub struct FetchSettings {
    pub connect_timeout: Duration,
    pub request_timeout: Duration,
    pub redirect_limit: usize,
    pub max_bytes: u64,
    pub allowed_content_types: Vec<String>,
    pub user_agent: String,
    pub retry_settings: RetrySettings,
}

impl Default for FetchSettings {
    fn default() -> Self {
        Self {
            connect_timeout: Duration::from_secs(10),
            request_timeout: Duration::from_secs(15),
            redirect_limit: 5,
            max_bytes: 5 * 1024 * 1024,
            allowed_content_types: vec![
                "text/html".to_string(),
                "application/xhtml+xml".to_string(),
                "text/plain".to_string(),
            ],
            user_agent: "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/122.0 Safari/537.36".to_string(),
            retry_settings: RetrySettings::default(),
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
        cancel: &CancellationToken,
    ) -> Result<FetchOutput, FetchError>;
}

#[derive(Debug, Clone)]
pub struct ReqwestFetcher {
    settings: FetchSettings,
    url_policy: UrlPolicy,
}

impl ReqwestFetcher {
    pub fn new(settings: FetchSettings, url_policy: UrlPolicy) -> Self {
        Self {
            settings,
            url_policy,
        }
    }

    fn build_client(
        &self,
        redirect_counter: Arc<AtomicUsize>,
    ) -> Result<reqwest::Client, FetchError> {
        let redirect_limit = self.settings.redirect_limit;
        let url_policy = self.url_policy.clone();
        let policy = reqwest::redirect::Policy::custom({
            let counter = redirect_counter.clone();
            move |attempt| {
                let count = attempt.previous().len();
                counter.store(count, Ordering::Relaxed);
                if count >= redirect_limit {
                    attempt.error("redirect limit exceeded")
                } else {
                    let target_url = attempt.url().clone();
                    if let Err(violation) = url_policy.check(&target_url) {
                        attempt.error(UrlPolicyRedirectError::new(format!(
                            "redirect target {} violated policy: {}",
                            target_url, violation
                        )))
                    } else {
                        attempt.follow()
                    }
                }
            }
        });

        let mut default_headers = HeaderMap::new();
        default_headers.insert(
            ACCEPT,
            HeaderValue::from_static(
                "text/html,application/xhtml+xml,application/xml;q=0.9,*/*;q=0.8",
            ),
        );
        default_headers.insert(ACCEPT_LANGUAGE, HeaderValue::from_static("en-US,en;q=0.9"));

        reqwest::Client::builder()
            .connect_timeout(self.settings.connect_timeout)
            .timeout(self.settings.request_timeout)
            .redirect(policy)
            .user_agent(self.settings.user_agent.clone())
            .default_headers(default_headers)
            .build()
            .map_err(|err| FetchError::new(FailureKind::Network, err.to_string()))
    }

    fn is_content_type_allowed(&self, content_type: &str) -> bool {
        let ct = content_type
            .split(';')
            .next()
            .unwrap_or(content_type)
            .trim();
        self.settings
            .allowed_content_types
            .iter()
            .any(|allowed| allowed.eq_ignore_ascii_case(ct))
    }

    async fn fetch_once(
        &self,
        job_id: JobId,
        url: &str,
        sink: &dyn ProgressSink,
    ) -> Result<FetchOutput, FetchError> {
        let parsed = reqwest::Url::parse(url).map_err(|err| {
            engine_warn!("[fetch] Invalid URL '{}': {}", url, err);
            FetchError::new(FailureKind::InvalidUrl, err.to_string())
        })?;
        if let Err(violation) = self.url_policy.check(&parsed) {
            let reason = violation.to_string();
            engine_warn!("[fetch] URL policy violation for '{}': {}", url, reason);
            return Err(FetchError::new(
                FailureKind::UrlPolicyViolation {
                    description: reason.clone(),
                },
                reason,
            ));
        }
        let redirect_counter = Arc::new(AtomicUsize::new(0));
        let client = self.build_client(redirect_counter.clone())?;

        let response = client.get(parsed.clone()).send().await.map_err(|err| {
            let fetch_err = map_reqwest_error(err);
            engine_info!("[fetch] Fetch failed for '{}': {}", url, fetch_err.kind);
            fetch_err
        })?;

        let status = response.status();
        if !status.is_success() {
            engine_info!("[fetch] HTTP error {} for URL '{}'", status.as_u16(), url);
            let retry_after = parse_retry_after(status, response.headers());
            return Err(FetchError::new_with_retry_after(
                FailureKind::HttpStatus(status.as_u16()),
                status.to_string(),
                retry_after,
            ));
        }

        if let Some(content_len) = response.content_length() {
            if content_len > self.settings.max_bytes {
                engine_warn!(
                    "[fetch] Response too large for '{}': {} bytes (max {})",
                    url,
                    content_len,
                    self.settings.max_bytes
                );
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
                engine_warn!(
                    "[fetch] Unsupported content type '{}' for URL '{}'",
                    ct,
                    url
                );
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
            content_preview: None,
        }));
        engine_info!(
            "[fetch] Fetch start job_id={} url_len={} url={}",
            job_id,
            url.len(),
            truncate_url_for_log(url)
        );

        let mut bytes = Vec::new();
        let mut stream = response.bytes_stream();
        while let Some(chunk) = stream.next().await {
            let chunk = chunk.map_err(|err| {
                let fetch_err = map_reqwest_error(err);
                engine_warn!("[fetch] Stream error for '{}': {}", url, fetch_err.kind);
                fetch_err
            })?;
            let next_len = bytes.len() as u64 + chunk.len() as u64;
            if next_len > self.settings.max_bytes {
                engine_warn!(
                    "[fetch] Response too large (streaming) for '{}': {} bytes (max {})",
                    url,
                    next_len,
                    self.settings.max_bytes
                );
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
                content_preview: None,
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

    fn calculate_backoff(&self, attempt: usize) -> Duration {
        let settings = &self.settings.retry_settings;
        let multiplier = settings.backoff_multiplier.powf((attempt - 1) as f64);
        let base_secs = settings.initial_backoff.as_secs_f64() * multiplier;
        let max_secs = settings.max_backoff.as_secs_f64();
        let scaled_secs = base_secs.min(max_secs);
        let jitter_factor = 1.0 + (fastrand::f64() - 0.5);
        let jittered = (scaled_secs * jitter_factor).min(max_secs);
        Duration::from_secs_f64(jittered.max(0.0)).max(Duration::from_millis(1))
    }
}

#[async_trait::async_trait]
impl Fetcher for ReqwestFetcher {
    async fn fetch(
        &self,
        job_id: JobId,
        url: &str,
        sink: &dyn ProgressSink,
        cancel: &CancellationToken,
    ) -> Result<FetchOutput, FetchError> {
        let max_attempts = self.settings.retry_settings.max_retries + 1;
        for attempt in 1..=max_attempts {
            match self.fetch_once(job_id, url, sink).await {
                Ok(output) => {
                    if attempt > 1 {
                        engine_info!(
                            "[fetch] succeeded on attempt {}/{} url={}",
                            attempt,
                            max_attempts,
                            url
                        );
                    }
                    return Ok(output);
                }
                Err(err) => {
                    if !is_retryable(&err.kind) || attempt == max_attempts {
                        engine_info!(
                            "[fetch] attempt {}/{} failed url={} error={}",
                            attempt,
                            max_attempts,
                            url,
                            err.kind
                        );
                        return Err(err);
                    }
                    let retry_after = err.retry_after;
                    let kind = err.kind;
                    let backoff = retry_after
                        .filter(|duration| *duration <= self.settings.retry_settings.max_backoff)
                        .unwrap_or_else(|| self.calculate_backoff(attempt));
                    engine_info!(
                        "[fetch] attempt {}/{} failed url={} error={} retrying_after={:?}",
                        attempt,
                        max_attempts,
                        url,
                        kind,
                        backoff
                    );
                    tokio::select! {
                        _ = tokio::time::sleep(backoff) => {},
                        _ = cancel.cancelled() => {
                            engine_warn!("[fetch] cancelled during retry backoff for url={}", url);
                            return Err(FetchError::new(FailureKind::Cancelled, "cancelled during retry backoff"));
                        }
                    }
                }
            }
        }
        unreachable!(
            "[fetch] fetch loop exited without returning for url={}",
            url
        )
    }
}

#[derive(Debug)]
struct UrlPolicyRedirectError(String);

impl UrlPolicyRedirectError {
    fn new(message: String) -> Self {
        Self(message)
    }
}

impl fmt::Display for UrlPolicyRedirectError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl StdError for UrlPolicyRedirectError {}

fn map_reqwest_error(err: reqwest::Error) -> FetchError {
    if err.is_timeout() {
        return FetchError::new(FailureKind::Timeout, err.to_string());
    }
    if let Some(source) = err
        .source()
        .and_then(|source| source.downcast_ref::<UrlPolicyRedirectError>())
    {
        let reason = source.0.clone();
        return FetchError::new(
            FailureKind::UrlPolicyViolation {
                description: reason.clone(),
            },
            reason,
        );
    }
    if err.is_redirect() {
        return FetchError::new(FailureKind::RedirectLimitExceeded, err.to_string());
    }

    FetchError::new(FailureKind::Network, err.to_string())
}

fn parse_retry_after(status: StatusCode, headers: &HeaderMap) -> Option<Duration> {
    match status {
        StatusCode::TOO_MANY_REQUESTS | StatusCode::SERVICE_UNAVAILABLE => headers
            .get(RETRY_AFTER)
            .and_then(|value| value.to_str().ok())
            .and_then(|value| value.trim().parse::<u64>().ok())
            .map(Duration::from_secs),
        _ => None,
    }
}

fn is_retryable(kind: &FailureKind) -> bool {
    matches!(
        kind,
        FailureKind::Timeout
            | FailureKind::Network
            | FailureKind::HttpStatus(408 | 429 | 500 | 502 | 503 | 504)
    )
}
