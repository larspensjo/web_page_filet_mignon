use std::{collections::HashMap, fs, path::Path};

use chrono::{DateTime, Utc};
use harvester_engine::{
    build_markdown_document, build_triage_archive, scan_archive_article_metadata,
    scan_archive_window_metadata, ExportError, WhitespaceTokenCounter,
};
use tempfile::tempdir;

fn date(raw: &str) -> DateTime<Utc> {
    DateTime::parse_from_rfc3339(raw)
        .unwrap()
        .with_timezone(&Utc)
}

fn article(dir: &Path, filename: &str, url: &str, fetched: &str, title: &str) {
    let (_, text) = build_markdown_document(
        url,
        Some(title),
        "utf-8",
        fetched,
        "A shared clean body with useful facts.",
        &WhitespaceTokenCounter,
    );
    fs::write(dir.join(filename), text).unwrap();
}

#[test]
fn population_matches_exporter_including_linked_and_filtered_url_duplicates() {
    let tmp = tempdir().unwrap();
    let dir = tmp.path();
    let linked = dir.join("linked");
    fs::create_dir(&linked).unwrap();
    let since = date("2026-10-02T00:00:00Z");
    let until = date("2026-10-07T04:57:36Z");
    article(
        dir,
        "a-old.md",
        "https://example.com/renewed",
        "2026-10-01T00:00:00Z",
        "Old",
    );
    article(
        &linked,
        "b-renewed.md",
        "https://example.com/renewed#part",
        "2026-10-03T00:00:00Z",
        "Renewed",
    );
    // Filename precedence is shared across root and linked, not root-first.
    article(
        &linked,
        "c-first.md",
        "https://example.com/duplicate",
        "2026-10-04T00:00:00Z",
        "Linked winner",
    );
    article(
        dir,
        "z-last.md",
        "https://EXAMPLE.com:443/duplicate#part",
        "2026-10-04T00:00:00Z",
        "Root loser",
    );
    article(
        dir,
        "d-malformed.md",
        "https://example.com/bad-date",
        "malformed",
        "Bad date",
    );
    article(
        dir,
        "archive-hidden.md",
        "https://example.com/artifact",
        "2026-10-04T00:00:00Z",
        "Artifact",
    );
    fs::write(
        dir.join("index-only.md"),
        "===== ARCHIVE INDEX =====\ndoc_count: 0\n",
    )
    .unwrap();
    fs::write(dir.join("ignore.MD"), "not an exporter markdown extension").unwrap();
    let members = scan_archive_window_metadata(dir, since, until).unwrap();
    assert_eq!(members.len(), 3);
    assert_eq!(members[1].source_title.as_deref(), Some("Linked winner"));
    assert_eq!(members[2].fetched_utc.as_deref(), Some("malformed"));
    let export = build_triage_archive(
        dir,
        "measurement",
        &[],
        Some(since),
        false,
        &HashMap::new(),
        &HashMap::new(),
        &HashMap::new(),
    )
    .unwrap();
    assert_eq!(export.window_count, Some(members.len()));
    assert_eq!(
        scan_archive_window_metadata(dir, since, until).unwrap(),
        members
    );
}

#[test]
fn bounds_are_inclusive_and_applied_before_url_deduplication() {
    let tmp = tempdir().unwrap();
    let dir = tmp.path();
    let since = date("2026-10-02T00:00:00Z");
    let until = date("2026-10-07T04:57:36Z");
    article(
        dir,
        "a-too-new.md",
        "https://example.com/reused",
        "2026-10-07T04:57:37Z",
        "Future",
    );
    article(
        dir,
        "b-start.md",
        "https://example.com/start",
        "2026-10-02T00:00:00Z",
        "Start",
    );
    article(
        dir,
        "c-end.md",
        "https://example.com/reused",
        "2026-10-07T04:57:36Z",
        "End",
    );
    let members = scan_archive_window_metadata(dir, since, until).unwrap();
    assert_eq!(members.len(), 2);
    assert_eq!(members[1].source_title.as_deref(), Some("End"));
    assert!(scan_archive_window_metadata(dir, until, since).is_err());
}

#[test]
fn metadata_reuses_clean_text_hash_and_includes_undated_articles_exporter_rejects() {
    let tmp = tempdir().unwrap();
    let dir = tmp.path();
    article(
        dir,
        "article.md",
        "https://example.com/a",
        "2026-10-03T00:00:00Z",
        "A",
    );
    let path = dir.join("article.md");
    let text = fs::read_to_string(&path)
        .unwrap()
        .lines()
        .filter(|line| !line.starts_with("fetched_utc:"))
        .collect::<Vec<_>>()
        .join("\n");
    fs::write(&path, &text).unwrap();
    let old_metadata = scan_archive_article_metadata(dir).unwrap();
    let members = scan_archive_window_metadata(
        dir,
        date("2026-10-02T00:00:00Z"),
        date("2026-10-07T04:57:36Z"),
    )
    .unwrap();
    assert_eq!(members.len(), 1);
    assert_eq!(
        members[0].content_hash,
        old_metadata[0].content_hash.as_deref().unwrap()
    );
    assert_eq!(members[0].fetched_utc, None);
    assert_eq!(fs::read_to_string(&path).unwrap(), text);
    assert_eq!(fs::read_dir(dir).unwrap().count(), 1);
    // The read-only scanner tolerates missing dates; export requires them.
    assert!(matches!(
        build_triage_archive(
            dir,
            "measurement",
            &[],
            Some(date("2026-10-02T00:00:00Z")),
            false,
            &HashMap::new(),
            &HashMap::new(),
            &HashMap::new(),
        ),
        Err(ExportError::MissingFrontmatter(_))
    ));
}
