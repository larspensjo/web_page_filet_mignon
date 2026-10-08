use harvester_engine::llm::PromptRegistry;
use harvester_engine::{CorpusScanIndex, HeldArticle, TriageArticleDelta};
use std::{fs, path::Path};

fn write(dir: &Path, name: &str, url: &str, date: &str, text: &str) {
    fs::write(
        dir.join(name),
        format!(
            "---\nurl: {url}\ntitle: {name}\nfetched_utc: {date}\n---\n\n{}",
            text.repeat(500)
        ),
    )
    .unwrap();
}
fn held(delta: &TriageArticleDelta) -> Vec<HeldArticle> {
    delta
        .members
        .iter()
        .map(|m| HeldArticle {
            url: m.url.clone(),
            content_hash: m.content_hash.clone(),
            preparation_budget: delta.preparation_budget,
        })
        .collect()
}

#[test]
fn unchanged_fingerprint_is_never_read_even_with_unparseable_bytes() {
    let dir = tempfile::tempdir().unwrap();
    let url = "https://example.com/a".to_string();
    write(
        dir.path(),
        "a.md",
        &url,
        "2026-09-20T00:00:00Z",
        "original text ",
    );
    let registry = PromptRegistry::with_defaults();
    let mut index = CorpusScanIndex::default();
    let (first, _) = index
        .load_delta(
            dir.path(),
            20_000,
            &registry,
            std::slice::from_ref(&url),
            None,
            &[],
            |_| {},
        )
        .unwrap();
    let path = dir.path().join("a.md");
    let metadata = fs::metadata(&path).unwrap();
    fs::write(&path, vec![0xff; metadata.len() as usize]).unwrap();
    fs::File::options()
        .write(true)
        .open(path)
        .unwrap()
        .set_modified(metadata.modified().unwrap())
        .unwrap();
    let (second, stats) = index
        .load_delta(
            dir.path(),
            20_000,
            &registry,
            &[url],
            None,
            &held(&first),
            |_| {},
        )
        .unwrap();
    assert_eq!(second.members, first.members);
    assert!(second.articles.is_empty());
    assert_eq!((stats.read, stats.reprepared, stats.reused), (0, 0, 1));
}

#[test]
fn scan_tracks_new_changed_deleted_files_and_requested_order_excluding_archives() {
    let dir = tempfile::tempdir().unwrap();
    let urls: Vec<String> = vec![
        "https://example.com/z".into(),
        "https://example.com/a".into(),
    ];
    write(
        dir.path(),
        "z.md",
        &urls[0],
        "2026-09-20T00:00:00Z",
        "first ",
    );
    fs::write(dir.path().join("archive.md"), "===== archived output =====").unwrap();
    let registry = PromptRegistry::with_defaults();
    let mut index = CorpusScanIndex::default();
    let (first, stats) = index
        .load_delta(dir.path(), 20_000, &registry, &urls, None, &[], |_| {})
        .unwrap();
    assert_eq!(stats.read, 2);
    assert_eq!(first.members.len(), 1);
    write(
        dir.path(),
        "a.md",
        &urls[1],
        "2026-09-21T00:00:00Z",
        "new article ",
    );
    let (second, stats) = index
        .load_delta(
            dir.path(),
            20_000,
            &registry,
            &urls,
            None,
            &held(&first),
            |_| {},
        )
        .unwrap();
    assert_eq!((stats.read, stats.reused), (1, 2));
    assert_eq!(
        second.members.iter().map(|m| &m.url).collect::<Vec<_>>(),
        vec![&urls[0], &urls[1]]
    );
    assert_eq!(second.articles.len(), 1);
    write(
        dir.path(),
        "z.md",
        &urls[0],
        "2026-09-20T00:00:00Z",
        "changed article contents ",
    );
    fs::remove_file(dir.path().join("a.md")).unwrap();
    let (third, stats) = index
        .load_delta(
            dir.path(),
            20_000,
            &registry,
            &urls,
            None,
            &held(&second),
            |_| {},
        )
        .unwrap();
    assert_eq!((stats.read, stats.removed, stats.reused), (1, 1, 1));
    assert_eq!(third.members.len(), 1);
    assert_ne!(third.members[0].content_hash, first.members[0].content_hash);
    assert_eq!(third.articles.len(), 1);
}

#[test]
fn duplicate_url_selects_first_filename_once_and_reuses_held_preparation() {
    let dir = tempfile::tempdir().unwrap();
    let url = "https://example.com/same".to_string();
    write(dir.path(), "a.md", &url, "2026-09-20T00:00:00Z", "first ");
    write(dir.path(), "z.md", &url, "2026-09-20T00:00:00Z", "second ");
    let registry = PromptRegistry::with_defaults();
    let mut index = CorpusScanIndex::default();
    let (first, _) = index
        .load_delta(
            dir.path(),
            20_000,
            &registry,
            std::slice::from_ref(&url),
            None,
            &[],
            |_| {},
        )
        .unwrap();
    assert_eq!(first.members.len(), 1);
    assert_eq!(first.articles.len(), 1);
    let (second, stats) = index
        .load_delta(
            dir.path(),
            20_000,
            &registry,
            &[url],
            None,
            &held(&first),
            |_| {},
        )
        .unwrap();
    assert_eq!(second.members, first.members);
    assert!(second.articles.is_empty());
    assert_eq!((stats.read, stats.reprepared), (0, 0));
}

#[test]
fn since_filter_uses_cached_timestamp_and_includes_missing_or_invalid_dates() {
    let dir = tempfile::tempdir().unwrap();
    let urls: Vec<String> = (0..4).map(|n| format!("https://example.com/{n}")).collect();
    for (n, date) in [
        "2026-09-19T00:00:00Z",
        "2026-09-21T00:00:00Z",
        "invalid",
        "missing",
    ]
    .iter()
    .enumerate()
    {
        write(dir.path(), &format!("{n}.md"), &urls[n], date, "text ");
    }
    let missing = dir.path().join("3.md");
    fs::write(
        &missing,
        fs::read_to_string(&missing)
            .unwrap()
            .replace("fetched_utc: missing\n", ""),
    )
    .unwrap();
    let registry = PromptRegistry::with_defaults();
    let mut index = CorpusScanIndex::default();
    let (first, _) = index
        .load_delta(dir.path(), 20_000, &registry, &urls, None, &[], |_| {})
        .unwrap();
    let since = Some("2026-09-20T00:00:00Z".parse().unwrap());
    let (filtered, stats) = index
        .load_delta(
            dir.path(),
            20_000,
            &registry,
            &urls,
            since,
            &held(&first),
            |_| {},
        )
        .unwrap();
    assert_eq!((stats.read, stats.reused), (0, 4));
    assert_eq!(
        filtered.members.iter().map(|m| &m.url).collect::<Vec<_>>(),
        vec![&urls[1], &urls[2], &urls[3]]
    );
    assert!(filtered.articles.is_empty());
}

#[test]
fn triage_delta_preserves_summary_hash_and_preparation_budget() {
    let dir = tempfile::tempdir().unwrap();
    let urls: Vec<String> = vec!["https://example.com/a".into()];
    write(
        dir.path(),
        "a.md",
        &urls[0],
        "2026-09-20T00:00:00Z",
        "long article text ",
    );
    let registry = PromptRegistry::with_defaults();
    let (delta, _) = CorpusScanIndex::default()
        .load_delta(dir.path(), 4_000, &registry, &urls, None, &[], |_| {})
        .unwrap();
    let metadata = harvester_engine::scan_archive_article_metadata(dir.path()).unwrap();
    assert_eq!(
        delta.articles[0].content_hash,
        metadata[0].content_hash.clone().unwrap()
    );
    assert_eq!(
        delta.preparation_budget,
        harvester_engine::summary_preparation_budget(4_000, &registry).unwrap()
    );
    assert_eq!(delta.articles[0].url, urls[0]);
    assert!(delta.articles[0].prepared_text.len() <= delta.preparation_budget);
}
