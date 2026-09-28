#[path = "../examples/replay_support/mod.rs"]
mod replay_support;

use harvester_engine::{build_markdown_document, WhitespaceTokenCounter};
use harvester_io::{
    load_brave_seen_set, load_completed_jobs, load_desktop_window_size, load_seen_set,
    load_signal_candidate_cache, load_summary_cache, load_triage_cache, load_window_size,
    persist_brave_seen_set, persist_seen_set,
};
use replay_support::{run_benchmark, BenchmarkHost, HarnessOptions};
use std::fs;

#[test]
fn replay_harness_completes_five_articles_and_saves_results() {
    let temp = tempfile::tempdir().expect("temporary replay directory");
    let source = temp.path().join("source");
    let work = temp.path().join("work");
    fs::create_dir_all(&source).expect("create synthetic source folder");
    fs::write(source.join(".sources.ron"), "(sources: [])")
        .expect("write synthetic sources registry");
    for index in 0..5 {
        let url = format!("https://replay.test/article-{index}");
        let detail = std::iter::repeat_n(
            "The company announced a dated business development affecting model pricing, enterprise adoption, infrastructure investment, and customer demand.",
            24,
        )
        .collect::<Vec<_>>()
        .join(" ");
        let body =
            format!("OpenAI announced a dated business development for article {index}. {detail}");
        let fetched = format!("2026-09-2{}T0{}:00:00Z", index + 1, index);
        let (_, markdown) = build_markdown_document(
            &url,
            Some(&format!("Replay article {index}")),
            "utf-8",
            &fetched,
            &body,
            &WhitespaceTokenCounter,
        );
        fs::write(source.join(format!("article-{index}.md")), markdown)
            .expect("write synthetic article");
    }
    let mut brave_seen = harvester_engine::BraveSeenSet::new();
    let mut rss_seen = harvester_engine::RssSeenSet::new();
    for index in 0..5 {
        let url = format!("https://replay.test/article-{index}");
        brave_seen.mark_seen(&url);
        rss_seen.mark_seen("bench-rss-24", &url);
    }
    rss_seen.mark_seen("bench-rss-24", "https://replay.test/active-seen-only");
    for index in 0..8 {
        rss_seen.mark_seen(
            &format!("retired-rss-{index}"),
            &format!("https://replay.test/retired-seen-{index}"),
        );
    }
    persist_brave_seen_set(&brave_seen, &source.join(".brave_seen_set.ron"))
        .expect("write synthetic Brave seen set");
    persist_seen_set(&rss_seen, &source.join(".seen_set.ron"))
        .expect("write synthetic RSS seen set");

    let unmarked = temp.path().join("unmarked");
    fs::create_dir_all(&unmarked).expect("create unmarked folder");
    assert!(run_benchmark(HarnessOptions {
        source_dir: source.clone(),
        work_dir: unmarked,
        hold_back: 5,
        reuse_copy: true,
        host: BenchmarkHost::Batch,
        llm_latency_ms: 0,
    })
    .expect_err("unmarked folder must be rejected")
    .contains("benchmark copy marker"));

    let report = run_benchmark(HarnessOptions {
        source_dir: source.clone(),
        work_dir: work.clone(),
        hold_back: 5,
        reuse_copy: false,
        host: BenchmarkHost::Batch,
        llm_latency_ms: 0,
    })
    .expect("five-article replay should complete");

    assert!(report.completed);
    assert_eq!(report.skipped_persisted_rss.sources, 4);
    assert_eq!(report.skipped_persisted_rss.entries, 4);
    let saved_report: serde_json::Value =
        serde_json::from_slice(&fs::read(&report.report_path).expect("read benchmark report"))
            .expect("parse benchmark report");
    assert_eq!(saved_report["skipped_persisted_rss"]["sources"], 4);
    assert_eq!(saved_report["skipped_persisted_rss"]["entries"], 4);
    assert_eq!(report.held_back_articles, 5);
    assert_eq!(report.effects_by_kind.get("EnqueueUrl"), Some(&5));
    assert!(
        report
            .effects_by_kind
            .get("RequestLlmCompletion")
            .copied()
            .unwrap_or(0)
            >= 5
    );
    for filename in [
        ".triage_cache.ron",
        ".summary_cache.ron",
        ".signal_candidate_cache.ron",
    ] {
        let path = work.join(filename);
        assert!(
            path.is_file(),
            "expected saved result store {}",
            path.display()
        );
        assert!(fs::metadata(path).unwrap().len() > 0);
    }
    let triage = load_triage_cache(&work.join(".triage_cache.ron"));
    let priorities = triage
        .iter()
        .map(|(_, entry)| entry.result.priority)
        .collect::<std::collections::BTreeSet<_>>();
    assert_eq!(priorities, (1..=5).collect());
    assert!(report.report_path.is_file());

    let desktop_work = temp.path().join("desktop-work");
    let desktop = run_benchmark(HarnessOptions {
        source_dir: source,
        work_dir: desktop_work.clone(),
        hold_back: 5,
        reuse_copy: false,
        host: BenchmarkHost::Desktop,
        llm_latency_ms: 300,
    })
    .expect("desktop replay should complete with real ticks");
    assert!(desktop.completed);
    assert_eq!(desktop.skipped_persisted_rss.sources, 4);
    assert_eq!(desktop.skipped_persisted_rss.entries, 4);
    assert!(desktop.desktop_driver_iterations > 0);
    assert_eq!(
        desktop.desktop_driver_iterations,
        desktop.desktop_iteration_wall_time.count
    );
    assert!(desktop.desktop_phase_totals_ms["effect_hand_off"] > 0.0);
    assert!(desktop
        .harness_synchronous_effect_handling
        .contains_key("canned_poll"));
    assert!(desktop
        .harness_synchronous_effect_handling
        .contains_key("held_article_delivery"));
    let reconciliation = desktop
        .desktop_reconciliation
        .as_ref()
        .expect("desktop reconciliation");
    assert!(reconciliation.accounted_percent > 95.0);
    assert!(reconciliation.accounted_percent <= 100.0);
    assert!(desktop.view_builds.count > 0);
    assert!(desktop.snapshots_emitted > 0);
    assert!(desktop.reducer_time_by_message_kind.contains_key("Tick"));
    assert!(desktop_work.join(".triage_cache.ron").is_file());
    assert!(desktop
        .private_file_writes
        .contains_key("llm_results/*.json"));
}

#[test]
fn carry_over_replay_preserves_restored_state_seen_sets_and_paid_results() {
    let temp = tempfile::tempdir().expect("temporary carry-over directory");
    let fixture = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("fixtures")
        .join("carry_over");
    let work = temp.path().join("carry-over-copy");
    let original_cache_bytes = [
        ".triage_cache.ron",
        ".summary_cache.ron",
        ".signal_candidate_cache.ron",
    ]
    .into_iter()
    .map(|name| {
        (
            name,
            fs::read(fixture.join(name)).expect("read fixture cache"),
        )
    })
    .collect::<std::collections::HashMap<_, _>>();

    let report = run_benchmark(HarnessOptions {
        source_dir: fixture,
        work_dir: work.clone(),
        hold_back: 0,
        reuse_copy: false,
        host: BenchmarkHost::Batch,
        llm_latency_ms: 0,
    })
    .expect("one carry-over replay cycle should complete");

    // 1. Restored and already-seen jobs are not enqueued again.
    assert_eq!(
        report
            .effects_by_kind
            .get("EnqueueUrl")
            .copied()
            .unwrap_or(0),
        0
    );

    // 2. Current-key completed work avoids every paid model call.
    assert!(report.llm_calls_by_prompt.is_empty());

    // 3. All three paid-result stores remain loadable with their fixture entries.
    let triage = load_triage_cache(&work.join(".triage_cache.ron"));
    let summary = load_summary_cache(&work.join(".summary_cache.ron"));
    let signal = load_signal_candidate_cache(&work.join(".signal_candidate_cache.ron"))
        .expect("load signal-candidate results");
    assert_eq!(triage.len(), 3, "two current and one stale triage result");
    assert_eq!(summary.len(), 3, "two current and one stale summary result");
    assert_eq!(signal.len(), 2, "one current and one stale signal result");
    assert!(triage
        .iter()
        .any(|(_, entry)| entry.result.category == "technology"));
    assert!(summary
        .iter()
        .any(|(_, entry)| entry.result.summary.contains("canned summary")));
    assert!(signal
        .entries
        .values()
        .any(|entry| entry.result.signal_key == "synthetic-current-event"));

    // 4. Cache-hit saves may rewrite RON, but sorted serialization preserves bytes.
    for (name, original) in original_cache_bytes {
        assert_eq!(
            fs::read(work.join(name)).expect("read replay cache"),
            original
        );
    }

    // 5. Rewritten RSS and Brave sets retain all pre-existing entries.
    let rss = load_seen_set(&work.join(".seen_set.ron"));
    assert!(rss.is_seen("carry-over-rss", "https://carry-over.synthetic/article-0"));
    assert!(rss.is_seen("carry-over-rss", "https://carry-over.synthetic/rss-only"));
    let brave = load_brave_seen_set(&work.join(".brave_seen_set.ron"));
    assert!(brave.is_seen("https://carry-over.synthetic/article-0"));
    assert!(brave.is_seen("https://carry-over.synthetic/article-1"));
    assert!(brave.is_seen("https://carry-over.synthetic/brave-only"));

    // 6. Every restored job still has its URL and recovered fetch time.
    assert!(report
        .private_file_writes
        .contains_key(".harvester_state.ron"));
    let jobs = load_completed_jobs(&work.join(".harvester_state.ron"));
    assert_eq!(jobs.len(), 2);
    for index in 0..2 {
        let url = format!("https://carry-over.synthetic/article-{index}");
        let job = jobs
            .iter()
            .find(|job| job.url == url)
            .expect("restored job");
        assert_eq!(
            chrono::DateTime::parse_from_rfc3339(job.fetched_utc.as_deref().expect("fetch time"))
                .expect("valid fetch time")
                .timestamp(),
            chrono::DateTime::parse_from_rfc3339("2026-09-20T09:15:00Z")
                .unwrap()
                .timestamp()
        );
        assert_eq!(job.links.len(), 1);
        assert_eq!(
            job.links[0].downloaded_path.as_deref(),
            Some("linked/carry-over-resource.md")
        );
    }
    assert_eq!(
        load_window_size(&work.join(".harvester_state.ron")),
        Some((1280, 800))
    );
    assert_eq!(
        load_desktop_window_size(&work.join(".harvester_state.ron")),
        Some((1512, 982))
    );
}
