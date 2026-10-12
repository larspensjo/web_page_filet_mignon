use super::*;
use crate::{FetchError, FetchMetadata, FetchOutput};

struct StubFetcher {
    result: Result<FetchOutput, FetchError>,
    cancel_after_fetch: bool,
}
#[async_trait::async_trait]
impl Fetcher for StubFetcher {
    async fn fetch(
        &self,
        _id: JobId,
        _url: &str,
        sink: &dyn ProgressSink,
        cancel: &CancellationToken,
    ) -> Result<FetchOutput, FetchError> {
        sink.fetch_attempt();
        if self.cancel_after_fetch {
            cancel.cancel();
        }
        self.result.clone()
    }
}
fn article(bytes: Vec<u8>) -> FetchOutput {
    FetchOutput {
        metadata: FetchMetadata {
            original_url: "https://example.com/article".into(),
            final_url: "https://example.com/article".into(),
            redirect_count: 0,
            content_type: Some("text/html; charset=utf-8".into()),
            byte_len: bytes.len() as u64,
        },
        bytes,
    }
}
fn successful_fetch() -> StubFetcher {
    StubFetcher { result: Ok(article(format!("<html><head><title>Timing</title></head><body><article><h1>Timing</h1><p>{}</p></article></body></html>", "A useful detailed report. ".repeat(100)).into_bytes())), cancel_after_fetch: false }
}
async fn completion(
    config: EngineConfig,
    fetcher: &StubFetcher,
    token: CancellationToken,
) -> (
    Result<(u64, u32), FailureKind>,
    Result<JobOutcome, FailureKind>,
    JobTiming,
) {
    let (tx, rx) = mpsc::channel();
    let queued_at = Instant::now() - Duration::from_millis(10);
    let returned = run_job(
        42,
        "https://example.com/article".into(),
        fetcher,
        tx,
        Arc::new(config),
        token,
        queued_at,
    )
    .await;
    let events: Vec<_> = rx.try_iter().collect();
    let completed: Vec<_> = events
        .iter()
        .filter_map(|event| match event {
            EngineEvent::JobCompleted {
                job_id,
                result,
                timing,
                sent_at,
            } => Some((*job_id, result.clone(), timing.as_ref().clone(), *sent_at)),
            _ => None,
        })
        .collect();
    assert_eq!(
        completed.len(),
        1,
        "every exit emits exactly one completion"
    );
    let (id, result, timing, sent) = completed.into_iter().next().unwrap();
    assert_eq!(id, 42);
    assert_eq!(timing.total, sent.saturating_duration_since(queued_at));
    assert!(timing.queue_wait >= Duration::from_millis(10));
    assert!(
        timing.total
            >= timing.queue_wait
                + timing.fetch
                + timing.decode
                + timing.extract
                + timing.tokenize
                + timing.document_build
                + timing.article_write
                + timing.marker_write
    );
    assert_eq!(timing.fetch_attempts, 1);
    (returned, result, timing)
}

#[tokio::test]
async fn successful_completion_has_all_step_timings_and_unchanged_document_bytes() {
    let output = tempfile::tempdir().unwrap();
    let mut config = EngineConfig::default_with_output(output.path().into());
    config.fetched_utc = Arc::new(|| "2026-10-12T00:00:00Z".into());
    let fetcher = successful_fetch();
    let (returned, result, timing) =
        completion(config.clone(), &fetcher, CancellationToken::new()).await;
    let outcome = result.unwrap();
    assert_eq!(
        returned.unwrap(),
        (outcome.bytes_written.unwrap(), outcome.tokens.unwrap())
    );
    assert_eq!(
        timing.bytes,
        fetcher.result.as_ref().unwrap().bytes.len() as u64
    );
    for duration in [
        timing.extract,
        timing.tokenize,
        timing.document_build,
        timing.article_write,
        timing.marker_write,
    ] {
        assert!(duration > Duration::ZERO);
    }
    let raw = fetcher.result.unwrap();
    let decoded = decode_html(&raw.bytes, raw.metadata.content_type.as_deref()).unwrap();
    let extracted = config
        .extraction_pipeline
        .extract(&decoded.html, Some(&raw.metadata.final_url));
    let (_, expected) = build_markdown_document(
        &raw.metadata.final_url,
        extracted.title.as_deref(),
        &decoded.encoding_label,
        "2026-10-12T00:00:00Z",
        &extracted.markdown,
        config.token_counter.as_ref(),
    );
    let path = output.path().join(deterministic_filename(
        extracted.title.as_deref(),
        "https://example.com/article",
    ));
    assert_eq!(std::fs::read(path).unwrap(), expected.as_bytes());
    assert!(output.path().join("harvester-corpus.json").is_file());
}

#[tokio::test]
async fn fetch_failures_carry_timing_without_running_later_steps() {
    let output = tempfile::tempdir().unwrap();
    for kind in [
        FailureKind::InvalidUrl,
        FailureKind::Network,
        FailureKind::Timeout,
        FailureKind::HttpStatus(404),
        FailureKind::RedirectLimitExceeded,
        FailureKind::TooLarge {
            max_bytes: 1,
            actual: Some(2),
        },
        FailureKind::UnsupportedContentType {
            content_type: "image/png".into(),
        },
        FailureKind::UrlPolicyViolation {
            description: "blocked".into(),
        },
        FailureKind::Cancelled,
    ] {
        let fetcher = StubFetcher {
            result: Err(FetchError {
                kind: kind.clone(),
                message: kind.to_string(),
                retry_after: None,
            }),
            cancel_after_fetch: false,
        };
        let (returned, result, timing) = completion(
            EngineConfig::default_with_output(output.path().into()),
            &fetcher,
            CancellationToken::new(),
        )
        .await;
        assert_eq!(returned, Err(kind.clone()));
        assert_eq!(result, Err(kind));
        assert_eq!(timing.fetch_attempts, 1);
        assert_eq!(timing.decode, Duration::ZERO);
        assert_eq!(timing.article_write, Duration::ZERO);
    }
}

#[tokio::test]
async fn decode_failure_carries_fetch_and_decode_timing() {
    let output = tempfile::tempdir().unwrap();
    let fetcher = StubFetcher {
        result: Ok(article(vec![0xff])),
        cancel_after_fetch: false,
    };
    let (returned, result, timing) = completion(
        EngineConfig::default_with_output(output.path().into()),
        &fetcher,
        CancellationToken::new(),
    )
    .await;
    assert_eq!(returned, Err(FailureKind::ProcessingError));
    assert_eq!(result, Err(FailureKind::ProcessingError));
    assert_eq!(timing.fetch_attempts, 1);
    assert_eq!(timing.extract, Duration::ZERO);
    assert_eq!(timing.bytes, 1);
}

#[tokio::test]
async fn cancellation_after_fetch_has_timing_and_preserves_outcome() {
    let output = tempfile::tempdir().unwrap();
    let mut fetcher = successful_fetch();
    fetcher.cancel_after_fetch = true;
    let (_, result, timing) = completion(
        EngineConfig::default_with_output(output.path().into()),
        &fetcher,
        CancellationToken::new(),
    )
    .await;
    assert_eq!(result, Err(FailureKind::Cancelled));
    assert_eq!(timing.fetch_attempts, 1);
    assert_eq!(timing.extract, Duration::ZERO);
}

struct CancellingCounter(CancellationToken);
impl TokenCounter for CancellingCounter {
    fn count(&self, text: &str) -> u32 {
        self.0.cancel();
        crate::WhitespaceTokenCounter.count(text)
    }
}

#[tokio::test]
async fn cancellation_after_tokenizing_has_timing_and_does_not_write() {
    let output = tempfile::tempdir().unwrap();
    let token = CancellationToken::new();
    let mut config = EngineConfig::default_with_output(output.path().into());
    config.token_counter = Arc::new(CancellingCounter(token.clone()));
    let (_, result, timing) = completion(config, &successful_fetch(), token).await;
    assert_eq!(result, Err(FailureKind::Cancelled));
    assert!(timing.tokenize > Duration::ZERO);
    assert_eq!(timing.document_build, Duration::ZERO);
    assert_eq!(std::fs::read_dir(output.path()).unwrap().count(), 0);
}

#[tokio::test]
async fn blocked_content_completion_has_extraction_timing() {
    let output = tempfile::tempdir().unwrap();
    let fetcher = StubFetcher { result: Ok(article(b"<html><head><title>Just a moment</title></head><body><h1>Just a moment</h1><p>Verify you are human.</p></body></html>".to_vec())), cancel_after_fetch: false };
    let (_, result, timing) = completion(
        EngineConfig::default_with_output(output.path().into()),
        &fetcher,
        CancellationToken::new(),
    )
    .await;
    assert!(matches!(result, Err(FailureKind::BlockedContent { .. })));
    assert!(timing.extract > Duration::ZERO);
    assert_eq!(timing.tokenize, Duration::ZERO);
}

#[tokio::test]
async fn article_and_marker_write_failures_keep_existing_failure_outcomes() {
    let output = tempfile::tempdir().unwrap();
    let invalid_output = output.path().join("file");
    std::fs::write(&invalid_output, "not a directory").unwrap();
    let (_, result, timing) = completion(
        EngineConfig::default_with_output(invalid_output),
        &successful_fetch(),
        CancellationToken::new(),
    )
    .await;
    assert_eq!(result, Err(FailureKind::ProcessingError));
    assert!(timing.article_write > Duration::ZERO);
    assert_eq!(timing.marker_write, Duration::ZERO);
    std::fs::create_dir(output.path().join("harvester-corpus.json")).unwrap();
    let (_, result, timing) = completion(
        EngineConfig::default_with_output(output.path().into()),
        &successful_fetch(),
        CancellationToken::new(),
    )
    .await;
    assert_eq!(result, Err(FailureKind::ProcessingError));
    assert!(timing.article_write > Duration::ZERO);
    assert!(timing.marker_write > Duration::ZERO);
}

#[test]
fn completion_contract_includes_timing_for_processing_timeouts_and_unstarted_failures() {
    let (tx, rx) = mpsc::channel();
    for failure in [
        FailureKind::ProcessingTimeout {
            stage: Stage::Sanitizing,
        },
        FailureKind::ProcessingTimeout {
            stage: Stage::Converting,
        },
        FailureKind::ProcessingTimeout {
            stage: Stage::Tokenizing,
        },
        FailureKind::QuotaExceeded {
            description: "quota".into(),
        },
        FailureKind::Cancelled,
    ] {
        let queued_at = Instant::now() - Duration::from_millis(10);
        let timing = JobTiming {
            queue_wait: queued_at.elapsed(),
            ..Default::default()
        };
        complete_job(
            &tx,
            3,
            "https://example.com/",
            Err(failure.clone()),
            timing,
            queued_at,
        );
        match rx.recv().unwrap() {
            EngineEvent::JobCompleted {
                job_id,
                result,
                timing,
                sent_at,
            } => {
                assert_eq!(job_id, 3);
                assert_eq!(result, Err(failure));
                assert_eq!(timing.total, sent_at.duration_since(queued_at));
                assert!(timing.total >= timing.queue_wait);
            }
            _ => panic!("expected completion"),
        }
    }
}
