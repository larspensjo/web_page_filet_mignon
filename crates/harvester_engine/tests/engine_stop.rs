use std::collections::BTreeMap;
use std::path::PathBuf;
use std::time::Duration;

use harvester_engine::{EngineConfig, EngineEvent, EngineHandle, FailureKind, UrlPolicy};
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

async fn wait_for_completions(
    engine: &EngineHandle,
    expected: &[u64],
) -> BTreeMap<u64, Result<harvester_engine::JobOutcome, FailureKind>> {
    tokio::time::timeout(Duration::from_secs(8), async {
        let mut completions = BTreeMap::new();
        loop {
            while let Some(event) = engine.try_recv() {
                if let EngineEvent::JobCompleted { job_id, result } = event {
                    completions.insert(job_id, result);
                }
            }
            if expected
                .iter()
                .all(|job_id| completions.contains_key(job_id))
            {
                return completions;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .expect("engine completion timed out")
}

#[tokio::test]
async fn finish_stop_drains_the_current_job_cancels_queued_jobs_and_accepts_the_next_run() {
    let server = MockServer::start().await;
    let article = format!(
        "<html><head><title>Stop drain</title></head><body><main><article><h1>Stop drain</h1><p>{}</p></article></main></body></html>",
        "A detailed report with enough content for the extractor. ".repeat(40)
    );
    Mock::given(method("GET"))
        .and(path("/slow"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_delay(Duration::from_millis(300))
                .set_body_raw(article.clone(), "text/html; charset=utf-8"),
        )
        .mount(&server)
        .await;
    for route in ["/queued-one", "/queued-two", "/next-run"] {
        Mock::given(method("GET"))
            .and(path(route))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_body_raw(article.clone(), "text/html; charset=utf-8"),
            )
            .mount(&server)
            .await;
    }

    let output = tempfile::tempdir().expect("output directory");
    let mut config = EngineConfig::default_with_output(PathBuf::from(output.path()));
    config.url_policy = UrlPolicy {
        block_private_ips: false,
        ..UrlPolicy::default()
    };
    let engine = EngineHandle::new(config);
    let url = |route: &str| format!("{}{route}", server.uri());

    engine.enqueue(1, url("/slow"));
    tokio::time::timeout(Duration::from_secs(3), async {
        loop {
            if server
                .received_requests()
                .await
                .is_some_and(|requests| !requests.is_empty())
            {
                break;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .expect("slow download did not start");

    engine.enqueue(2, url("/queued-one"));
    engine.enqueue(3, url("/queued-two"));
    engine.stop(false);

    let completions = wait_for_completions(&engine, &[1, 2, 3]).await;
    assert!(
        completions[&1].is_ok(),
        "the in-flight download must finish"
    );
    assert!(matches!(completions[&2], Err(FailureKind::Cancelled)));
    assert!(matches!(completions[&3], Err(FailureKind::Cancelled)));
    assert_eq!(server.received_requests().await.unwrap().len(), 1);

    engine.enqueue(4, url("/next-run"));
    let completions = wait_for_completions(&engine, &[4]).await;
    assert!(matches!(completions[&4], Err(FailureKind::Cancelled)));
    assert_eq!(server.received_requests().await.unwrap().len(), 1);

    engine.resume();
    engine.enqueue(5, url("/next-run"));
    let completions = wait_for_completions(&engine, &[5]).await;
    assert!(
        completions[&5].is_ok(),
        "the next run must get a fresh token"
    );
    assert_eq!(server.received_requests().await.unwrap().len(), 2);
}
