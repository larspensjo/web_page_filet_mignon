use harvester_engine::{
    build_concatenated_export, build_markdown_document, deterministic_filename, Converter,
    ExportOptions, Extractor, Html2MdConverter, ReadabilityLikeExtractor, TokenCounter,
    WhitespaceTokenCounter,
};
use pretty_assertions::assert_eq;

struct CountingTokens;
impl TokenCounter for CountingTokens {
    fn count(&self, text: &str) -> u32 {
        text.split_whitespace().count() as u32
    }
}

#[test]
fn filename_is_deterministic_and_safe() {
    let fname = deterministic_filename(Some("My: Title?/Bad"), "https://example.com/foo");
    assert!(fname.starts_with("My_ Title_Bad--"));
    assert!(fname.ends_with(".md"));

    // Stable hash
    let fname2 = deterministic_filename(Some("My: Title?/Bad"), "https://example.com/foo");
    assert_eq!(fname, fname2);

    // Reserved name patched
    let fname3 = deterministic_filename(Some("CON"), "https://example.com/foo");
    assert!(fname3.starts_with("CON_"));
}

#[test]
fn frontmatter_includes_token_count() {
    let token_counter = CountingTokens;
    let (_tokens, doc) = build_markdown_document(
        "https://example.com",
        Some("Example"),
        "UTF-8",
        "2024-01-01T00:00:00Z",
        "hello world",
        &token_counter,
    );

    assert!(doc.contains("url: https://example.com"));
    assert!(doc.contains("title: Example"));
    assert!(doc.contains("token_count: 2"));
    assert!(doc.contains("---\n\nhello world"));
}

#[test]
fn pipeline_assemble_markdown_end_to_end() {
    let html =
        r#"<html><head><title>T</title></head><body><article><p>A B</p></article></body></html>"#;
    let extracted = ReadabilityLikeExtractor.extract(html);
    let md = Html2MdConverter.to_markdown(&extracted.content_html);
    let (tokens, doc) = build_markdown_document(
        "https://example.com/x",
        extracted.title.as_deref(),
        "UTF-8",
        "2024-01-01T00:00:00Z",
        &md,
        &WhitespaceTokenCounter,
    );
    assert_eq!(tokens, 2);
    assert!(doc.contains("title: T"));
    assert!(doc.contains("A B"));
}

#[test]
fn concatenated_export_builds_delimited_output_and_manifest() {
    let temp = tempfile::TempDir::new().unwrap();
    let dir = temp.path();
    let md1 = "---\nurl: https://a\ntitle: A\ntoken_count: 2\nfetched_utc: 2024-01-01T00:00:00Z\nencoding: UTF-8\n---\n\nBody A\n";
    let md2 = "---\nurl: https://b\ntitle: B\ntoken_count: 3\nfetched_utc: 2024-01-02T00:00:00Z\nencoding: UTF-8\n---\n\nBody B\n";
    std::fs::write(dir.join("a.md"), md1).unwrap();
    std::fs::write(dir.join("b.md"), md2).unwrap();

    let summary = build_concatenated_export(dir, ExportOptions::default()).unwrap();
    let export = std::fs::read_to_string(summary.output_path).unwrap();

    assert!(export.contains("===== DOC START ====="));
    assert!(export.contains("url: https://a"));
    assert!(export.contains("url: https://b"));
    assert!(export.contains("===== DOC END ====="));
    assert_eq!(summary.doc_count, 2);
    assert_eq!(summary.total_tokens, 5);

    let manifest = std::fs::read_to_string(summary.manifest_path.unwrap()).unwrap();
    assert!(manifest.contains("\"doc_count\":2"));
    assert!(manifest.contains("\"total_tokens\":5"));
}

#[test]
fn concatenated_export_creates_missing_output_dir() {
    let temp = tempfile::TempDir::new().unwrap();
    let missing_dir = temp.path().join("missing_output");

    let summary = build_concatenated_export(&missing_dir, ExportOptions::default()).unwrap();

    assert!(summary.output_path.exists());
    let export = std::fs::read_to_string(summary.output_path).unwrap();
    assert!(export.is_empty());

    let manifest = std::fs::read_to_string(summary.manifest_path.unwrap()).unwrap();
    assert!(manifest.contains("\"doc_count\":0"));
    assert!(manifest.contains("\"total_tokens\":0"));
}
