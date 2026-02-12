use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use harvester_engine::{
    EngineEvent, FailureKind, FetchSettings, Fetcher, JobProgress, ProgressSink, ReqwestFetcher,
    RetrySettings, Stage, UrlPolicy,
};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;
use tokio_util::sync::CancellationToken;
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, Request, ResponseTemplate};

#[derive(Default)]
struct TestSink {
    events: Arc<Mutex<Vec<EngineEvent>>>,
}

impl TestSink {
    fn new() -> Self {
        Self {
            events: Arc::new(Mutex::new(Vec::new())),
        }
    }

    fn take(&self) -> Vec<EngineEvent> {
        self.events.lock().unwrap().drain(..).collect()
    }
}

impl ProgressSink for TestSink {
    fn emit(&self, event: EngineEvent) {
        self.events.lock().unwrap().push(event);
    }
}

fn allow_local_policy() -> UrlPolicy {
    UrlPolicy {
        block_private_ips: false,
        ..Default::default()
    }
}

fn short_retry_settings(max_retries: usize, max_backoff: Duration) -> RetrySettings {
    RetrySettings {
        max_retries,
        initial_backoff: Duration::from_millis(10),
        max_backoff,
        backoff_multiplier: 1.5,
    }
}

fn settings_with_retry(retry_settings: RetrySettings) -> FetchSettings {
    FetchSettings {
        retry_settings,
        ..Default::default()
    }
}

#[tokio::test]
async fn fetcher_returns_html_and_emits_progress() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/doc"))
        .respond_with(
            ResponseTemplate::new(200).set_body_raw("<html>ok</html>", "text/html; charset=utf-8"),
        )
        .mount(&server)
        .await;

    let fetcher = ReqwestFetcher::new(FetchSettings::default(), allow_local_policy());
    let sink = TestSink::new();
    let url = format!("{}/doc", server.uri());

    let output = fetcher
        .fetch(1, &url, &sink, &CancellationToken::new())
        .await
        .expect("fetch ok");
    assert_eq!(output.metadata.original_url, url);
    assert_eq!(output.metadata.final_url, output.metadata.original_url);
    assert_eq!(output.metadata.redirect_count, 0);
    assert!(output
        .metadata
        .content_type
        .unwrap()
        .starts_with("text/html"));
    assert_eq!(output.bytes, b"<html>ok</html>");

    let progress = sink
        .take()
        .into_iter()
        .filter_map(|event| match event {
            EngineEvent::Progress(JobProgress { stage, .. }) => Some(stage),
            _ => None,
        })
        .collect::<Vec<_>>();
    assert!(progress.contains(&Stage::Downloading));
}

#[tokio::test]
async fn fetcher_fails_on_http_status() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/missing"))
        .respond_with(ResponseTemplate::new(404))
        .mount(&server)
        .await;

    let fetcher = ReqwestFetcher::new(FetchSettings::default(), allow_local_policy());
    let sink = TestSink::new();
    let url = format!("{}/missing", server.uri());

    let err = fetcher
        .fetch(7, &url, &sink, &CancellationToken::new())
        .await
        .unwrap_err();
    assert_eq!(err.kind, FailureKind::HttpStatus(404));
}

#[tokio::test]
async fn fetcher_times_out_on_slow_response() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/slow"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_delay(Duration::from_millis(250))
                .set_body_string("slow"),
        )
        .mount(&server)
        .await;

    let settings = FetchSettings {
        request_timeout: Duration::from_millis(50),
        ..FetchSettings::default()
    };
    let fetcher = ReqwestFetcher::new(settings, allow_local_policy());
    let sink = TestSink::new();
    let url = format!("{}/slow", server.uri());

    let err = fetcher
        .fetch(2, &url, &sink, &CancellationToken::new())
        .await
        .unwrap_err();
    assert_eq!(err.kind, FailureKind::Timeout);
}

#[tokio::test]
async fn fetcher_rejects_too_large_response() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/large"))
        .respond_with(
            ResponseTemplate::new(200)
                .insert_header("Content-Type", "text/html")
                .insert_header("Content-Length", "11")
                .set_body_string("01234567890"),
        )
        .mount(&server)
        .await;

    let settings = FetchSettings {
        max_bytes: 10,
        ..FetchSettings::default()
    };
    let fetcher = ReqwestFetcher::new(settings, allow_local_policy());
    let sink = TestSink::new();
    let url = format!("{}/large", server.uri());

    let err = fetcher
        .fetch(3, &url, &sink, &CancellationToken::new())
        .await
        .unwrap_err();
    assert_eq!(
        err.kind,
        FailureKind::TooLarge {
            max_bytes: 10,
            actual: Some(11)
        }
    );
}

#[tokio::test]
async fn fetcher_rejects_private_ip_addresses() {
    let fetcher = ReqwestFetcher::new(FetchSettings::default(), UrlPolicy::default());
    let sink = TestSink::new();
    let err = fetcher
        .fetch(9, "http://127.0.0.1/", &sink, &CancellationToken::new())
        .await
        .unwrap_err();
    assert!(matches!(
        err.kind,
        FailureKind::UrlPolicyViolation { description: _ }
    ));
}

#[tokio::test]
async fn retries_on_503_then_succeeds() {
    let server = Arc::new(MockServer::start().await);
    let route = "/unstable";
    let attempts = Arc::new(AtomicUsize::new(0));
    let attempts_clone = attempts.clone();
    Mock::given(method("GET"))
        .and(path(route))
        .respond_with(move |_req: &Request| {
            let attempt = attempts_clone.fetch_add(1, Ordering::SeqCst);
            if attempt < 2 {
                ResponseTemplate::new(503)
            } else {
                ResponseTemplate::new(200).set_body_string("ok")
            }
        })
        .mount(&server)
        .await;

    let settings = settings_with_retry(short_retry_settings(2, Duration::from_secs(1)));
    let fetcher = ReqwestFetcher::new(settings, allow_local_policy());
    let sink = TestSink::new();
    let url = format!("{}/{}", server.uri(), route.trim_start_matches('/'));

    let output = fetcher
        .fetch(10, &url, &sink, &CancellationToken::new())
        .await
        .expect("should eventually succeed");
    assert_eq!(output.bytes, b"ok");
    assert_eq!(attempts.load(Ordering::SeqCst), 3);
}

#[tokio::test]
async fn does_not_retry_403() {
    let server = Arc::new(MockServer::start().await);
    Mock::given(method("GET"))
        .and(path("/forbidden"))
        .respond_with(ResponseTemplate::new(403))
        .expect(1)
        .mount(&server)
        .await;

    let fetcher = ReqwestFetcher::new(FetchSettings::default(), allow_local_policy());
    let sink = TestSink::new();
    let err = fetcher
        .fetch(
            11,
            &format!("{}/forbidden", server.uri()),
            &sink,
            &CancellationToken::new(),
        )
        .await
        .unwrap_err();
    assert_eq!(err.kind, FailureKind::HttpStatus(403));
}

#[tokio::test]
async fn does_not_retry_url_policy_violation() {
    let server = MockServer::start().await;
    let policy = UrlPolicy {
        block_private_ips: true,
        ..Default::default()
    };
    let fetcher = ReqwestFetcher::new(FetchSettings::default(), policy);
    let sink = TestSink::new();

    let err = fetcher
        .fetch(
            12,
            &format!("{}/doc", server.uri()),
            &sink,
            &CancellationToken::new(),
        )
        .await
        .unwrap_err();

    assert!(matches!(err.kind, FailureKind::UrlPolicyViolation { .. }));
    assert!(server
        .received_requests()
        .await
        .expect("server recorded requests")
        .is_empty());
}

#[tokio::test]
async fn retries_network_error_then_succeeds() {
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("failed to bind test listener");
    let addr = listener.local_addr().expect("listener has address");
    let server_handle = tokio::spawn(async move {
        let mut first = true;
        loop {
            let (mut socket, _) = listener.accept().await.expect("accept failed");
            if first {
                first = false;
                let _ = socket.shutdown().await;
                continue;
            }
            let mut buf = [0u8; 1024];
            let _ = socket.read(&mut buf).await;
            let body = "recovered";
            let response = format!(
                "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                body.len(),
                body
            );
            let _ = socket.write_all(response.as_bytes()).await;
            let _ = socket.shutdown().await;
            break;
        }
    });

    let settings = settings_with_retry(short_retry_settings(2, Duration::from_secs(1)));
    let fetcher = ReqwestFetcher::new(settings, allow_local_policy());
    let sink = TestSink::new();
    let url = format!("http://127.0.0.1:{}/drop", addr.port());

    let output = fetcher
        .fetch(13, &url, &sink, &CancellationToken::new())
        .await
        .expect("should recover after network error");
    assert_eq!(output.bytes, b"recovered");
    server_handle.await.unwrap();
}

#[tokio::test]
async fn respects_max_retries() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/always"))
        .respond_with(ResponseTemplate::new(503))
        .expect(2)
        .mount(&server)
        .await;

    let settings = settings_with_retry(short_retry_settings(1, Duration::from_millis(200)));
    let fetcher = ReqwestFetcher::new(settings, allow_local_policy());
    let sink = TestSink::new();

    let err = fetcher
        .fetch(
            14,
            &format!("{}/always", server.uri()),
            &sink,
            &CancellationToken::new(),
        )
        .await
        .unwrap_err();
    assert_eq!(err.kind, FailureKind::HttpStatus(503));
}

#[tokio::test]
async fn cancellation_stops_retry_loop() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/slow"))
        .respond_with(ResponseTemplate::new(503))
        .mount(&server)
        .await;

    let settings = settings_with_retry(short_retry_settings(2, Duration::from_millis(100)));
    let fetcher = ReqwestFetcher::new(settings, allow_local_policy());
    let sink = TestSink::new();
    let url = format!("{}/slow", server.uri());
    let cancel_token = CancellationToken::new();
    let cancel_handle = {
        let cancel_clone = cancel_token.clone();
        tokio::spawn(async move {
        tokio::time::sleep(Duration::from_millis(1)).await;
            cancel_clone.cancel();
        })
    };

    let err = fetcher
        .fetch(15, &url, &sink, &cancel_token)
        .await
        .unwrap_err();
    cancel_handle.await.unwrap();
    assert_eq!(err.kind, FailureKind::Cancelled);
}

#[tokio::test]
async fn sends_browser_headers() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/headers"))
        .respond_with(|req: &Request| {
            assert_eq!(
                req.headers
                    .get("accept")
                    .and_then(|value| value.to_str().ok())
                    .unwrap(),
                "text/html,application/xhtml+xml,application/xml;q=0.9,*/*;q=0.8"
            );
            assert_eq!(
                req.headers
                    .get("accept-language")
                    .and_then(|value| value.to_str().ok())
                    .unwrap(),
                "en-US,en;q=0.9"
            );
            ResponseTemplate::new(200)
        })
        .expect(1)
        .mount(&server)
        .await;

    let fetcher = ReqwestFetcher::new(FetchSettings::default(), allow_local_policy());
    let sink = TestSink::new();
    let _ = fetcher
        .fetch(
            16,
            &format!("{}/headers", server.uri()),
            &sink,
            &CancellationToken::new(),
        )
        .await
        .expect("should succeed");
}

#[tokio::test]
async fn uses_retry_after_header() {
    let server = MockServer::start().await;
    let timestamps = Arc::new(Mutex::new(Vec::new()));
    let attempts = Arc::new(AtomicUsize::new(0));
    let timestamps_clone = timestamps.clone();
    let attempts_clone = attempts.clone();
    Mock::given(method("GET"))
        .and(path("/rate-limited"))
        .respond_with(move |_req: &Request| {
            let attempt = attempts_clone.fetch_add(1, Ordering::SeqCst);
            timestamps_clone.lock().unwrap().push(Instant::now());
            if attempt == 0 {
                ResponseTemplate::new(429).insert_header("Retry-After", "1")
            } else {
                ResponseTemplate::new(200).set_body_string("after")
            }
        })
        .mount(&server)
        .await;

    let settings = settings_with_retry(short_retry_settings(2, Duration::from_secs(5)));
    let fetcher = ReqwestFetcher::new(settings, allow_local_policy());
    let sink = TestSink::new();

    let output = fetcher
        .fetch(
            17,
            &format!("{}/rate-limited", server.uri()),
            &sink,
            &CancellationToken::new(),
        )
        .await
        .expect("should eventually succeed");

    assert_eq!(output.bytes, b"after");
    let recorded = timestamps.lock().unwrap();
    assert_eq!(recorded.len(), 2);
    let elapsed = recorded[1].duration_since(recorded[0]);
    assert!(elapsed >= Duration::from_secs(1));
    assert_eq!(attempts.load(Ordering::SeqCst), 2);
}
