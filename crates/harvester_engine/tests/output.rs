use harvester_engine::{
    archive_url_key, build_concatenated_export, build_markdown_document, build_triage_archive,
    deterministic_filename, Converter, ExportOptions, Extractor, Html2MdConverter,
    ReadabilityLikeExtractor, TokenCounter, WhitespaceTokenCounter,
};
use pretty_assertions::assert_eq;
use serde_json::Value;

struct CountingTokens;
impl TokenCounter for CountingTokens {
    fn count(&self, text: &str) -> u32 {
        text.split_whitespace().count() as u32
    }
}

fn read_manifest(path: &std::path::Path) -> Value {
    let manifest = std::fs::read_to_string(path).unwrap();
    serde_json::from_str(&manifest).unwrap()
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

    assert!(doc.contains("url: \"https://example.com\""));
    assert!(doc.contains("title: \"Example\""));
    assert!(doc.contains("token_count: 2"));
    assert!(doc.contains("---\n\nhello world"));
}

#[test]
fn pipeline_assemble_markdown_end_to_end() {
    let html =
        r#"<html><head><title>T</title></head><body><article><p>A B</p></article></body></html>"#;
    let extracted = ReadabilityLikeExtractor.extract(html);
    let md = Html2MdConverter.to_markdown(&extracted.content_html, None);
    let (tokens, doc) = build_markdown_document(
        "https://example.com/x",
        extracted.title.as_deref(),
        "UTF-8",
        "2024-01-01T00:00:00Z",
        &md.markdown,
        &WhitespaceTokenCounter,
    );
    assert_eq!(tokens, 2);
    assert!(doc.contains("title: \"T\""));
    assert!(doc.contains("A B"));
}

#[test]
fn concatenated_export_builds_delimited_output_and_manifest() {
    let temp = tempfile::TempDir::new().unwrap();
    let dir = temp.path();
    let md1 = "---\nurl: \"https://a\"\ntitle: \"A\"\ntoken_count: 2\nfetched_utc: \"2024-01-01T00:00:00Z\"\nencoding: \"UTF-8\"\n---\n\nBody A\n";
    let md2 = "---\nurl: \"https://b\"\ntitle: \"B\"\ntoken_count: 3\nfetched_utc: \"2024-01-02T00:00:00Z\"\nencoding: \"UTF-8\"\n---\n\nBody B\n";
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

    let manifest = read_manifest(summary.manifest_path.as_ref().unwrap());
    assert_eq!(manifest["doc_count"].as_u64(), Some(2));
    assert_eq!(manifest["total_tokens"].as_u64(), Some(5));
    assert_eq!(manifest["files"].as_array().unwrap().len(), 2);
}

#[test]
fn concatenated_export_creates_missing_output_dir() {
    let temp = tempfile::TempDir::new().unwrap();
    let missing_dir = temp.path().join("missing_output");

    let summary = build_concatenated_export(&missing_dir, ExportOptions::default()).unwrap();

    assert!(summary.output_path.exists());
    let export = std::fs::read_to_string(summary.output_path).unwrap();
    assert!(export.is_empty());

    let manifest = read_manifest(summary.manifest_path.as_ref().unwrap());
    assert_eq!(manifest["doc_count"].as_u64(), Some(0));
    assert_eq!(manifest["total_tokens"].as_u64(), Some(0));
    assert_eq!(manifest["files"].as_array().unwrap().len(), 0);
}

#[test]
fn concatenated_export_includes_linked_pages_and_dedupes_urls() {
    let temp = tempfile::TempDir::new().unwrap();
    let dir = temp.path();
    let linked_dir = dir.join("linked");
    std::fs::create_dir_all(&linked_dir).unwrap();

    let root_md = "---\nurl: \"https://root\"\ntitle: \"Root\"\ntoken_count: 1\nfetched_utc: \"2024-01-01T00:00:00Z\"\nencoding: \"UTF-8\"\n---\n\nroot\n";
    let link_md = "---\nurl: \"https://link\"\ntitle: \"Link\"\ntoken_count: 2\nfetched_utc: \"2024-01-02T00:00:00Z\"\nencoding: \"UTF-8\"\n---\n\nlink\n";
    let duplicate_md = "---\nurl: \"https://link/\"\ntitle: \"Link Dup\"\ntoken_count: 3\nfetched_utc: \"2024-01-03T00:00:00Z\"\nencoding: \"UTF-8\"\n---\n\ndup\n";

    std::fs::write(dir.join("root.md"), root_md).unwrap();
    std::fs::write(linked_dir.join("link.md"), link_md).unwrap();
    std::fs::write(dir.join("duplicate.md"), duplicate_md).unwrap();

    let summary = build_concatenated_export(dir, ExportOptions::default()).unwrap();
    assert_eq!(summary.doc_count, 2);
    let export = std::fs::read_to_string(summary.output_path).unwrap();
    assert!(export.contains("url: https://root"));
    assert!(export.contains("url: https://link"));
    assert!(!export.contains("link Dup"));

    let manifest = read_manifest(summary.manifest_path.as_ref().unwrap());
    let urls = manifest["files"]
        .as_array()
        .unwrap()
        .iter()
        .map(|entry| entry["url"].as_str().unwrap())
        .collect::<Vec<_>>();
    assert_eq!(urls.len(), 2);
    assert!(urls.contains(&"https://root"));
    assert!(
        urls.iter()
            .any(|url| url.trim_end_matches('/') == "https://link"),
        "linked url missing: {urls:?}"
    );
}

#[test]
fn triage_archive_uses_ordered_urls_and_preserves_full_markdown() {
    let temp = tempfile::TempDir::new().unwrap();
    let dir = temp.path();
    let md_a = "---\nurl: \"https://a\"\ntitle: \"A\"\ntoken_count: 2\nfetched_utc: \"2026-02-01T00:00:00Z\"\nencoding: \"UTF-8\"\n---\n\n# A Heading\n\nBody A\n";
    let md_b = "---\nurl: \"https://b\"\ntitle: \"B\"\ntoken_count: 3\nfetched_utc: \"2026-02-02T00:00:00Z\"\nencoding: \"UTF-8\"\n---\n\n# B Heading\n\nBody B\n";
    std::fs::write(dir.join("a.md"), md_a).unwrap();
    std::fs::write(dir.join("b.md"), md_b).unwrap();

    let options = ExportOptions {
        output_filename: "archive.md".to_string(),
        manifest_filename: None,
        ..ExportOptions::default()
    };
    let summary = build_triage_archive(
        dir,
        "custom-archive.md",
        &["https://b".to_string(), "https://a".to_string()],
        None,
        options,
        false,
        &std::collections::HashMap::new(),
    )
    .unwrap();
    assert_eq!(
        summary
            .output_path
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap(),
        "custom-archive.md"
    );
    assert_eq!(summary.doc_count, 2);
    assert_eq!(summary.total_tokens, 5);
    assert!(summary.manifest_path.is_none());

    let archive = std::fs::read_to_string(summary.output_path).unwrap();
    let idx_b = archive.find("url: \"https://b\"").unwrap();
    let idx_a = archive.find("url: \"https://a\"").unwrap();
    assert!(idx_b < idx_a, "archive order should follow ordered_urls");
    assert!(!archive.contains("url: https://b\n"));
    assert!(!archive.contains("url: https://a\n"));
    assert!(archive.contains("url: \"https://b\""));
    assert!(archive.contains("url: \"https://a\""));
    assert!(archive.contains("# B Heading"));
    assert!(archive.contains("# A Heading"));
}

#[test]
fn triage_archive_since_filter_excludes_old_docs_but_keeps_malformed_timestamps() {
    let temp = tempfile::TempDir::new().unwrap();
    let dir = temp.path();
    let md_old = "---\nurl: \"https://old\"\ntitle: \"Old\"\ntoken_count: 1\nfetched_utc: \"2024-01-01T00:00:00Z\"\nencoding: \"UTF-8\"\n---\n\nold\n";
    let md_bad = "---\nurl: \"https://bad\"\ntitle: \"Bad\"\ntoken_count: 2\nfetched_utc: \"not-a-date\"\nencoding: \"UTF-8\"\n---\n\nbad\n";
    std::fs::write(dir.join("old.md"), md_old).unwrap();
    std::fs::write(dir.join("bad.md"), md_bad).unwrap();

    let since = chrono::DateTime::parse_from_rfc3339("2025-01-01T00:00:00Z")
        .unwrap()
        .with_timezone(&chrono::Utc);
    let options = ExportOptions {
        output_filename: "archive.md".to_string(),
        manifest_filename: None,
        ..ExportOptions::default()
    };
    let summary = build_triage_archive(
        dir,
        "archive.md",
        &["https://old".to_string(), "https://bad".to_string()],
        Some(since),
        options,
        false,
        &std::collections::HashMap::new(),
    )
    .unwrap();
    assert_eq!(
        summary
            .output_path
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap(),
        "archive.md"
    );

    assert_eq!(summary.doc_count, 1);
    assert_eq!(summary.total_tokens, 2);
    let archive = std::fs::read_to_string(summary.output_path).unwrap();
    assert!(!archive.contains("url: \"https://old\""));
    assert!(archive.contains("url: \"https://bad\""));
}

#[test]
fn triage_archive_ignores_existing_archive_md_artifact() {
    let temp = tempfile::TempDir::new().unwrap();
    let dir = temp.path();
    let md = "---\nurl: \"https://keep\"\ntitle: \"Keep\"\ntoken_count: 1\nfetched_utc: \"2026-02-15T00:00:00Z\"\nencoding: \"UTF-8\"\n---\n\nkeep\n";
    std::fs::write(dir.join("keep.md"), md).unwrap();
    std::fs::write(dir.join("archive.md"), "not-frontmatter-archive-content").unwrap();
    std::fs::write(
        dir.join("archive-all-2026-02-01.md"),
        "old archive artifact",
    )
    .unwrap();

    let options = ExportOptions {
        output_filename: "archive.md".to_string(),
        manifest_filename: None,
        ..ExportOptions::default()
    };
    let summary = build_triage_archive(
        dir,
        "archive.md",
        &["https://keep".to_string()],
        None,
        options,
        false,
        &std::collections::HashMap::new(),
    )
    .unwrap();
    assert_eq!(
        summary
            .output_path
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap(),
        "archive.md"
    );
    assert_eq!(summary.doc_count, 1);
    let archive = std::fs::read_to_string(summary.output_path).unwrap();
    assert!(archive.contains("url: \"https://keep\""));
}

#[test]
fn triage_archive_uses_summary_body_when_provided() {
    let temp = tempfile::TempDir::new().unwrap();
    let dir = temp.path();
    let md = "---\nurl: \"https://example.com/a\"\ntitle: \"Article A\"\ntoken_count: 500\nfetched_utc: \"2026-04-01T00:00:00Z\"\nencoding: \"UTF-8\"\n---\n\nFull article body text.\n";
    std::fs::write(dir.join("a.md"), md).unwrap();

    let mut summaries = std::collections::HashMap::new();
    summaries.insert(
        archive_url_key("https://example.com/a"),
        "## Summary\nCompact summary.\n\n## Key Points\n- Key point one\n".to_string(),
    );

    let options = ExportOptions {
        output_filename: "archive.md".to_string(),
        manifest_filename: None,
        ..ExportOptions::default()
    };
    let summary = build_triage_archive(
        dir,
        "archive.md",
        &["https://example.com/a".to_string()],
        None,
        options,
        true,
        &summaries,
    )
    .unwrap();

    let content = std::fs::read_to_string(&summary.output_path).unwrap();
    assert!(content.contains("content: summary"));
    assert!(content.contains("Compact summary."));
    assert!(!content.contains("Full article body text."));
}

#[test]
fn triage_archive_falls_back_to_full_body_when_no_summary() {
    let temp = tempfile::TempDir::new().unwrap();
    let dir = temp.path();
    let md = "---\nurl: \"https://example.com/b\"\ntitle: \"Article B\"\ntoken_count: 100\nfetched_utc: \"2026-04-01T00:00:00Z\"\nencoding: \"UTF-8\"\n---\n\nFull fallback body.\n";
    std::fs::write(dir.join("b.md"), md).unwrap();

    let mut summaries = std::collections::HashMap::new();
    summaries.insert(
        archive_url_key("https://other.com/x"),
        "## Summary\nOther.\n".to_string(),
    );

    let options = ExportOptions {
        output_filename: "archive.md".to_string(),
        manifest_filename: None,
        ..ExportOptions::default()
    };
    let summary = build_triage_archive(
        dir,
        "archive.md",
        &["https://example.com/b".to_string()],
        None,
        options,
        true,
        &summaries,
    )
    .unwrap();

    let content = std::fs::read_to_string(&summary.output_path).unwrap();
    assert!(content.contains("Full fallback body."));
    assert!(content.contains("content: full"));
    assert!(!content.contains("content: summary"));
}

#[test]
fn triage_archive_summary_mode_with_empty_map_uses_fallback_format() {
    let temp = tempfile::TempDir::new().unwrap();
    let dir = temp.path();
    let md = "---\nurl: \"https://example.com/e\"\ntitle: \"No Summary\"\ntoken_count: 50\nfetched_utc: \"2026-04-01T00:00:00Z\"\nencoding: \"UTF-8\"\n---\n\nBody text.\n";
    std::fs::write(dir.join("e.md"), md).unwrap();

    let options = ExportOptions {
        output_filename: "archive.md".to_string(),
        manifest_filename: None,
        ..ExportOptions::default()
    };
    let summary = build_triage_archive(
        dir,
        "archive.md",
        &["https://example.com/e".to_string()],
        None,
        options,
        true,
        &std::collections::HashMap::new(),
    )
    .unwrap();

    let content = std::fs::read_to_string(&summary.output_path).unwrap();
    assert!(content.contains("Body text."));
    assert!(content.contains("content: full"));
    assert!(
        !content.contains("token_count: 50"),
        "YAML frontmatter must not appear in summary mode"
    );
}

#[test]
fn triage_archive_truncates_large_fallback_body_safely() {
    let temp = tempfile::TempDir::new().unwrap();
    let dir = temp.path();
    let prefix = "x".repeat(50_010);
    let suffix = "😀".repeat(5);
    let body = format!("{prefix}{suffix}");
    let md = format!("---\nurl: \"https://example.com/c\"\ntitle: \"Big\"\ntoken_count: 15000\nfetched_utc: \"2026-04-01T00:00:00Z\"\nencoding: \"UTF-8\"\n---\n\n{body}\n");
    std::fs::write(dir.join("c.md"), md).unwrap();

    let options = ExportOptions {
        output_filename: "archive.md".to_string(),
        manifest_filename: None,
        ..ExportOptions::default()
    };
    let summary = build_triage_archive(
        dir,
        "archive.md",
        &["https://example.com/c".to_string()],
        None,
        options,
        true,
        &std::collections::HashMap::new(),
    )
    .unwrap();

    let content = std::fs::read_to_string(&summary.output_path).unwrap();
    assert!(content.contains("content: full-truncated"));
}

#[test]
fn concatenated_export_ignores_custom_archive_artifacts_by_content() {
    let temp = tempfile::TempDir::new().unwrap();
    let dir = temp.path();
    let md = "---\nurl: \"https://keep\"\ntitle: \"Keep\"\ntoken_count: 1\nfetched_utc: \"2026-02-15T00:00:00Z\"\nencoding: \"UTF-8\"\n---\n\nkeep\n";
    std::fs::write(dir.join("keep.md"), md).unwrap();
    std::fs::write(
        dir.join("old-custom.md"),
        "===== DOC START =====\n---\nurl: \"https://ignore\"\ntitle: \"Ignore\"\ntoken_count: 1\nfetched_utc: \"2026-02-14T00:00:00Z\"\nencoding: \"UTF-8\"\n---\n\nignore\n",
    )
    .unwrap();

    let summary = build_concatenated_export(dir, ExportOptions::default()).unwrap();
    assert_eq!(summary.doc_count, 1);
    let export = std::fs::read_to_string(summary.output_path).unwrap();
    assert!(export.contains("url: https://keep"));
    assert!(!export.contains("url: https://ignore"));
}
