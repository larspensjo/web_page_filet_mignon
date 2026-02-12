use std::{fs, path::PathBuf};

use harvester_engine::{
    build_markdown_document, Converter, EngineEvent, Extractor, FailureKind, FetchSettings,
    Fetcher, Html2MdConverter, ProgressSink, ReadabilityLikeExtractor, ReqwestFetcher, UrlPolicy,
    WhitespaceTokenCounter,
};
use reqwest::Url;
use tokio_util::sync::CancellationToken;
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

fn fixture_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("fixtures")
        .join("poisoned")
}

fn load_fixture(name: &str) -> String {
    fs::read_to_string(fixture_dir().join(name)).expect("failed to load fixture")
}

fn render_doc_from_fixture(name: &str) -> String {
    let html = load_fixture(name);
    let extractor = ReadabilityLikeExtractor;
    let extracted = extractor.extract(&html);
    let converter = Html2MdConverter;
    let conversion = converter.to_markdown(&extracted.content_html, None);
    let (_tokens, doc) = build_markdown_document(
        "https://example.com",
        extracted.title.as_deref(),
        "utf-8",
        "2026-02-08T00:00:00Z",
        &conversion.markdown,
        &WhitespaceTokenCounter,
    );
    doc
}

fn title_line(doc: &str) -> &str {
    doc.lines()
        .find(|line| line.starts_with("title: "))
        .expect("title line missing")
}

#[test]
fn frontmatter_injection_is_defanged() {
    let doc = render_doc_from_fixture("frontmatter_injection.html");
    let title_line = title_line(&doc);
    assert_eq!(title_line, "title: \"Poisoned Title --- injected: true\"");
    let delimiter_count = doc.lines().filter(|line| line.trim() == "---").count();
    assert_eq!(delimiter_count, 2, "expected a single frontmatter block");
    assert!(
        !doc.lines()
            .any(|line| line.trim_start().starts_with("injected:")),
        "no injected fields should appear in the frontmatter"
    );
}

#[test]
fn giant_title_truncates_at_char_boundary() {
    let doc = render_doc_from_fixture("giant_title.html");
    let title_line = title_line(&doc);
    let inner = title_line
        .trim_start_matches("title: ")
        .trim()
        .strip_prefix('\"')
        .and_then(|rest| rest.strip_suffix('\"'))
        .expect("title not quoted correctly");
    assert_eq!(inner.chars().count(), 500);
}

struct NoopSink;

impl ProgressSink for NoopSink {
    fn emit(&self, _: EngineEvent) {}
}

#[tokio::test]
async fn url_policy_allows_wiremock_host_when_allowed() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/doc"))
        .respond_with(ResponseTemplate::new(200).set_body_string("<html></html>"))
        .mount(&server)
        .await;

    let host = Url::parse(&server.uri())
        .expect("valid server uri")
        .host_str()
        .expect("host present")
        .to_string();
    let policy = UrlPolicy {
        block_private_ips: false,
        allowed_hosts: Some(vec![host]),
        ..Default::default()
    };
    let fetcher = ReqwestFetcher::new(FetchSettings::default(), policy);
    let sink = NoopSink;
    let url = format!("{}/doc", server.uri());
    let output = fetcher
        .fetch(1, &url, &sink, &CancellationToken::new())
        .await
        .expect("fetch should succeed");
    assert_eq!(output.metadata.original_url, url);
}

#[tokio::test]
async fn url_policy_redirect_to_private_is_blocked() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/redirect"))
        .respond_with(
            ResponseTemplate::new(302).insert_header("Location", "http://10.255.255.1/secret"),
        )
        .mount(&server)
        .await;

    let host = Url::parse(&server.uri())
        .expect("valid server uri")
        .host_str()
        .expect("host present")
        .to_string();
    let policy = UrlPolicy {
        block_private_ips: false,
        allowed_hosts: Some(vec![host]),
        blocked_hosts: vec!["10.255.255.1".to_string()],
        ..Default::default()
    };
    let fetcher = ReqwestFetcher::new(FetchSettings::default(), policy);
    let sink = NoopSink;
    let url = format!("{}/redirect", server.uri());
    let err = fetcher
        .fetch(2, &url, &sink, &CancellationToken::new())
        .await
        .unwrap_err();
    assert!(matches!(err.kind, FailureKind::UrlPolicyViolation { .. }));
}
