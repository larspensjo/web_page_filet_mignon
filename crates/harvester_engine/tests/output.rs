use harvester_engine::{
    archive_url_key, build_concatenated_export, build_markdown_document, build_triage_archive,
    deterministic_filename, ArchiveDocAnnotations, Converter, ExportOptions, Extractor,
    Html2MdConverter, ReadabilityLikeExtractor, TokenCounter, WhitespaceTokenCounter,
    CORPUS_MANIFEST_FILENAME, CORPUS_SCHEMA_VERSION, MAX_FALLBACK_BODY_CHARS,
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
fn concatenated_export_writes_corpus_manifest() {
    let temp = tempfile::TempDir::new().unwrap();
    let dir = temp.path();
    let md = "---\nurl: \"https://a\"\ntitle: \"A\"\ntoken_count: 2\nfetched_utc: \"2024-01-01T00:00:00Z\"\nencoding: \"UTF-8\"\n---\n\nBody A\n";
    std::fs::write(dir.join("a.md"), md).unwrap();

    build_concatenated_export(dir, ExportOptions::default()).unwrap();

    let manifest_path = dir.join(CORPUS_MANIFEST_FILENAME);
    assert!(manifest_path.exists());
    let manifest: Value =
        serde_json::from_str(&std::fs::read_to_string(manifest_path).unwrap()).unwrap();
    assert_eq!(manifest["format"].as_str(), Some("harvester-corpus"));
    assert_eq!(
        manifest["schema_version"].as_u64(),
        Some(CORPUS_SCHEMA_VERSION as u64)
    );
    assert_eq!(manifest["layout"]["articles"].as_array().unwrap().len(), 2);
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
        &std::collections::HashMap::new(),
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
    let idx_b = archive.find("url: https://b").unwrap();
    let idx_a = archive.find("url: https://a").unwrap();
    assert!(idx_b < idx_a, "archive order should follow ordered_urls");
    assert!(archive.contains("url: https://b\n"));
    assert!(archive.contains("url: https://a\n"));
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
        &std::collections::HashMap::new(),
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
    assert!(!archive.contains("https://old"));
    assert!(archive.contains("url: https://bad"));
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
        &std::collections::HashMap::new(),
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
    assert!(archive.contains("url: https://keep"));
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
        &std::collections::HashMap::new(),
        &std::collections::HashMap::new(),
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
        &std::collections::HashMap::new(),
        &std::collections::HashMap::new(),
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
        &std::collections::HashMap::new(),
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
        &std::collections::HashMap::new(),
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

const MIXED_URLS: [&str; 5] = [
    "https://example.com/triaged",
    "https://example.com/untriaged",
    "https://example.com/scored",
    "https://example.com/score-zero",
    "https://example.com/no-model",
];

fn archive_options(output_filename: &str) -> ExportOptions {
    ExportOptions {
        output_filename: output_filename.into(),
        manifest_filename: None,
        ..ExportOptions::default()
    }
}

fn write_article(
    dir: &std::path::Path,
    filename: &str,
    url: &str,
    title: &str,
    fetched_utc: &str,
    body: &str,
) {
    let document = format!(
        "---\nurl: {url:?}\ntitle: {title:?}\ntoken_count: 2\nfetched_utc: {fetched_utc:?}\nencoding: \"UTF-8\"\n---\n\n{body}\n"
    );
    std::fs::write(dir.join(filename), document).unwrap();
}

fn write_mixed_articles(dir: &std::path::Path, first_body: &str) {
    for (index, (url, name)) in MIXED_URLS
        .iter()
        .zip(["triaged", "untriaged", "scored", "score-zero", "no-model"])
        .enumerate()
    {
        let body = if index == 0 {
            first_body.to_string()
        } else {
            format!("Body {name}.")
        };
        write_article(
            dir,
            &format!("{index}-{name}.md"),
            url,
            &name.replace('-', " "),
            &format!("2026-09-0{}T00:00:00Z", index + 1),
            &body,
        );
    }
}

fn mixed_annotations() -> std::collections::HashMap<String, ArchiveDocAnnotations> {
    [
        (
            MIXED_URLS[0],
            ArchiveDocAnnotations {
                priority: Some(4),
                tags: Some(vec![]),
                triage_model: Some("triage-v1".into()),
                ..Default::default()
            },
        ),
        (
            MIXED_URLS[2],
            ArchiveDocAnnotations {
                signal_key: Some("scored-event".into()),
                signal_score: Some(77),
                themes: Some(vec!["chips".into(), "AI".into()]),
                ..Default::default()
            },
        ),
        (
            MIXED_URLS[3],
            ArchiveDocAnnotations {
                signal_key: Some("zero-event".into()),
                signal_score: Some(0),
                themes: Some(vec![]),
                ..Default::default()
            },
        ),
        (
            MIXED_URLS[4],
            ArchiveDocAnnotations {
                priority: Some(2),
                tags: Some(vec!["zeta".into(), "alpha".into()]),
                ..Default::default()
            },
        ),
    ]
    .into_iter()
    .map(|(url, annotation)| (archive_url_key(url), annotation))
    .collect()
}

fn export_mixed_fixture(
    fixture: &[u8],
    use_summaries: bool,
    summaries: &std::collections::HashMap<String, String>,
    first_body: &str,
) {
    let temp = tempfile::TempDir::new().unwrap();
    write_mixed_articles(temp.path(), first_body);
    build_triage_archive(
        temp.path(),
        "archive.md",
        &MIXED_URLS.map(str::to_string),
        None,
        archive_options("archive.md"),
        use_summaries,
        summaries,
        &mixed_annotations(),
        &std::collections::HashMap::new(),
    )
    .unwrap();
    let actual = std::fs::read(temp.path().join("archive.md")).unwrap();
    assert_eq!(actual, fixture);
    assert!(!actual.contains(&b'\r'));
}

#[test]
fn triage_archive_schema2_since_matches_shared_golden_fixture() {
    let temp = tempfile::TempDir::new().unwrap();
    write_mixed_articles(temp.path(), "Body triaged.");
    for (priority, count) in [(5, 1), (4, 2), (3, 3)] {
        for index in 0..count {
            let name = format!("golden-priority-{priority}-{index}");
            write_article(
                temp.path(),
                &format!("{name}.md"),
                &format!("https://example.com/{name}"),
                &name,
                "2026-09-06T00:00:00Z",
                &format!("Body {name}."),
            );
        }
    }
    for index in 0..4 {
        let name = format!("golden-unavailable-{index}");
        write_article(
            temp.path(),
            &format!("{name}.md"),
            &format!("https://example.com/{name}"),
            &name,
            "2026-09-06T00:00:00Z",
            &format!("Body {name}."),
        );
    }
    let since = chrono::DateTime::parse_from_rfc3339("2026-09-02T00:00:00Z")
        .unwrap()
        .with_timezone(&chrono::Utc);
    let mut priority_snapshot: std::collections::HashMap<_, _> = [
        ("https://example.com/score-zero", 1),
        ("https://example.com/no-model", 2),
        ("https://example.com/triaged", 4),
    ]
    .into_iter()
    .map(|(url, priority)| (archive_url_key(url), priority))
    .collect();
    for (priority, count) in [(5, 1), (4, 2), (3, 3)] {
        for index in 0..count {
            priority_snapshot.insert(
                archive_url_key(&format!(
                    "https://example.com/golden-priority-{priority}-{index}"
                )),
                priority,
            );
        }
    }
    build_triage_archive(
        temp.path(),
        "archive.md",
        &[
            "https://example.com/untriaged".into(),
            "https://example.com/scored".into(),
        ],
        Some(since),
        archive_options("archive.md"),
        false,
        &Default::default(),
        &mixed_annotations(),
        &priority_snapshot,
    )
    .unwrap();
    assert_eq!(
        std::fs::read(temp.path().join("archive.md")).unwrap(),
        include_bytes!("fixtures/archive_export/schema2_since_raw.md")
    );
}

#[test]
fn triage_archive_since_coverage_counts_window_remainders_and_zero_export() {
    let temp = tempfile::TempDir::new().unwrap();
    let dir = temp.path();
    for index in 0..2 {
        let name = format!("selected-{index}");
        write_article(
            dir,
            &format!("{name}.md"),
            &format!("https://coverage.example/{name}"),
            &name,
            "2026-09-02T00:00:00Z",
            &name,
        );
    }

    let linked = dir.join("linked");
    std::fs::create_dir_all(&linked).unwrap();
    let mut priority_snapshot = std::collections::HashMap::new();
    for index in 0..2 {
        priority_snapshot.insert(
            archive_url_key(&format!("https://coverage.example/selected-{index}")),
            5,
        );
    }
    for (priority, count) in [(5, 1), (4, 2), (3, 3), (2, 4), (1, 5)] {
        for index in 0..count {
            let name = match (priority, index) {
                (3, 0) => "manually-excluded".to_string(),
                (1, 0) => "below-threshold".to_string(),
                (4, 0) => "linked".to_string(),
                _ => format!("priority-{priority}-{index}"),
            };
            let article_dir = if name == "linked" { &linked } else { dir };
            let url = format!("https://coverage.example/{name}");
            write_article(
                article_dir,
                &format!("{name}.md"),
                &url,
                &name,
                "2026-09-03T00:00:00Z",
                &name,
            );
            priority_snapshot.insert(archive_url_key(&url), priority);
        }
    }
    for index in 0..6 {
        let name = if index == 0 {
            "no-triage".to_string()
        } else if index == 1 {
            "malformed-date".to_string()
        } else if index == 5 {
            "invalid-priority".to_string()
        } else {
            format!("unavailable-{index}")
        };
        let fetched_utc = if index == 1 {
            "not-a-date"
        } else {
            "2026-09-04T00:00:00Z"
        };
        write_article(
            dir,
            &format!("{name}.md"),
            &format!("https://coverage.example/{name}"),
            &name,
            fetched_utc,
            &name,
        );
        if index == 5 {
            priority_snapshot.insert(
                archive_url_key("https://coverage.example/invalid-priority"),
                6,
            );
        }
    }

    write_article(
        &linked,
        "duplicate-linked.md",
        "https://coverage.example/selected-1#linked",
        "Duplicate linked",
        "2026-09-05T00:00:00Z",
        "duplicate linked",
    );
    let since = chrono::DateTime::parse_from_rfc3339("2026-09-02T00:00:00Z")
        .unwrap()
        .with_timezone(&chrono::Utc);
    let coverage_summary = build_triage_archive(
        dir,
        "coverage.md",
        &[
            "https://coverage.example/selected-0".into(),
            "https://coverage.example/selected-1".into(),
        ],
        Some(since),
        archive_options("coverage.md"),
        false,
        &Default::default(),
        &Default::default(),
        &priority_snapshot,
    )
    .unwrap();
    let archive = std::fs::read_to_string(dir.join("coverage.md")).unwrap();
    assert_eq!(coverage_summary.window_count, Some(23));
    assert_eq!(
        coverage_summary.unexported_by_priority,
        Some([1, 2, 3, 4, 5, 6])
    );
    assert!(archive.contains("window_count: 23\n"));
    assert!(archive.contains(
        "unexported_by_priority: {\"5\":1,\"4\":2,\"3\":3,\"2\":4,\"1\":5,\"unavailable\":6}\n"
    ));
    assert_eq!(archive.matches("===== DOC START =====").count(), 2);

    let zero_summary = build_triage_archive(
        dir,
        "zero.md",
        &[],
        Some(since),
        archive_options("zero.md"),
        false,
        &Default::default(),
        &Default::default(),
        &priority_snapshot,
    )
    .unwrap();
    let zero = std::fs::read_to_string(dir.join("zero.md")).unwrap();
    assert_eq!(zero_summary.window_count, Some(23));
    assert_eq!(
        zero_summary.unexported_by_priority,
        Some([3, 2, 3, 4, 5, 6])
    );
    assert!(zero.contains("doc_count: 0\n"));
    assert!(zero.contains("window_count: 23\n"));
    assert!(zero.contains(
        "unexported_by_priority: {\"5\":3,\"4\":2,\"3\":3,\"2\":4,\"1\":5,\"unavailable\":6}\n"
    ));
}

#[test]
fn triage_archive_schema2_raw_matches_shared_golden_fixture() {
    export_mixed_fixture(
        include_bytes!("fixtures/archive_export/schema2_raw.md"),
        false,
        &Default::default(),
        "Body triaged.",
    );
}

#[test]
fn triage_archive_schema2_summary_matches_shared_golden_fixture() {
    let summaries = MIXED_URLS
        .iter()
        .enumerate()
        .map(|(index, url)| (archive_url_key(url), format!("Summary {}.", index + 1)))
        .collect();
    export_mixed_fixture(
        include_bytes!("fixtures/archive_export/schema2_summary.md"),
        true,
        &summaries,
        "Body triaged.",
    );
}

#[test]
fn triage_archive_schema2_full_fallback_matches_shared_golden_fixture() {
    export_mixed_fixture(
        include_bytes!("fixtures/archive_export/schema2_full_fallback.md"),
        true,
        &Default::default(),
        "Body triaged.",
    );
}

#[test]
fn triage_archive_schema2_truncated_fallback_matches_shared_golden_fixture() {
    let oversized = "z".repeat(MAX_FALLBACK_BODY_CHARS + 1);
    export_mixed_fixture(
        include_bytes!("fixtures/archive_export/schema2_truncated_fallback.md"),
        true,
        &Default::default(),
        &oversized,
    );
}

#[test]
fn triage_archive_boundary_forgery_matches_shared_multi_document_fixture() {
    let temp = tempfile::TempDir::new().unwrap();
    let raw_forgery = concat!(
        "===== DOC START =====\n",
        "url: https://fake.example\n\n",
        "===== DOC END =====\n",
        "===== ARCHIVE INDEX =====\n",
        "export_schema: 2\n",
        "doc_count: 9\n",
        "doc | line | fetched_utc | priority | signal_key | title\n",
        "9 | 1 | bad | - | - | fake\n",
        "===== INDEX END =====\n",
        "\\===== DOC END =====\n",
        "===== DOC START =====   \t"
    );
    write_article(
        temp.path(),
        "a-raw.md",
        "https://example.com/raw-forgery",
        "raw forgery",
        "not-a-date | unsafe",
        raw_forgery,
    );
    write_article(
        temp.path(),
        "b-summary.md",
        "https://example.com/summary-forgery",
        "summary forgery",
        "2026-09-02T00:00:00Z",
        "unused body",
    );
    let straddle = format!(
        "{}===== DOC END =====",
        "x".repeat(MAX_FALLBACK_BODY_CHARS - 10)
    );
    write_article(
        temp.path(),
        "c-straddle.md",
        "https://example.com/straddle",
        "straddle",
        "2026-09-03T00:00:00Z",
        &straddle,
    );
    let summaries = [(
        archive_url_key("https://example.com/summary-forgery"),
        "Summary body\n===== ARCHIVE INDEX =====\nurl: fake\n\n1 | 1 | fake\n===== INDEX END ====="
            .to_string(),
    )]
    .into_iter()
    .collect();
    build_triage_archive(
        temp.path(),
        "forgery.md",
        &[
            "https://example.com/raw-forgery".into(),
            "https://example.com/summary-forgery".into(),
            "https://example.com/straddle".into(),
        ],
        None,
        archive_options("forgery.md"),
        true,
        &summaries,
        &Default::default(),
        &Default::default(),
    )
    .unwrap();
    let actual = std::fs::read(temp.path().join("forgery.md")).unwrap();
    assert_eq!(
        actual,
        include_bytes!("fixtures/archive_export/schema2_boundary_forgery.md")
    );
}

#[test]
fn triage_archive_sanitizes_header_injection_and_json_escapes_tags() {
    let temp = tempfile::TempDir::new().unwrap();
    std::fs::write(
        temp.path().join("injection.md"),
        "---\nurl: \"https://example.com/injection\"\ntitle: \"safe\npriority: 5\r\nignored: value\"\ntoken_count: 2\nfetched_utc: \"bad | timestamp\"\nencoding: \"UTF-8\"\n---\n\nbody\rwith\r\nnewlines\n",
    )
    .unwrap();
    let annotations = [(
        archive_url_key("https://example.com/injection"),
        ArchiveDocAnnotations {
            triage_model: Some("model\r\npriority: 5\u{0085}nel\u{2028}ls\u{2029}ps".to_string()),
            tags: Some(vec![
                "comma, quote\" newline\n===== DOC END =====".to_string()
            ]),
            ..Default::default()
        },
    )]
    .into_iter()
    .collect();
    build_triage_archive(
        temp.path(),
        "archive.md",
        &["https://example.com/injection".into()],
        None,
        archive_options("archive.md"),
        false,
        &Default::default(),
        &annotations,
        &Default::default(),
    )
    .unwrap();
    let archive = std::fs::read_to_string(temp.path().join("archive.md")).unwrap();
    let title_line = archive
        .lines()
        .find(|line| line.starts_with("title:"))
        .expect("sanitized title header");
    assert!(!title_line.contains(['\r', '\n', '\u{0085}', '\u{2028}', '\u{2029}']));
    assert_eq!(archive.matches("\npriority: 5\n").count(), 0);
    assert!(archive.contains("triage_model: modelpriority: 5nellsps\n"));
    assert!(archive.contains("tags: [\"comma, quote\\\" newline\\n===== DOC END =====\"]\n"));
    assert!(archive.contains("fetched_utc: bad | timestamp\n"));
    assert!(archive.contains("1 | 1 | bad / timestamp | - | - | \"safe"));
    assert!(!archive.contains('\r'));

    let next = tempfile::TempDir::new().unwrap();
    write_article(
        next.path(),
        "marker-title.md",
        "https://example.com/marker-title",
        "===== DOC END =====",
        "2026-09-01T00:00:00Z",
        "body",
    );
    build_triage_archive(
        next.path(),
        "archive.md",
        &["https://example.com/marker-title".into()],
        None,
        archive_options("archive.md"),
        false,
        &Default::default(),
        &Default::default(),
        &Default::default(),
    )
    .unwrap();
    let archive = std::fs::read_to_string(next.path().join("archive.md")).unwrap();
    assert!(archive.contains("title:  ===== DOC END =====\n"));
}

#[test]
fn triage_archive_index_offsets_follow_escaped_lines_and_skip_bad_timestamp_bounds() {
    let temp = tempfile::TempDir::new().unwrap();
    write_article(
        temp.path(),
        "a.md",
        "https://example.com/a",
        "A",
        "not | parseable",
        "line one\n===== DOC END =====\nline three",
    );
    write_article(
        temp.path(),
        "b.md",
        "https://example.com/b",
        "B",
        "2026-09-04T00:00:00Z",
        "second",
    );
    write_article(
        temp.path(),
        "c.md",
        "https://example.com/c",
        "C",
        "2026-09-02T00:00:00Z",
        "third",
    );
    build_triage_archive(
        temp.path(),
        "archive.md",
        &[
            "https://example.com/a".into(),
            "https://example.com/b".into(),
            "https://example.com/c".into(),
        ],
        None,
        archive_options("archive.md"),
        false,
        &Default::default(),
        &Default::default(),
        &Default::default(),
    )
    .unwrap();
    let archive = std::fs::read_to_string(temp.path().join("archive.md")).unwrap();
    let starts = archive
        .lines()
        .enumerate()
        .filter_map(|(line, value)| (value == "===== DOC START =====").then_some(line + 1))
        .collect::<Vec<_>>();
    assert_eq!(starts.len(), 3);
    assert!(archive.contains(&format!(
        "2 | {} | 2026-09-04T00:00:00Z | - | - | B",
        starts[1]
    )));
    assert!(archive.contains("1 | 1 | not / parseable | - | - | A"));
    assert!(archive.contains("fetched_from: 2026-09-02T00:00:00Z"));
    assert!(archive.contains("fetched_to: 2026-09-04T00:00:00Z"));
}

#[test]
fn triage_archive_unparseable_only_timestamp_has_dash_bounds() {
    let temp = tempfile::TempDir::new().unwrap();
    write_article(
        temp.path(),
        "bad.md",
        "https://example.com/bad",
        "Bad",
        "not-a-date",
        "body",
    );
    build_triage_archive(
        temp.path(),
        "archive.md",
        &["https://example.com/bad".into()],
        None,
        archive_options("archive.md"),
        false,
        &Default::default(),
        &Default::default(),
        &Default::default(),
    )
    .unwrap();
    let archive = std::fs::read_to_string(temp.path().join("archive.md")).unwrap();
    assert!(archive.contains("fetched_from: -\nfetched_to: -\n"));
    assert!(archive.contains("1 | 1 | not-a-date | - | - | Bad"));
}

#[test]
fn triage_archive_zero_documents_matches_fixture_and_is_excluded_from_next_export() {
    let temp = tempfile::TempDir::new().unwrap();
    build_triage_archive(
        temp.path(),
        "custom-empty.md",
        &[],
        None,
        archive_options("custom-empty.md"),
        false,
        &Default::default(),
        &Default::default(),
        &Default::default(),
    )
    .unwrap();
    assert_eq!(
        std::fs::read(temp.path().join("custom-empty.md")).unwrap(),
        include_bytes!("fixtures/archive_export/schema2_index_only.md")
    );

    write_article(
        temp.path(),
        "article.md",
        "https://example.com/article",
        "Article",
        "2026-09-01T00:00:00Z",
        "body",
    );
    let next = build_triage_archive(
        temp.path(),
        "next.md",
        &["https://example.com/article".into()],
        None,
        archive_options("next.md"),
        false,
        &Default::default(),
        &Default::default(),
        &Default::default(),
    )
    .unwrap();
    assert_eq!(next.doc_count, 1);
}
