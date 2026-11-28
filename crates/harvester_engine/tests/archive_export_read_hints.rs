//! Archive export read hints (Follow-up, 2026-11-26).
//!
//! The export may skip reading a root article only when the startup scan index
//! proves the file is unchanged since the scan and dated before the checkpoint.
//! Every other file is read exactly as before. The hinted export must produce a
//! byte-identical archive to the full read, including files added, changed and
//! deleted after the scan.
use std::{
    collections::HashMap,
    fs,
    path::Path,
    time::{Duration, SystemTime},
};

use chrono::{DateTime, Utc};
use harvester_engine::llm::PromptRegistry;
use harvester_engine::{
    build_markdown_document, build_triage_archive_with, ArchiveReadProgress, CorpusScanIndex,
    ExportSummary, WhitespaceTokenCounter,
};
use tempfile::tempdir;

const SINCE: &str = "2026-11-01T00:00:00Z";

fn date(raw: &str) -> DateTime<Utc> {
    DateTime::parse_from_rfc3339(raw)
        .unwrap()
        .with_timezone(&Utc)
}

fn article(dir: &Path, filename: &str, url: &str, fetched: &str) {
    let (_, text) = build_markdown_document(
        url,
        Some(filename),
        "utf-8",
        fetched,
        "A clean article body with a few useful facts for the export.",
        &WhitespaceTokenCounter,
    );
    fs::write(dir.join(filename), text).unwrap();
}

fn bump_mtime(path: &Path) {
    let later = SystemTime::now() + Duration::from_secs(3_600);
    fs::File::options()
        .write(true)
        .open(path)
        .unwrap()
        .set_modified(later)
        .unwrap();
}

struct Fixture {
    _tmp: tempfile::TempDir,
    window_urls: Vec<String>,
}

impl Fixture {
    fn dir(&self) -> &Path {
        self._tmp.path()
    }
}

/// 60 old and 8 new articles, a malformed date, an archive artifact and a
/// non-article Markdown file; the scan index is populated before the
/// after-scan edits are made.
fn fixture_with_scanned_index(index: &mut CorpusScanIndex) -> Fixture {
    let tmp = tempdir().unwrap();
    let dir = tmp.path();
    for i in 0..60 {
        article(
            dir,
            &format!("old-{i:03}.md"),
            &format!("https://example.com/old/{i}"),
            &format!("2026-10-{:02}T08:00:00Z", (i % 28) + 1),
        );
    }
    let mut window_urls = Vec::new();
    for i in 0..8 {
        let url = format!("https://example.com/new/{i}");
        article(
            dir,
            &format!("new-{i:03}.md"),
            &url,
            &format!("2026-11-{:02}T08:00:00Z", i + 2),
        );
        window_urls.push(url);
    }
    article(
        dir,
        "bad-date.md",
        "https://example.com/bad-date",
        "malformed",
    );
    window_urls.push("https://example.com/bad-date".into());
    fs::write(
        dir.join("renamed-export.md"),
        "===== ARCHIVE INDEX =====\ndoc_count: 0\n",
    )
    .unwrap();
    fs::write(dir.join("notes.md"), "no frontmatter here\n").unwrap();

    let registry = PromptRegistry::with_defaults();
    index
        .load_url_delta(dir, 20_000, &registry, &window_urls, None, &[], |_| {})
        .unwrap();

    // After the scan: one file added inside the window.
    article(
        dir,
        "late.md",
        "https://example.com/late",
        "2026-11-20T08:00:00Z",
    );
    window_urls.push("https://example.com/late".into());
    // One old file rewritten into the window. The date has the same length, so
    // only the modification time tells the scan index that it changed.
    article(
        dir,
        "old-005.md",
        "https://example.com/old/5",
        "2026-11-06T08:00:00Z",
    );
    bump_mtime(&dir.join("old-005.md"));
    window_urls.push("https://example.com/old/5".into());
    // One window file deleted after the scan.
    fs::remove_file(dir.join("new-003.md")).unwrap();

    Fixture {
        _tmp: tmp,
        window_urls,
    }
}

fn export(
    dir: &Path,
    urls: &[String],
    since: Option<DateTime<Utc>>,
    index: Option<&CorpusScanIndex>,
    progress: &mut Vec<ArchiveReadProgress>,
) -> (ExportSummary, Vec<u8>) {
    let hints = index.map(CorpusScanIndex::archive_read_hints);
    let summary = build_triage_archive_with(
        dir,
        harvester_engine::ArchiveTarget::InOutput {
            basename: "archive.md",
        },
        urls,
        harvester_engine::ArchiveWindow::from_submission(
            since,
            "2026-12-01T00:00:00Z".parse().unwrap(),
        ),
        false,
        &HashMap::new(),
        &HashMap::new(),
        &HashMap::new(),
        None,
        hints.as_ref(),
        &mut |p| progress.push(p),
    )
    .unwrap();
    let bytes = fs::read(&summary.output_path).unwrap();
    (summary, bytes)
}

#[test]
fn hinted_export_is_byte_identical_to_the_full_read_after_adds_changes_and_deletes() {
    let mut index = CorpusScanIndex::default();
    let fixture = fixture_with_scanned_index(&mut index);
    let since = Some(date(SINCE));

    let (full, full_bytes) = export(
        fixture.dir(),
        &fixture.window_urls,
        since,
        None,
        &mut Vec::new(),
    );
    let (hinted, hinted_bytes) = export(
        fixture.dir(),
        &fixture.window_urls,
        since,
        Some(&index),
        &mut Vec::new(),
    );

    assert_eq!(hinted_bytes, full_bytes);
    assert_eq!(hinted.doc_count, full.doc_count);
    assert_eq!(hinted.window_count, full.window_count);
    assert_eq!(hinted.unexported_by_priority, full.unexported_by_priority);
    // The changed and the added file both made it into the archive.
    let text = String::from_utf8_lossy(&hinted_bytes);
    assert!(text.contains("https://example.com/old/5"));
    assert!(text.contains("https://example.com/late"));
}

#[test]
fn hinted_export_reads_a_file_with_changed_length_and_unchanged_mtime() {
    let mut index = CorpusScanIndex::default();
    let mut fixture = fixture_with_scanned_index(&mut index);
    let path = fixture.dir().join("old-006.md");
    let original = fs::metadata(&path).unwrap();
    let modified = original.modified().unwrap();
    let updated = fs::read_to_string(&path)
        .unwrap()
        .replace("2026-10-07T08:00:00Z", "2026-11-07T08:00:00Z");
    fs::write(&path, format!("{updated}\nAn additional detail.\n")).unwrap();
    fs::File::options()
        .write(true)
        .open(&path)
        .unwrap()
        .set_modified(modified)
        .unwrap();
    let rewritten = fs::metadata(path).unwrap();
    assert_eq!(rewritten.modified().unwrap(), modified);
    assert_ne!(rewritten.len(), original.len());

    let changed_url = "https://example.com/old/6";
    fixture.window_urls.push(changed_url.into());
    let dir = fixture.dir();
    let urls = &fixture.window_urls;
    let since = Some(date(SINCE));
    let (full, full_bytes) = export(dir, urls, since, None, &mut Vec::new());
    let (hinted, hinted_bytes) = export(dir, urls, since, Some(&index), &mut Vec::new());

    assert!(str::from_utf8(&hinted_bytes).unwrap().contains(changed_url));
    assert_eq!(hinted_bytes, full_bytes);
    assert_eq!(hinted.files_read, 13);
    assert_eq!(full.files_read, 71);
}

#[test]
fn hinted_export_reads_only_files_the_index_cannot_rule_out() {
    let mut index = CorpusScanIndex::default();
    let fixture = fixture_with_scanned_index(&mut index);
    let since = Some(date(SINCE));

    let (full, _) = export(
        fixture.dir(),
        &fixture.window_urls,
        since,
        None,
        &mut Vec::new(),
    );
    let (hinted, _) = export(
        fixture.dir(),
        &fixture.window_urls,
        since,
        Some(&index),
        &mut Vec::new(),
    );

    // Root listing: 60 old + 7 remaining new + bad-date + renamed-export.md + notes.md + late.md.
    assert_eq!(full.files_read, 71);
    // Read: 7 new, late.md (unknown to the index), old-005.md (changed),
    // bad-date.md (no parseable date), renamed-export.md and notes.md (not articles).
    assert_eq!(hinted.files_read, 12);
}

#[test]
fn without_a_checkpoint_every_file_is_read_even_with_hints() {
    let mut index = CorpusScanIndex::default();
    let fixture = fixture_with_scanned_index(&mut index);
    let (full, full_bytes) = export(
        fixture.dir(),
        &fixture.window_urls,
        None,
        None,
        &mut Vec::new(),
    );
    let (hinted, hinted_bytes) = export(
        fixture.dir(),
        &fixture.window_urls,
        None,
        Some(&index),
        &mut Vec::new(),
    );
    assert_eq!(hinted.files_read, full.files_read);
    assert_eq!(hinted_bytes, full_bytes);
}

#[test]
fn an_empty_index_falls_back_to_the_full_read() {
    let mut scanned = CorpusScanIndex::default();
    let fixture = fixture_with_scanned_index(&mut scanned);
    let empty = CorpusScanIndex::default();
    let since = Some(date(SINCE));
    let (full, full_bytes) = export(
        fixture.dir(),
        &fixture.window_urls,
        since,
        None,
        &mut Vec::new(),
    );
    let (hinted, hinted_bytes) = export(
        fixture.dir(),
        &fixture.window_urls,
        since,
        Some(&empty),
        &mut Vec::new(),
    );
    assert_eq!(hinted.files_read, full.files_read);
    assert_eq!(hinted_bytes, full_bytes);
}

#[test]
fn progress_is_monotonic_and_ends_at_the_listed_total() {
    let mut index = CorpusScanIndex::default();
    let fixture = fixture_with_scanned_index(&mut index);
    let mut progress = Vec::new();
    export(
        fixture.dir(),
        &fixture.window_urls,
        Some(date(SINCE)),
        Some(&index),
        &mut progress,
    );
    assert!(!progress.is_empty());
    assert!(progress
        .windows(2)
        .all(|pair| pair[0].files_checked <= pair[1].files_checked));
    let last = progress.last().unwrap();
    assert_eq!(last.files_total, 71);
    assert_eq!(last.files_checked, last.files_total);
    assert!(progress.iter().all(|p| p.files_total == 71));
}
