use harvester_engine::{
    EngineConfig, EngineEvent, EngineHandle, FailureKind, JobTiming, UrlPolicy,
};
use std::time::{Duration, Instant};
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

async fn completed(
    engine: &EngineHandle,
) -> (
    Result<harvester_engine::JobOutcome, FailureKind>,
    JobTiming,
    Instant,
) {
    tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            if let Some(EngineEvent::JobCompleted {
                result,
                timing,
                sent_at,
                ..
            }) = engine.try_recv()
            {
                return (result, *timing, sent_at);
            }
            tokio::time::sleep(Duration::from_millis(2)).await;
        }
    })
    .await
    .expect("completion with timing")
}

#[tokio::test]
async fn engine_reports_retry_attempts_and_fetch_wall_time_on_failure() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/fail"))
        .respond_with(ResponseTemplate::new(500).set_delay(Duration::from_millis(10)))
        .expect(3)
        .mount(&server)
        .await;
    let output = tempfile::tempdir().unwrap();
    let mut config = EngineConfig::default_with_output(output.path().into());
    config.url_policy = UrlPolicy {
        block_private_ips: false,
        ..Default::default()
    };
    config.fetch_settings.retry_settings.initial_backoff = Duration::from_millis(1);
    config.fetch_settings.retry_settings.max_backoff = Duration::from_millis(2);
    let engine = EngineHandle::new(config);
    engine.enqueue(1, format!("{}/fail", server.uri()));
    let (result, timing, sent) = completed(&engine).await;
    assert_eq!(result, Err(FailureKind::HttpStatus(500)));
    assert_eq!(timing.fetch_attempts, 3);
    assert!(timing.fetch >= Duration::from_millis(30));
    assert!(timing.total >= timing.queue_wait + timing.fetch);
    assert_eq!(timing.decode, Duration::ZERO);
    assert!(sent <= Instant::now());
}

#[tokio::test]
async fn quota_and_rejected_enqueue_completions_carry_unstarted_timings() {
    let output = tempfile::tempdir().unwrap();
    let mut config = EngineConfig::default_with_output(output.path().into());
    config.run_quotas.max_urls_per_run = Some(0);
    let engine = EngineHandle::new(config);
    engine.enqueue(1, "https://example.com/article");
    let (result, timing, _) = completed(&engine).await;
    assert!(matches!(result, Err(FailureKind::QuotaExceeded { .. })));
    assert_eq!(timing.fetch_attempts, 0);
    assert_eq!(timing.fetch, Duration::ZERO);
    assert!(timing.total >= timing.queue_wait);
    engine.stop(false);
    engine.enqueue(2, "https://example.com/article");
    let (result, timing, _) = completed(&engine).await;
    assert_eq!(result, Err(FailureKind::Cancelled));
    assert_eq!(timing.fetch_attempts, 0);
    assert_eq!(timing.article_write, Duration::ZERO);
    assert!(timing.total >= timing.queue_wait);
}

#[tokio::test]
async fn queued_cancellation_keeps_queue_wait_and_does_not_make_a_request() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/slow"))
        .respond_with(ResponseTemplate::new(404).set_delay(Duration::from_millis(60)))
        .expect(1)
        .mount(&server)
        .await;
    let output = tempfile::tempdir().unwrap();
    let mut config = EngineConfig::default_with_output(output.path().into());
    config.url_policy = UrlPolicy {
        block_private_ips: false,
        ..Default::default()
    };
    let engine = EngineHandle::new(config);
    engine.enqueue(1, format!("{}/slow", server.uri()));
    tokio::time::timeout(Duration::from_secs(2), async {
        while server
            .received_requests()
            .await
            .unwrap_or_default()
            .is_empty()
        {
            tokio::time::sleep(Duration::from_millis(2)).await;
        }
    })
    .await
    .unwrap();
    engine.enqueue(2, format!("{}/never", server.uri()));
    engine.stop(false);
    let (first, timing, _) = completed(&engine).await;
    assert_eq!(first, Err(FailureKind::HttpStatus(404)));
    assert_eq!(timing.fetch_attempts, 1);
    let (queued, timing, _) = completed(&engine).await;
    assert_eq!(queued, Err(FailureKind::Cancelled));
    assert_eq!(timing.fetch_attempts, 0);
    assert!(timing.queue_wait > Duration::ZERO);
    assert!(timing.total >= timing.queue_wait);
    assert_eq!(server.received_requests().await.unwrap().len(), 1);
}

#[tokio::test]
async fn run_quota_reset_admits_urls_after_exhaustion() {
    let output = tempfile::tempdir().unwrap();
    let mut config = EngineConfig::default_with_output(output.path().into());
    config.run_quotas.max_urls_per_run = Some(1);
    let engine = EngineHandle::new(config);
    // Private addresses fail the default URL policy without network access,
    // but still count as started URLs.
    engine.enqueue(1, "https://127.0.0.1/first");
    let (result, _, _) = completed(&engine).await;
    assert!(!matches!(result, Err(FailureKind::QuotaExceeded { .. })));
    engine.enqueue(2, "https://127.0.0.1/second");
    let (result, _, _) = completed(&engine).await;
    assert!(matches!(result, Err(FailureKind::QuotaExceeded { .. })));
    engine.reset_run_quota();
    engine.enqueue(3, "https://127.0.0.1/third");
    let (result, _, _) = completed(&engine).await;
    assert!(!matches!(result, Err(FailureKind::QuotaExceeded { .. })));
}
