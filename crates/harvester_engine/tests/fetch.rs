use std::sync::{Arc, Mutex};
use std::time::Duration;

use harvester_engine::{
    EngineEvent, FailureKind, FetchSettings, Fetcher, JobProgress, ProgressSink, ReqwestFetcher,
    Stage, UrlPolicy,
};
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

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

    let output = fetcher.fetch(1, &url, &sink).await.expect("fetch ok");
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

    let err = fetcher.fetch(7, &url, &sink).await.unwrap_err();
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

    let err = fetcher.fetch(2, &url, &sink).await.unwrap_err();
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

    let err = fetcher.fetch(3, &url, &sink).await.unwrap_err();
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
        .fetch(9, "http://127.0.0.1/", &sink)
        .await
        .unwrap_err();
    assert!(matches!(
        err.kind,
        FailureKind::UrlPolicyViolation { description: _ }
    ));
}
