use harvester_engine::llm::PromptRegistry;
use harvester_engine::{
    build_triage_archive_with, CorpusScanIndex, HeldArticle, TriageArticleDelta, TriageLoadEntry,
};
use std::{collections::HashMap, fs, path::Path};

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

const REQUESTED: &str =
    "https://digitimes.com/news/a20261126VL214/huawei-ascend-chips-nvidia-atlas.html";
const FINAL: &str =
    "https://www.digitimes.com/newsshow/article.asp?datePublish=2026/11/26&pages=vl&seq=214";
const FETCHED: &str = "2026-11-27T15:34:37Z";
const BEFORE_WINDOW: &str = "2026-11-26T15:34:37Z";

type RedirectedJob<'a> = (&'a str, Option<&'a str>, Option<&'a str>);

fn redirected_members(dir: &Path, jobs: &[RedirectedJob<'_>]) -> Vec<String> {
    load_redirected_jobs(dir, jobs, None, None)
        .members
        .into_iter()
        .map(|member| member.url)
        .collect()
}

fn load_redirected_jobs(
    dir: &Path,
    jobs: &[RedirectedJob<'_>],
    since: Option<&str>,
    request_id: Option<u64>,
) -> TriageArticleDelta {
    let entries: Vec<_> = jobs
        .iter()
        .enumerate()
        .map(|(index, (url, final_url, fetched))| TriageLoadEntry::Job {
            id: Some(index as u64 + 1),
            url: url.to_string(),
            final_url: final_url.map(str::to_owned),
            fetched_utc: fetched.map(|date| date.parse().unwrap()),
        })
        .collect();
    CorpusScanIndex::default()
        .load_delta(
            dir,
            20_000,
            &PromptRegistry::with_defaults(),
            &entries,
            since.map(|date| date.parse().unwrap()),
            &[],
            request_id,
            |_| {},
        )
        .unwrap()
        .0
}

#[test]
fn redirected_job_matches_by_final_url() {
    let tmp = tempfile::tempdir().unwrap();
    let output = tmp.path().join("output");
    fs::create_dir(&output).unwrap();
    fs::write(
        output.join("a.md"),
        format!(
            "---\nurl: {FINAL}\ntitle: Redirected article\nfetched_utc: {FETCHED}\n---\n\n{}",
            "article ".repeat(300)
        ),
    )
    .unwrap();
    assert_eq!(
        redirected_members(&output, &[(REQUESTED, Some(FINAL), None)]),
        [FINAL]
    );
}

#[test]
fn legacy_redirect_matches_by_exact_fetch_instant() {
    let dir = tempfile::tempdir().unwrap();
    write(dir.path(), "a.md", FINAL, FETCHED, "article ");
    for timestamp in [FETCHED, "2026-11-27T16:34:37.000+01:00"] {
        assert_eq!(
            redirected_members(dir.path(), &[(REQUESTED, None, Some(timestamp))]),
            [FINAL]
        );
    }
}

#[test]
fn legacy_redirect_with_two_files_at_one_instant_is_ambiguous() {
    let dir = tempfile::tempdir().unwrap();
    write(dir.path(), "a.md", FINAL, FETCHED, "article ");
    write(
        dir.path(),
        "b.md",
        "https://example.com/b",
        FETCHED,
        "other ",
    );
    assert!(redirected_members(dir.path(), &[(REQUESTED, None, Some(FETCHED))]).is_empty());
}

#[test]
fn legacy_redirect_with_two_jobs_at_one_instant_is_ambiguous() {
    let dir = tempfile::tempdir().unwrap();
    write(dir.path(), "a.md", FINAL, FETCHED, "article ");
    assert!(redirected_members(
        dir.path(),
        &[
            (REQUESTED, None, Some(FETCHED)),
            ("https://example.com/other", None, Some(FETCHED))
        ],
    )
    .is_empty());
}

#[test]
fn query_identified_redirects_match_distinct_files_in_job_order() {
    let dir = tempfile::tempdir().unwrap();
    let other = FINAL.replace("seq=214", "seq=227");
    write(dir.path(), "a.md", FINAL, FETCHED, "article ");
    write(dir.path(), "b.md", &other, FETCHED, "other ");
    assert_eq!(
        redirected_members(
            dir.path(),
            &[
                ("https://digitimes.com/news/other", Some(&other), None),
                (REQUESTED, Some(FINAL), None)
            ],
        ),
        [other.as_str(), FINAL]
    );
    fs::remove_file(dir.path().join("b.md")).unwrap();
    assert!(redirected_members(
        dir.path(),
        &[("https://digitimes.com/news/other", Some(&other), None)],
    )
    .is_empty());
}

#[test]
fn query_preserving_aliases_win_for_requested_urls() {
    let dir = tempfile::tempdir().unwrap();
    let other = FINAL.replace("seq=214", "seq=227");
    let other_alias = other.replace("https://www.", "http://");
    write(dir.path(), "a.md", FINAL, FETCHED, "article ");
    write(dir.path(), "b.md", &other, FETCHED, "other ");
    write(
        dir.path(),
        "c.md",
        "https://example.com/timestamp-decoy",
        BEFORE_WINDOW,
        "decoy ",
    );
    assert_eq!(
        redirected_members(
            dir.path(),
            &[
                (&other_alias, None, Some(BEFORE_WINDOW)),
                (FINAL, None, None),
            ]
        ),
        [other.as_str(), FINAL]
    );
}

#[test]
fn unique_fetch_instant_wins_before_query_stripping_even_with_a_saved_final_url() {
    let dir = tempfile::tempdir().unwrap();
    let base = "https://example.com/article";
    let requested = "https://example.com/article?source=feed";
    write(dir.path(), "a.md", base, BEFORE_WINDOW, "alias ");
    write(dir.path(), "b.md", FINAL, FETCHED, "article ");

    assert_eq!(
        redirected_members(dir.path(), &[(requested, None, None)]),
        [base]
    );
    for saved_final in [None, Some("https://example.com/previous-redirect")] {
        assert_eq!(
            redirected_members(dir.path(), &[(requested, saved_final, Some(FETCHED))]),
            [FINAL]
        );
    }
}

#[test]
fn unique_fetch_instant_cannot_reclaim_an_article_matched_by_url() {
    let dir = tempfile::tempdir().unwrap();
    write(dir.path(), "article.md", FINAL, FETCHED, "article ");
    assert_eq!(
        redirected_members(
            dir.path(),
            &[(REQUESTED, None, Some(FETCHED)), (FINAL, None, None)]
        ),
        [FINAL]
    );
}

#[test]
fn url_matches_prefer_an_in_window_file() {
    for (requested, older, newer) in [
        (FINAL, FINAL, FINAL),
        (
            "http://www.example.com/article#section",
            "https://www.example.com/article",
            "https://example.com/article",
        ),
        (
            "https://example.com/article",
            "https://example.com/article?id=1",
            "https://example.com/article?id=2",
        ),
    ] {
        let dir = tempfile::tempdir().unwrap();
        write(dir.path(), "a.md", older, BEFORE_WINDOW, "older ");
        write(dir.path(), "b.md", newer, FETCHED, "current ");

        let delta =
            load_redirected_jobs(dir.path(), &[(requested, None, None)], Some(FETCHED), None);
        assert_eq!(delta.members.len(), 1, "{requested}");
        assert_eq!(delta.members[0].url, newer);
        assert_eq!(delta.members[0].fetched_utc.as_deref(), Some(FETCHED));
        assert_eq!(delta.articles.len(), 1);
    }
}

#[test]
fn out_of_window_url_match_cannot_admit_a_file_by_timestamp() {
    let dir = tempfile::tempdir().unwrap();
    write(dir.path(), "older.md", FINAL, BEFORE_WINDOW, "older ");
    write(dir.path(), "decoy.md", REQUESTED, FETCHED, "current ");
    let jobs = [(FINAL, None, Some(FETCHED))];

    assert_eq!(redirected_members(dir.path(), &jobs), [FINAL]);
    let delta = load_redirected_jobs(dir.path(), &jobs, Some(FETCHED), None);
    assert!(delta.members.is_empty());
    assert!(delta.articles.is_empty());
}

#[test]
fn different_fetch_instant_produces_one_capped_warning() {
    // The log facade has one logger per process. Run this case alone so other tests
    // can initialize their own logger without affecting the warning assertions.
    const ISOLATED: &str = "HARVESTER_TEST_CORPUS_WARNING_ISOLATED";
    if std::env::var_os(ISOLATED).is_none() {
        let child = std::process::Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "different_fetch_instant_produces_one_capped_warning",
            ])
            .env(ISOLATED, "1")
            .output()
            .unwrap();
        let stdout = String::from_utf8_lossy(&child.stdout);
        assert!(
            child.status.success() && stdout.contains("1 passed"),
            "{stdout}\n{}",
            String::from_utf8_lossy(&child.stderr)
        );
        return;
    }

    let dir = tempfile::tempdir().unwrap();
    let log_path = dir.path().join("warnings.log");
    engine_logging::initialize_file_only_at(&log_path);
    write(dir.path(), "a.md", FINAL, FETCHED, "article ");
    let outside = "https://example.com/outside-window";
    let claimed = "https://example.com/already-claimed";
    let urls: Vec<_> = (0..21)
        .map(|i| format!("https://example.com/missing/{i:02}"))
        .collect();
    let unnamed = [
        (outside, None, Some(BEFORE_WINDOW)),
        (claimed, None, Some(FETCHED)),
        (FINAL, None, None),
    ];
    let mut jobs = unnamed.to_vec();
    jobs.push((REQUESTED, None, Some("2026-11-27T15:34:37.000000001Z")));
    jobs.extend(urls.iter().map(|url| (url.as_str(), None, None)));
    let delta = load_redirected_jobs(dir.path(), &jobs, Some(FETCHED), Some(42));
    assert_eq!(delta.members.len(), 1);
    assert_eq!(delta.members[0].url, FINAL);
    load_redirected_jobs(dir.path(), &unnamed, Some(FETCHED), Some(43));

    log::logger().flush();
    let log = fs::read_to_string(log_path).unwrap();
    let warnings: Vec<_> = log
        .lines()
        .filter(|line| line.contains("[corpus-index]"))
        .collect();
    assert_eq!(warnings.len(), 2, "{log}");
    let warning = warnings[0];
    assert!(
        warning.contains("[corpus-index] request_id=42"),
        "{warning}"
    );
    assert!(warning.contains("22 completed job(s) in the load window or without a fetch time could not be matched to an article file"), "{warning}");
    assert!(
        warning.contains(
            "1 completed job(s) outside the load window could not be matched to an article file"
        ),
        "{warning}"
    );
    assert!(
        warning.contains(
            "1 completed job(s) resolved to an article file already matched to another job"
        ),
        "{warning}"
    );
    assert!(warning.contains("Showing 20 of 22"), "{warning}");
    assert!(warning.contains(REQUESTED));
    assert!(warning.contains("job_id=4)"));
    assert!(warning.contains(&urls[18]));
    for url in [
        urls[19].as_str(),
        urls[20].as_str(),
        outside,
        claimed,
        FINAL,
    ] {
        assert!(!warning.contains(url), "{warning}");
    }
    assert!(warnings[1].contains("[corpus-index] request_id=43"));
    assert!(warnings[1].contains("0 completed job(s) in the load window"));
    assert!(!warnings[1].contains("Showing"));
}

#[test]
fn archive_hints_preserve_frontmatter_and_checkpoint_boundary_behavior() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    let linked = root.join("linked");
    fs::create_dir(&linked).unwrap();
    let url = "https://example.com/a";
    let old = "2026-10-20T00:00:00Z";
    let cutoff = "2026-11-01T00:00:00Z";
    write(root, "a-old-duplicate.md", url, old, "old ");
    // The last fetched_utc wins and its offset resolves to the cutoff exactly.
    fs::write(
        root.join("b-at-checkpoint.md"),
        format!("---\nurl: {url}\ntitle: Boundary\nfetched_utc: {old}\nfetched_utc: \"2026-10-31T20:00:00-04:00\"\n---\nBoundary body"),
    )
    .unwrap();
    // The scanner accepts this for triage, but it is an archive non-article.
    fs::write(
        root.join("title-missing.md"),
        format!("---\nurl: https://example.com/no-title\nfetched_utc: {old}\n---\nBody"),
    )
    .unwrap();
    write(
        &linked,
        "old.md",
        "https://example.com/linked",
        old,
        "linked ",
    );

    let urls = [url.to_string()];
    let mut index = CorpusScanIndex::default();
    index
        .load_url_delta(
            root,
            20_000,
            &PromptRegistry::with_defaults(),
            &urls,
            None,
            &[],
            |_| {},
        )
        .unwrap();
    let hints = index.archive_read_hints();
    let since = cutoff.parse().unwrap();
    let export = |hints| {
        let summary = build_triage_archive_with(
            root,
            harvester_engine::ArchiveTarget::InOutput {
                basename: "archive.md",
            },
            &urls,
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
        let bytes = fs::read_to_string(summary.output_path).unwrap();
        (summary.files_read, bytes)
    };
    let (full_count, full) = export(None);
    let (hinted_count, hinted) = export(Some(&hints));
    assert_eq!((full_count, hinted_count), (4, 3));
    assert!(full.contains("Boundary body"));
    assert_eq!(hinted, full);
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
        .load_url_delta(
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
        .load_url_delta(
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
        .load_url_delta(dir.path(), 20_000, &registry, &urls, None, &[], |_| {})
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
        .load_url_delta(
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
        .load_url_delta(
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
        .load_url_delta(
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
        .load_url_delta(
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
        .load_url_delta(dir.path(), 20_000, &registry, &urls, None, &[], |_| {})
        .unwrap();
    let since = Some("2026-09-20T00:00:00Z".parse().unwrap());
    let (filtered, stats) = index
        .load_url_delta(
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
        .load_url_delta(dir.path(), 4_000, &registry, &urls, None, &[], |_| {})
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
