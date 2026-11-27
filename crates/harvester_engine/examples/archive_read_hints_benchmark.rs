//! Local fixture benchmark for checkpoint-scoped archive reads.
use std::{collections::HashMap, fs, path::Path, time::Instant};

use chrono::{DateTime, Utc};
use harvester_engine::llm::PromptRegistry;
use harvester_engine::{
    build_markdown_document, build_triage_archive_with, ArchiveReadHints, CorpusScanIndex,
    WhitespaceTokenCounter,
};

fn write_article(dir: &Path, id: usize, fetched_utc: &str, body: &str) -> String {
    let url = format!("https://example.invalid/fixture/{id}");
    let (_, markdown) = build_markdown_document(
        &url,
        Some("Fixture article"),
        "utf-8",
        fetched_utc,
        body,
        &WhitespaceTokenCounter,
    );
    fs::write(dir.join(format!("article-{id:05}.md")), markdown).unwrap();
    url
}

fn main() {
    const ROOT_BEFORE: usize = 14_000;
    const IN_WINDOW: usize = 660;
    const LINKED_BEFORE: usize = 3_340;
    const SELECTED: usize = 194;
    const BEFORE: &str = "2026-10-20T08:00:00Z";
    const SINCE: &str = "2026-11-01T00:00:00Z";

    let fixture = tempfile::tempdir().unwrap();
    let root = fixture.path();
    let linked = root.join("linked");
    fs::create_dir(&linked).unwrap();
    let body =
        "The local fixture article contains some ordinary text to read and parse. ".repeat(114);
    for id in 0..ROOT_BEFORE {
        write_article(root, id, BEFORE, &body);
    }
    let window_urls: Vec<_> = (ROOT_BEFORE..ROOT_BEFORE + IN_WINDOW)
        .map(|id| write_article(root, id, SINCE, &body))
        .collect();
    // Legacy linked articles make up the remaining files and exercise the
    // full-read fallback for paths outside the root scan index.
    for id in ROOT_BEFORE + IN_WINDOW..ROOT_BEFORE + IN_WINDOW + LINKED_BEFORE {
        write_article(&linked, id, BEFORE, &body);
    }
    let since = DateTime::parse_from_rfc3339(SINCE)
        .unwrap()
        .with_timezone(&Utc);
    let mut index = CorpusScanIndex::default();
    index
        .load_delta(
            root,
            20_000,
            &PromptRegistry::with_defaults(),
            &window_urls,
            Some(since),
            &[],
            |_| {},
        )
        .unwrap();
    let export = |label, hints: Option<&ArchiveReadHints>, started: Instant| {
        let summary = build_triage_archive_with(
            root,
            harvester_engine::ArchiveTarget::InOutput {
                basename: "archive.md",
            },
            &window_urls[..SELECTED],
            harvester_engine::ArchiveWindow::from_submission(
                Some(since),
                "2026-12-01T00:00:00Z".parse().unwrap(),
            ),
            false,
            &HashMap::new(),
            &HashMap::new(),
            &HashMap::new(),
            None,
            hints,
            &mut |_| {},
        )
        .unwrap();
        println!(
            "{label}: {:.3} s, files_read={}, window_count={}, doc_count={}",
            started.elapsed().as_secs_f64(),
            summary.files_read,
            summary.window_count.unwrap(),
            summary.doc_count,
        );
        (summary, fs::read(root.join("archive.md")).unwrap())
    };
    let (full, full_bytes) = export("full read", None, Instant::now());
    let hinted_started = Instant::now();
    let hints = index.archive_read_hints();
    let (hinted, hinted_bytes) = export("hinted", Some(&hints), hinted_started);
    assert_eq!(full.files_read, ROOT_BEFORE + IN_WINDOW + LINKED_BEFORE);
    assert_eq!(hinted.files_read, IN_WINDOW + LINKED_BEFORE);
    assert!(full_bytes == hinted_bytes, "archive bytes differ");
}
