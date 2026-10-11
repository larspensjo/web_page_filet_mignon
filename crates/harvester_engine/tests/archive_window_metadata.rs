use std::{collections::HashMap, fs, path::Path};

use chrono::{DateTime, Utc};
use harvester_engine::{
    build_markdown_document, build_triage_archive, scan_archive_article_metadata,
    scan_archive_window_metadata, ArchiveFoldReason, ArchiveFoldReport, ArchiveFoldedArticle,
    WhitespaceTokenCounter,
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
        None,
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
fn metadata_reuses_clean_text_hash_and_skips_undated_articles_like_exporter() {
    let tmp = tempdir().unwrap();
    let dir = tmp.path();
    article(
        dir,
        "valid.md",
        "https://example.com/valid",
        "2026-10-03T00:00:00Z",
        "Valid",
    );
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
        old_metadata
            .iter()
            .find(|article| article.url == "https://example.com/valid")
            .unwrap()
            .content_hash
            .as_deref()
            .unwrap()
    );
    assert_eq!(members[0].url, "https://example.com/valid");
    assert_eq!(
        members[0].fetched_utc.as_deref(),
        Some("2026-10-03T00:00:00Z")
    );
    assert_eq!(fs::read_to_string(&path).unwrap(), text);
    assert_eq!(fs::read_dir(dir).unwrap().count(), 2);
    let export = build_triage_archive(
        dir,
        "measurement",
        &[],
        Some(date("2026-10-02T00:00:00Z")),
        false,
        &HashMap::new(),
        &HashMap::new(),
        &HashMap::new(),
        None,
    )
    .unwrap();
    assert_eq!(export.window_count, Some(members.len()));
    assert_eq!(export.doc_count, 0);
    assert_eq!(export.unexported_by_priority, Some([0, 0, 0, 0, 0, 1]));
}

#[test]
fn bounded_exports_skip_unparseable_articles_with_and_without_fold_report() {
    let tmp = tempdir().unwrap();
    let dir = tmp.path();
    let linked = dir.join("linked");
    fs::create_dir(&linked).unwrap();
    let since = date("2026-10-02T00:00:00Z");
    let until = date("2026-10-07T04:57:36Z");
    let missing_date = "https://example.com/missing-date";
    let no_frontmatter = "https://example.com/no-frontmatter";
    fs::write(
        dir.join("missing-date.md"),
        format!("---\nurl: {missing_date}\ntitle: Missing date\n---\n\nbody\n"),
    )
    .unwrap();
    fs::write(
        linked.join("no-frontmatter.md"),
        "body without frontmatter\n",
    )
    .unwrap();
    // Empty required values have the same exclusion rule in both readers.
    for field in ["url", "title", "fetched_utc"] {
        let text = "---\nurl: https://example.com/empty\ntitle: Empty\nfetched_utc: 2026-10-03T00:00:00Z\n---\n\nbody\n";
        let text = text
            .lines()
            .map(|line| {
                if line.starts_with(&format!("{field}:")) {
                    format!("{field}: \"\"")
                } else {
                    line.to_string()
                }
            })
            .collect::<Vec<_>>()
            .join("\n");
        fs::write(linked.join(format!("empty-{field}.md")), text).unwrap();
    }
    let promoted = "https://example.com/promoted";
    let same_event = "https://example.com/same-event";
    let identical = "https://example.com/identical";
    for (filename, url, fetched) in [
        ("promoted.md", promoted, "malformed"),
        ("same-event.md", same_event, "2026-10-03T00:00:00Z"),
        ("identical.md", identical, "2026-10-03T00:00:00Z"),
        ("old.md", "https://example.com/old", "2026-10-01T00:00:00Z"),
    ] {
        article(dir, filename, url, fetched, filename);
    }
    let folds: ArchiveFoldReport = [(
        harvester_engine::archive_url_key(missing_date),
        [
            (no_frontmatter, ArchiveFoldReason::SameEvent),
            (promoted, ArchiveFoldReason::SameEvent),
            (same_event, ArchiveFoldReason::SameEvent),
            (identical, ArchiveFoldReason::IdenticalContent),
        ]
        .into_iter()
        .map(|(url, reason)| ArchiveFoldedArticle {
            url: url.into(),
            reason,
            signal_key: None,
        })
        .collect(),
    )]
    .into_iter()
    .collect();
    let members = scan_archive_window_metadata(dir, since, until).unwrap();
    assert_eq!(members.len(), 3);
    for report in [Some(&folds), None] {
        let export = build_triage_archive(
            dir,
            "archive.md",
            &[missing_date.into(), no_frontmatter.into()],
            Some(since),
            false,
            &HashMap::new(),
            &HashMap::new(),
            &HashMap::new(),
            report,
        )
        .unwrap();
        assert_eq!(export.window_count, Some(members.len()));
        let remainder: usize = export.unexported_by_priority.unwrap().iter().sum();
        assert_eq!(members.len(), export.doc_count + remainder);
        let archive = fs::read_to_string(export.output_path).unwrap();
        if report.is_some() {
            assert_eq!(export.doc_count, 1);
            assert!(archive.contains(&format!("url: {promoted}\n")));
            assert!(!archive.contains(&format!("url: {missing_date}\n")));
            assert_eq!(export.unexported_folded, Some([1, 1]));
            let [same_event, identical_content] = export.unexported_folded.unwrap();
            assert!(same_event + identical_content <= remainder);
        } else {
            assert_eq!(export.doc_count, 0);
            assert_eq!(export.unexported_folded, None);
        }
        assert_eq!(
            scan_archive_window_metadata(dir, since, until).unwrap(),
            members
        );
    }
}
