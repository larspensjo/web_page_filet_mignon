use std::time::Instant;

use harvester_core::{
    update, AppState, ArticleSummaryResult, CompletedJobSnapshot, Effect, LinkSnapshotRecord,
    LoadedArticle, Msg, SummaryCache, SummaryCacheEntry, SummaryCacheKey, MAX_EXTRACTED_LINKS,
};
use harvester_engine::llm::{prompt::PromptId, SummaryEntities};
use harvester_ui_bridge::{
    probe::{PROBE_CORPUS_JOBS, PROBE_TYPICAL_LIST_ROWS},
    project, HOST_DRAIN_BUDGET_MS,
};

/// Synthetic representative distribution; output/ is not sampled because the checked-in corpus
/// is not a stable test fixture. Buckets are `(jobs_per_20, min_links, max_links)`.
const DRAIN_COST_LINK_DISTRIBUTION: [(usize, usize, usize); 3] =
    [(11, 0, 0), (7, 1, 5), (2, 20, 40)];
const DRAIN_COST_ITERATIONS: usize = 5;
const SUMMARY_TITLE_ROWS: usize = 110;

#[test]
fn view_build_compare_and_project_stay_within_the_drain_budget() {
    let (state, summary_cache_entries) = drain_cost_state();
    let previous = state.desktop_view();
    let shape = assert_representative_shape(&previous, None, summary_cache_entries);
    let measured = median_drain_cost(&state, &previous);
    let measured_ms = measured.total_ms;
    println!(
        "host drain cost: measured_ms={measured_ms} view_us={} compare_us={} project_us={} total_links={} job_count={} summary_cache_entries={} scoped_count={} rows={} titled_rows={}",
        measured.view_us,
        measured.compare_us,
        measured.project_us,
        total_links(&state),
        shape.job_count,
        shape.summary_cache_entries,
        shape.scoped_count,
        shape.rows,
        shape.titled_rows,
    );
    assert!(
        measured_ms <= HOST_DRAIN_BUDGET_MS,
        "view build + comparison + projection took {measured_ms} ms; budget is {HOST_DRAIN_BUDGET_MS} ms"
    );
}

#[test]
fn search_driven_view_rebuild_stays_within_the_drain_budget() {
    let (state, summary_cache_entries) = drain_cost_state();
    let previous = state.desktop_view();
    let (state, _) = update(state, Msg::JobsSearchQueryChanged("needle".into()));
    let searched = state.desktop_view();
    let shape = assert_representative_shape(&searched, Some("needle"), summary_cache_entries);
    let measured = median_drain_cost(&state, &previous);
    let measured_ms = measured.total_ms;
    println!(
        "search-driven host drain cost: measured_ms={measured_ms} view_us={} compare_us={} project_us={} total_links={} job_count={} summary_cache_entries={} scoped_count={} matched_rows={} titled_rows={} title_only_matches={}",
        measured.view_us,
        measured.compare_us,
        measured.project_us,
        total_links(&state),
        shape.job_count,
        shape.summary_cache_entries,
        shape.scoped_count,
        shape.rows,
        shape.titled_rows,
        shape.title_only_matches,
    );
    assert!(
        measured_ms <= HOST_DRAIN_BUDGET_MS,
        "search-driven view build + comparison + projection took {measured_ms} ms; budget is {HOST_DRAIN_BUDGET_MS} ms"
    );
}

fn drain_cost_state() -> (AppState, usize) {
    let snapshots = (0..PROBE_CORPUS_JOBS)
        .map(|index| {
            let recent = index >= PROBE_CORPUS_JOBS - SUMMARY_TITLE_ROWS;
            CompletedJobSnapshot {
                url: job_url(index),
                tokens: Some(1_000 + (index % 900) as u32),
                bytes: Some(32_768 + index as u64),
                links: link_snapshots(index),
                fetched_utc: Some(if recent {
                    format!("2026-09-05T12:{:02}:00Z", index % 60)
                } else {
                    "2026-09-04T12:00:00Z".into()
                }),
            }
        })
        .collect();
    let (state, _) = update(AppState::new(), Msg::RestoreCompletedJobs(snapshots));
    let (state, _) = update(
        state,
        Msg::BriefingCheckpointSet(Some("2026-09-05T12:00:00Z".into())),
    );
    seed_summary_titles_through_cache(state)
}

fn link_snapshots(index: usize) -> Vec<LinkSnapshotRecord> {
    let position = index % 20;
    debug_assert_eq!(
        DRAIN_COST_LINK_DISTRIBUTION
            .iter()
            .map(|(jobs, _, _)| jobs)
            .sum::<usize>(),
        20
    );
    let mut bucket_start = 0;
    let (min, max) = DRAIN_COST_LINK_DISTRIBUTION
        .iter()
        .find_map(|(jobs, min, max)| {
            let bucket_end = bucket_start + jobs;
            let found = (position < bucket_end).then_some((*min, *max));
            bucket_start = bucket_end;
            found
        })
        .expect("link distribution covers all twenty positions");
    let count = min + (index % (max - min + 1));
    (0..count.min(MAX_EXTRACTED_LINKS))
        .map(|link_index| LinkSnapshotRecord {
            url: format!("https://links.drain-cost.invalid/{index}/{link_index}"),
            downloaded_path: None,
        })
        .collect()
}

fn seed_summary_titles_through_cache(mut state: AppState) -> (AppState, usize) {
    let summary_urls = (PROBE_CORPUS_JOBS - SUMMARY_TITLE_ROWS..PROBE_CORPUS_JOBS)
        .map(job_url)
        .collect::<Vec<_>>();
    let (next_state, _) = update(
        state,
        Msg::EvaluatePreTriageRefresh {
            ordered_urls: summary_urls.clone(),
            triggered_by_job_done: false,
        },
    );
    state = next_state;
    let request_id = (0..10)
        .find_map(|tick| {
            let (next_state, effects) = update(
                std::mem::take(&mut state),
                Msg::Tick {
                    now: chrono::DateTime::from_timestamp(1_778_000_000 + tick, 0)
                        .expect("valid fixture timestamp"),
                },
            );
            state = next_state;
            effects.into_iter().find_map(|effect| match effect {
                Effect::LoadArticlesForTriage { request_id, .. } => Some(request_id),
                _ => None,
            })
        })
        .expect("pre-triage article load should be dispatched");
    let articles = (0..SUMMARY_TITLE_ROWS)
        .map(|offset| {
            let index = PROBE_CORPUS_JOBS - SUMMARY_TITLE_ROWS + offset;
            LoadedArticle {
                url: job_url(index),
                source_title: Some(format!("Representative summary source {index}")),
                prepared_text: format!("Representative article body {index}"),
                content_hash: format!("drain-cost-content-{index}"),
                fetched_utc: Some(format!("2026-09-05T12:{:02}:00Z", index % 60)),
            }
        })
        .collect::<Vec<_>>();
    let (next_state, _) = update(
        state,
        Msg::TriageArticlesLoaded {
            request_id,
            articles: articles.clone(),
        },
    );
    state = next_state;
    let mut cache = SummaryCache::new();
    for index in 0..PROBE_CORPUS_JOBS {
        let key = SummaryCacheKey::try_new(
            &format!("drain-cost-content-{index}"),
            PromptId::ArticleSummary,
            Some(1),
            Some("drain-cost-model"),
            &[],
        )
        .expect("complete summary cache key");
        cache.insert(
            key,
            SummaryCacheEntry {
                result: ArticleSummaryResult {
                    title: format!(
                        "Representative {}summary title {}",
                        if index.is_multiple_of(10) {
                            "needle "
                        } else {
                            ""
                        },
                        job_url(index)
                    ),
                    summary: "Representative summary".into(),
                    key_points: vec!["Representative point".into()],
                    input_tokens: 128,
                    output_tokens: 64,
                    entities: SummaryEntities::default(),
                },
                created_at_utc: "2026-09-05T12:59:00Z".into(),
            },
        );
    }
    let summary_cache_entries = cache.len();
    (
        update(state, Msg::SummaryCacheHydrated { cache }).0,
        summary_cache_entries,
    )
}

fn job_url(index: usize) -> String {
    format!(
        "https://drain-cost.invalid/articles/{index}{}",
        if index.is_multiple_of(100) {
            "-needle"
        } else {
            ""
        }
    )
}

#[derive(Debug)]
struct FixtureShape {
    job_count: usize,
    summary_cache_entries: usize,
    scoped_count: usize,
    rows: usize,
    titled_rows: usize,
    title_only_matches: usize,
}

fn assert_representative_shape(
    view: &harvester_core::AppViewModel,
    query: Option<&str>,
    summary_cache_entries: usize,
) -> FixtureShape {
    let rows = &view.desktop_job_list.rows;
    let titled_rows = rows
        .iter()
        .filter(|row| row.summary_title.is_some())
        .count();
    let title_only_matches = query.map_or(0, |query| {
        rows.iter()
            .filter(|row| {
                !row.url.contains(query)
                    && row
                        .summary_title
                        .as_deref()
                        .is_some_and(|title| title.to_ascii_lowercase().contains(query))
            })
            .count()
    });
    assert_eq!(view.job_count, PROBE_CORPUS_JOBS);
    assert_eq!(summary_cache_entries, PROBE_CORPUS_JOBS);
    if query.is_none() {
        assert_eq!(view.desktop_job_list.scoped_count, PROBE_TYPICAL_LIST_ROWS);
        assert_eq!(rows.len(), PROBE_TYPICAL_LIST_ROWS);
        assert!(titled_rows >= PROBE_TYPICAL_LIST_ROWS * 9 / 10);
    } else {
        assert_eq!(view.desktop_job_list.scoped_count, rows.len());
        assert!((5..=25).contains(&rows.len()));
        assert!(titled_rows >= 5);
        assert!(title_only_matches > 0);
    }
    FixtureShape {
        job_count: view.job_count,
        summary_cache_entries,
        scoped_count: view.desktop_job_list.scoped_count,
        rows: rows.len(),
        titled_rows,
        title_only_matches,
    }
}

#[derive(Debug)]
struct DrainCostMeasurement {
    total_ms: u64,
    view_us: u128,
    compare_us: u128,
    project_us: u128,
}

fn median_drain_cost(
    state: &AppState,
    previous: &harvester_core::AppViewModel,
) -> DrainCostMeasurement {
    let measurements = (0..DRAIN_COST_ITERATIONS)
        .map(|_| {
            let started = Instant::now();
            let view_started = Instant::now();
            let view = state.desktop_view();
            let view_us = view_started.elapsed().as_micros();
            let compare_started = Instant::now();
            let _changed = view != *previous;
            let compare_us = compare_started.elapsed().as_micros();
            let project_started = Instant::now();
            let _snapshot = project(&view);
            let project_us = project_started.elapsed().as_micros();
            DrainCostMeasurement {
                total_ms: started.elapsed().as_millis() as u64,
                view_us,
                compare_us,
                project_us,
            }
        })
        .collect::<Vec<_>>();
    DrainCostMeasurement {
        total_ms: median(measurements.iter().map(|sample| sample.total_ms)),
        view_us: median(measurements.iter().map(|sample| sample.view_us)),
        compare_us: median(measurements.iter().map(|sample| sample.compare_us)),
        project_us: median(measurements.iter().map(|sample| sample.project_us)),
    }
}

fn median<T: Ord + Copy>(samples: impl Iterator<Item = T>) -> T {
    let mut samples = samples.collect::<Vec<_>>();
    samples.sort_unstable();
    samples[samples.len() / 2]
}

fn total_links(state: &AppState) -> usize {
    state
        .completed_jobs_snapshot()
        .into_iter()
        .map(|job| job.links.len())
        .sum()
}
