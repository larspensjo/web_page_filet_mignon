use std::collections::VecDeque;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::sync::{mpsc, Arc, Mutex};
use std::thread;
use std::time::Instant;

use engine_logging::{engine_debug, engine_info, engine_warn};
use tokio::runtime::Runtime;
use tokio::time::{timeout, Duration};
use tokio_util::sync::CancellationToken;

use crate::blocker_page::detect_blocked_page;
use crate::content_extraction::{ExtractionPipeline, ExtractionPolicy};
use crate::decode::decode_html;
use crate::fetch::{ChannelProgressSink, FetchSettings, Fetcher, ProgressSink, ReqwestFetcher};
use crate::frontmatter::build_markdown_document;
use crate::persist::{AtomicFileWriter, PersistError};
use crate::quota::{QuotaTracker, SessionQuotas};
use crate::token::TokenCounter;
use crate::url_policy::UrlPolicy;
use crate::{
    deterministic_filename, write_corpus_manifest, EngineEvent, FailureKind, JobId, JobOutcome,
    JobProgress, JobTiming, Stage,
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

#[derive(Clone)]
pub struct EngineConfig {
    pub fetch_settings: FetchSettings,
    pub url_policy: UrlPolicy,
    pub session_quotas: SessionQuotas,
    pub output_dir: PathBuf,
    pub extraction_pipeline: Arc<ExtractionPipeline>,
    pub token_counter: Arc<dyn TokenCounter>,
    /// Returns UTC timestamp string. Tests can inject fixed value.
    pub fetched_utc: Arc<dyn Fn() -> String + Send + Sync>,
    pub extract_timeout: Duration,
    pub convert_timeout: Duration,
    pub tokenize_timeout: Duration,
    pub writing_timeout: Duration,
}

impl EngineConfig {
    pub fn default_with_output(output_dir: PathBuf) -> Self {
        Self {
            fetch_settings: FetchSettings::default(),
            url_policy: UrlPolicy::default(),
            session_quotas: SessionQuotas::default(),
            output_dir,
            extraction_pipeline: Arc::new(ExtractionPipeline::new(ExtractionPolicy::default())),
            token_counter: Arc::new(crate::WhitespaceTokenCounter),
            fetched_utc: Arc::new(|| "1970-01-01T00:00:00Z".to_string()),
            extract_timeout: Duration::from_secs(30),
            convert_timeout: Duration::from_secs(15),
            tokenize_timeout: Duration::from_secs(10),
            writing_timeout: Duration::from_secs(10),
        }
    }
}

enum EngineCommand {
    Enqueue {
        job_id: JobId,
        url: String,
        queued_at: Instant,
    },
    Stop {
        immediate: bool,
    },
    Resume,
}

#[derive(Clone)]
pub struct EngineHandle {
    cmd_tx: mpsc::Sender<EngineCommand>,
    event_rx: Arc<Mutex<mpsc::Receiver<EngineEvent>>>,
}

impl EngineHandle {
    pub fn new(config: EngineConfig) -> Self {
        let (cmd_tx, cmd_rx) = mpsc::channel();
        let (event_tx, event_rx_raw) = mpsc::channel();
        let event_rx = Arc::new(Mutex::new(event_rx_raw));
        let config = Arc::new(config);

        thread::spawn(move || worker_loop(cmd_rx, event_tx, config));

        Self { cmd_tx, event_rx }
    }

    pub fn enqueue(&self, job_id: JobId, url: impl Into<String>) {
        let _ = self.cmd_tx.send(EngineCommand::Enqueue {
            job_id,
            url: url.into(),
            queued_at: Instant::now(),
        });
    }

    pub fn stop(&self, immediate: bool) {
        let _ = self.cmd_tx.send(EngineCommand::Stop { immediate });
    }

    pub fn resume(&self) {
        let _ = self.cmd_tx.send(EngineCommand::Resume);
    }

    pub fn try_recv(&self) -> Option<EngineEvent> {
        if let Ok(rx) = self.event_rx.lock() {
            rx.try_recv().ok()
        } else {
            None
        }
    }
}

fn worker_loop(
    cmd_rx: mpsc::Receiver<EngineCommand>,
    event_tx: mpsc::Sender<EngineEvent>,
    config: Arc<EngineConfig>,
) {
    let runtime = Runtime::new().expect("tokio runtime");
    let fetcher = Arc::new(ReqwestFetcher::new(
        config.fetch_settings.clone(),
        config.url_policy.clone(),
    ));
    let mut queue: VecDeque<(JobId, String, Instant)> = VecDeque::new();
    let mut accept_new = true;
    let mut cancel_token = CancellationToken::new();
    let mut quota_tracker = QuotaTracker::new(config.session_quotas.clone());

    loop {
        while let Ok(cmd) = cmd_rx.try_recv() {
            match cmd {
                EngineCommand::Enqueue {
                    job_id,
                    url,
                    queued_at,
                } => {
                    if !accept_new {
                        complete_job(
                            &event_tx,
                            job_id,
                            &url,
                            Err(FailureKind::Cancelled),
                            JobTiming {
                                queue_wait: queued_at.elapsed(),
                                ..JobTiming::default()
                            },
                            queued_at,
                        );
                        continue;
                    }
                    queue.push_back((job_id, url, queued_at));
                }
                EngineCommand::Stop { immediate } => {
                    accept_new = false;
                    if immediate {
                        cancel_token.cancel();
                    }
                    // Cancel queued (not yet started) immediately.
                    for (job_id, url, queued_at) in queue.drain(..) {
                        complete_job(
                            &event_tx,
                            job_id,
                            &url,
                            Err(FailureKind::Cancelled),
                            JobTiming {
                                queue_wait: queued_at.elapsed(),
                                ..JobTiming::default()
                            },
                            queued_at,
                        );
                    }
                }

                EngineCommand::Resume => {
                    cancel_token = CancellationToken::new();
                    accept_new = true;
                }
            }
        }

        if let Some((job_id, url, queued_at)) = queue.pop_front() {
            if let Err(failure) = quota_tracker.check_url() {
                complete_job(
                    &event_tx,
                    job_id,
                    &url,
                    Err(failure),
                    JobTiming {
                        queue_wait: queued_at.elapsed(),
                        ..JobTiming::default()
                    },
                    queued_at,
                );
                continue;
            }
            let fetcher = fetcher.clone();
            let event_tx = event_tx.clone();
            let config = config.clone();
            let child_token = cancel_token.child_token();
            let job_result = runtime.block_on(async move {
                run_job(
                    job_id,
                    url,
                    fetcher.as_ref(),
                    event_tx,
                    config,
                    child_token,
                    queued_at,
                )
                .await
            });
            match job_result {
                Ok((bytes_written, tokens)) => {
                    quota_tracker.record_job(bytes_written, tokens as u64)
                }
                Err(_) => quota_tracker.record_job(0, 0),
            };
        } else {
            // Block until next command arrives.
            match cmd_rx.recv() {
                Ok(cmd) => {
                    // push back into the queue / handle stop.
                    match cmd {
                        EngineCommand::Enqueue {
                            job_id,
                            url,
                            queued_at,
                        } => {
                            if !accept_new {
                                complete_job(
                                    &event_tx,
                                    job_id,
                                    &url,
                                    Err(FailureKind::Cancelled),
                                    JobTiming {
                                        queue_wait: queued_at.elapsed(),
                                        ..JobTiming::default()
                                    },
                                    queued_at,
                                );
                                continue;
                            }
                            queue.push_back((job_id, url, queued_at));
                        }
                        EngineCommand::Stop { immediate } => {
                            accept_new = false;
                            if immediate {
                                cancel_token.cancel();
                            }
                            for (job_id, url, queued_at) in queue.drain(..) {
                                complete_job(
                                    &event_tx,
                                    job_id,
                                    &url,
                                    Err(FailureKind::Cancelled),
                                    JobTiming {
                                        queue_wait: queued_at.elapsed(),
                                        ..JobTiming::default()
                                    },
                                    queued_at,
                                );
                            }
                        }

                        EngineCommand::Resume => {
                            cancel_token = CancellationToken::new();
                            accept_new = true;
                        }
                    }
                }
                Err(_) => break,
            }
        }
    }
}

async fn run_job(
    job_id: JobId,
    url: String,
    fetcher: &dyn Fetcher,
    event_tx: mpsc::Sender<EngineEvent>,
    config: Arc<EngineConfig>,
    cancel_token: CancellationToken,
    queued_at: Instant,
) -> Result<(u64, u32), FailureKind> {
    engine_info!("Job {} starting: {}", job_id, truncate_url_for_log(&url));
    let sink = TimingProgressSink {
        inner: ChannelProgressSink::new(event_tx.clone()),
        attempts: AtomicUsize::new(0),
        bytes: AtomicU64::new(0),
    };
    let mut timing = JobTiming {
        queue_wait: queued_at.elapsed(),
        ..JobTiming::default()
    };

    let fetch_result = measured(
        &mut timing.fetch,
        fetcher.fetch(job_id, &url, &sink, &cancel_token),
    )
    .await;
    timing.fetch_attempts = sink.attempts.load(Ordering::Relaxed);
    timing.bytes = sink.bytes.load(Ordering::Relaxed);
    let fetch_output = match fetch_result {
        Ok(out) => {
            engine_debug!(
                "Job {} fetched {} bytes from {}",
                job_id,
                out.metadata.byte_len,
                out.metadata.final_url
            );
            out
        }
        Err(err) => {
            // Error already logged in fetch.rs
            let failure = err.kind.clone();
            complete_job(
                &event_tx,
                job_id,
                &url,
                Err(failure.clone()),
                timing,
                queued_at,
            );
            return Err(failure);
        }
    };

    timing.bytes = fetch_output.metadata.byte_len;
    let decoded = match measured(
        &mut timing.decode,
        timeout(config.extract_timeout, async {
            decode_html(
                &fetch_output.bytes,
                fetch_output.metadata.content_type.as_deref(),
            )
        }),
    )
    .await
    {
        Ok(Ok(decoded)) => decoded,
        Ok(Err(_)) => {
            let failure = FailureKind::ProcessingError;
            complete_job(
                &event_tx,
                job_id,
                &url,
                Err(failure.clone()),
                timing,
                queued_at,
            );
            return Err(failure);
        }
        Err(_) => {
            let failure = FailureKind::ProcessingTimeout {
                stage: Stage::Sanitizing,
            };
            complete_job(
                &event_tx,
                job_id,
                &url,
                Err(failure.clone()),
                timing,
                queued_at,
            );
            return Err(failure);
        }
    };

    if cancel_token.is_cancelled() {
        let failure = FailureKind::Cancelled;
        complete_job(
            &event_tx,
            job_id,
            &url,
            Err(failure.clone()),
            timing,
            queued_at,
        );
        return Err(failure);
    }

    let extracted_article = match measured(
        &mut timing.extract,
        timeout(config.extract_timeout, async {
            config.extraction_pipeline.extract(
                &decoded.html,
                Some(fetch_output.metadata.final_url.as_str()),
            )
        }),
    )
    .await
    {
        Ok(article) => article,
        Err(_) => {
            let failure = FailureKind::ProcessingTimeout {
                stage: Stage::Converting,
            };
            complete_job(
                &event_tx,
                job_id,
                &url,
                Err(failure.clone()),
                timing,
                queued_at,
            );
            return Err(failure);
        }
    };

    engine_debug!(
        "[extract-pipeline] job={} candidate={:?} score={:.1} pruned={} cleanup_dropped={} bytes_in={} bytes_out={}",
        job_id,
        extracted_article.diagnostics.candidate_kind,
        extracted_article.diagnostics.candidate_score.unwrap_or(0.0),
        extracted_article.diagnostics.pruned_by_attr,
        extracted_article.diagnostics.cleanup_blocks_dropped.total(),
        extracted_article.diagnostics.original_html_bytes,
        extracted_article.diagnostics.final_markdown_bytes,
    );

    let markdown = extracted_article.markdown;

    if let Some(blocked_kind) = detect_blocked_page(
        fetch_output.metadata.final_url.as_str(),
        extracted_article.title.as_deref(),
        &markdown,
    ) {
        let failure = FailureKind::BlockedContent {
            description: blocked_kind.to_string(),
        };
        engine_info!(
            "[fetch-blocker] job_id={} original_url={} final_url={} reason={}",
            job_id,
            truncate_url_for_log(&url),
            truncate_url_for_log(fetch_output.metadata.final_url.as_str()),
            blocked_kind
        );
        complete_job(
            &event_tx,
            job_id,
            &url,
            Err(failure.clone()),
            timing,
            queued_at,
        );
        return Err(failure);
    }

    let _ = event_tx.send(EngineEvent::Progress(JobProgress {
        job_id,
        stage: Stage::Converting,
        bytes: None,
        tokens: None,
    }));

    if cancel_token.is_cancelled() {
        let failure = FailureKind::Cancelled;
        complete_job(
            &event_tx,
            job_id,
            &url,
            Err(failure.clone()),
            timing,
            queued_at,
        );
        return Err(failure);
    }

    let tokens = match measured(
        &mut timing.tokenize,
        timeout(config.tokenize_timeout, async {
            config.token_counter.count(&markdown)
        }),
    )
    .await
    {
        Ok(t) => t,
        Err(_) => {
            let failure = FailureKind::ProcessingTimeout {
                stage: Stage::Tokenizing,
            };
            complete_job(
                &event_tx,
                job_id,
                &url,
                Err(failure.clone()),
                timing,
                queued_at,
            );
            return Err(failure);
        }
    };

    let _ = event_tx.send(EngineEvent::Progress(JobProgress {
        job_id,
        stage: Stage::Tokenizing,
        bytes: None,
        tokens: Some(tokens),
    }));

    if cancel_token.is_cancelled() {
        let failure = FailureKind::Cancelled;
        complete_job(
            &event_tx,
            job_id,
            &url,
            Err(failure.clone()),
            timing,
            queued_at,
        );
        return Err(failure);
    }

    let fetched_utc_str = (config.fetched_utc)();
    let build_started = Instant::now();
    let (token_count, doc) = build_markdown_document(
        fetch_output.metadata.final_url.as_str(),
        extracted_article.title.as_deref(),
        &decoded.encoding_label,
        &fetched_utc_str,
        &markdown,
        config.token_counter.as_ref(),
    );

    timing.document_build = build_started.elapsed();
    let filename = deterministic_filename(extracted_article.title.as_deref(), &url);
    let writer = AtomicFileWriter::new(config.output_dir.clone());
    let output_dir = config.output_dir.clone();

    let doc_for_write = doc.clone();
    let write_timings = Arc::new(Mutex::new(WriteMeasurements::default()));
    let observed_write_timings = Arc::clone(&write_timings);
    let write_result = timeout(config.writing_timeout, async move {
        tokio::task::spawn_blocking(move || {
            let started = Instant::now();
            observed_write_timings.lock().unwrap().article_started = Some(started);
            let path = writer.write(&filename, &doc);
            {
                let mut timings = observed_write_timings.lock().unwrap();
                timings.article = started.elapsed();
                timings.article_started = None;
            }
            let path = path?;
            let started = Instant::now();
            observed_write_timings.lock().unwrap().marker_started = Some(started);
            let marker = write_corpus_manifest(&output_dir);
            {
                let mut timings = observed_write_timings.lock().unwrap();
                timings.marker = started.elapsed();
                timings.marker_started = None;
            }
            marker?;
            Ok::<_, PersistError>(path)
        })
        .await
    })
    .await;

    {
        let writes = write_timings.lock().unwrap();
        timing.article_write = writes
            .article_started
            .map_or(writes.article, |started| started.elapsed());
        timing.marker_write = writes
            .marker_started
            .map_or(writes.marker, |started| started.elapsed());
    }
    match write_result {
        Ok(Ok(Ok(_path))) => {
            engine_info!(
                "Job {} completed: {} tokens, {} bytes written",
                job_id,
                token_count,
                doc_for_write.len()
            );
            complete_job(
                &event_tx,
                job_id,
                &url,
                Ok(JobOutcome {
                    final_url: fetch_output.metadata.final_url,
                    tokens: Some(token_count),
                    bytes_written: Some(doc_for_write.len() as u64),
                    extracted_links: extracted_article.links,
                    fetched_utc: Some(fetched_utc_str),
                }),
                timing,
                queued_at,
            );
            Ok((doc_for_write.len() as u64, token_count))
        }
        _ => {
            let failure = FailureKind::ProcessingError;
            engine_warn!("Job {} failed: write error", job_id);
            complete_job(
                &event_tx,
                job_id,
                &url,
                Err(failure.clone()),
                timing,
                queued_at,
            );
            Err(failure)
        }
    }
}

async fn measured<T>(elapsed: &mut Duration, future: impl std::future::Future<Output = T>) -> T {
    let started = Instant::now();
    let result = future.await;
    *elapsed = started.elapsed();
    result
}

struct TimingProgressSink {
    inner: ChannelProgressSink,
    attempts: AtomicUsize,
    bytes: AtomicU64,
}

#[derive(Default)]
struct WriteMeasurements {
    article: Duration,
    marker: Duration,
    article_started: Option<Instant>,
    marker_started: Option<Instant>,
}
impl ProgressSink for TimingProgressSink {
    fn emit(&self, event: EngineEvent) {
        if let EngineEvent::Progress(progress) = &event {
            if let Some(bytes) = progress.bytes {
                self.bytes.store(bytes, Ordering::Relaxed);
            }
        }
        self.inner.emit(event);
    }
    fn fetch_attempt(&self) {
        self.attempts.fetch_add(1, Ordering::Relaxed);
    }
}

fn complete_job(
    tx: &mpsc::Sender<EngineEvent>,
    job_id: JobId,
    url: &str,
    result: Result<JobOutcome, FailureKind>,
    mut timing: JobTiming,
    queued_at: Instant,
) {
    let sent_at = Instant::now();
    timing.total = sent_at.saturating_duration_since(queued_at);
    let outcome = match &result {
        Ok(_) => "success",
        Err(FailureKind::Cancelled) => "cancelled",
        Err(_) => "failed",
    };
    engine_info!(
        "[job-timing] job_id={} url={} outcome={} queue_wait_ms={} fetch_ms={} attempts={} decode_ms={} extract_ms={} tokenize_ms={} build_ms={} write_ms={} manifest_ms={} total_ms={} bytes={} failure={}",
        job_id, truncate_url_for_log(url), outcome,
        timing.queue_wait.as_millis(), timing.fetch.as_millis(), timing.fetch_attempts,
        timing.decode.as_millis(), timing.extract.as_millis(), timing.tokenize.as_millis(),
        timing.document_build.as_millis(), timing.article_write.as_millis(), timing.marker_write.as_millis(),
        timing.total.as_millis(), timing.bytes, failure_label(result.as_ref().err()),
    );
    let _ = tx.send(EngineEvent::JobCompleted {
        job_id,
        result,
        timing: Box::new(timing),
        sent_at,
    });
}

fn failure_label(failure: Option<&FailureKind>) -> &'static str {
    match failure {
        None => "none",
        Some(FailureKind::InvalidUrl) => "invalid_url",
        Some(FailureKind::HttpStatus(_)) => "http_status",
        Some(FailureKind::Timeout) => "timeout",
        Some(FailureKind::RedirectLimitExceeded) => "redirect_limit_exceeded",
        Some(FailureKind::TooLarge { .. }) => "too_large",
        Some(FailureKind::UnsupportedContentType { .. }) => "unsupported_content_type",
        Some(FailureKind::BlockedContent { .. }) => "blocked_content",
        Some(FailureKind::ProcessingTimeout { .. }) => "processing_timeout",
        Some(FailureKind::Cancelled) => "cancelled",
        Some(FailureKind::ProcessingError) => "processing_error",
        Some(FailureKind::Network) => "network",
        Some(FailureKind::UrlPolicyViolation { .. }) => "url_policy_violation",
        Some(FailureKind::QuotaExceeded { .. }) => "quota_exceeded",
        Some(FailureKind::PathPolicyViolation { .. }) => "path_policy_violation",
        Some(FailureKind::LlmError { .. }) => "llm_error",
        Some(FailureKind::LlmValidationFailed { .. }) => "llm_validation_failed",
    }
}

#[cfg(test)]
mod timing_tests;
