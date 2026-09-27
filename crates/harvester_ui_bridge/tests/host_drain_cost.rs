use std::collections::HashMap;
use std::sync::{Mutex, MutexGuard};
use std::time::Instant;

use harvester_core::{
    update, AppState, ArticleSummaryResult, ArticleTriageResult, CompletedJobSnapshot, Effect,
    JobResultKind, LinkSnapshotRecord, LlmResultKind, LoadedArticle, Msg, Stage, SummaryCache,
    SummaryCacheEntry, SummaryCacheKey, TriageCache, TriageCacheKey, TriagePhase, TriageSession,
    UnfinishedWork, ACTIVITY_FEED_CAPACITY, MAX_EXTRACTED_LINKS,
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
const PRODUCTION_SCALE_WINDOW_ARTICLES: usize = 10_450;
static HOST_DRAIN_TEST_MUTEX: Mutex<()> = Mutex::new(());

fn serialize_timing_tests() -> MutexGuard<'static, ()> {
    HOST_DRAIN_TEST_MUTEX
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

#[test]
fn view_build_compare_and_project_stay_within_the_drain_budget() {
    let _guard = serialize_timing_tests();
    let (state, summary_cache_entries) = drain_cost_state();
    let previous = state.view();
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
    let _guard = serialize_timing_tests();
    let (state, summary_cache_entries) = drain_cost_state();
    let previous = state.view();
    let (state, _) = update(state, Msg::JobsSearchQueryChanged("needle".into()));
    let searched = state.view();
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

#[test]
fn production_scale_included_window_view_cost() {
    let _guard = serialize_timing_tests();
    let state = load_production_scale_window();
    assert_eq!(
        state.batch_observation().jobs_done,
        PRODUCTION_SCALE_WINDOW_ARTICLES
    );
    let unfinished = match state.unfinished_work() {
        UnfinishedWork::Known(summary) => summary,
        UnfinishedWork::Unknown => panic!("article metadata should make completeness known"),
    };
    assert_eq!(
        unfinished.window_articles(),
        PRODUCTION_SCALE_WINDOW_ARTICLES
    );
    assert_eq!(unfinished.not_eligible, 6_666);
    assert_eq!(unfinished.needs_triage, 450);
    assert_eq!(unfinished.needs_scoring, 3_334);

    let previous = state.view();
    let measured = median_drain_cost(&state, &previous);
    println!("production-scale included view cost: measured_ms={} view_us={} compare_us={} project_us={}", measured.total_ms, measured.view_us, measured.compare_us, measured.project_us);
    assert!(
        measured.total_ms <= HOST_DRAIN_BUDGET_MS,
        "production-scale view build + comparison + projection took {} ms; budget is {HOST_DRAIN_BUDGET_MS} ms",
        measured.total_ms
    );
}

#[test]
fn production_scale_loaded_triaging_view_cost() {
    let _guard = serialize_timing_tests();
    let state = load_production_scale_triage_session();
    let observation = state.batch_observation();
    assert!(matches!(observation.triage_phase, TriagePhase::Triaging));
    assert_eq!(observation.triage_total, PRODUCTION_SCALE_WINDOW_ARTICLES);
    assert_eq!(observation.jobs_done, PRODUCTION_SCALE_WINDOW_ARTICLES);
    let previous = state.view();
    let measured = median_drain_cost(&state, &previous);
    println!("production-scale loaded triaging view cost: measured_ms={} view_us={} compare_us={} project_us={}", measured.total_ms, measured.view_us, measured.compare_us, measured.project_us);
    assert!(measured.total_ms <= HOST_DRAIN_BUDGET_MS);
}

#[test]
fn production_scale_loaded_complete_view_cost() {
    let _guard = serialize_timing_tests();
    let state = load_production_scale_complete_session();
    let observation = state.batch_observation();
    assert!(
        matches!(observation.triage_phase, TriagePhase::Complete),
        "{observation:?}"
    );
    assert_eq!(observation.triage_total, PRODUCTION_SCALE_WINDOW_ARTICLES);
    assert_eq!(observation.triage_completed, 10_000);
    assert_eq!(observation.triage_failed, 450);
    assert_eq!(observation.jobs_done, PRODUCTION_SCALE_WINDOW_ARTICLES);
    let previous = state.view();
    assert_eq!(previous.archive_filtered_count, 3_334);
    let measured = median_drain_cost(&state, &previous);
    println!("production-scale loaded complete view cost: measured_ms={} view_us={} compare_us={} project_us={}", measured.total_ms, measured.view_us, measured.compare_us, measured.project_us);
    assert!(measured.total_ms <= HOST_DRAIN_BUDGET_MS);
}

#[test]
fn production_scale_article_completion_reducer_stays_within_budget() {
    let _guard = serialize_timing_tests();
    let mut state = load_production_scale_window();
    let (next, effects) = update(
        state,
        Msg::PipelineRunRequested {
            scope: harvester_core::PipelineRunScope::Resume,
        },
    );
    state = next;
    let configuration_request = effects
        .iter()
        .find_map(|effect| match effect {
            Effect::LoadProcessingConfiguration { request_id, .. } => Some(*request_id),
            _ => None,
        })
        .expect("triage configuration request");
    let (versions, models) = production_metadata();
    let (state, effects) = update(
        state,
        Msg::ProcessingConfigurationLoaded {
            request_id: configuration_request,
            contexts: production_contexts(),
            active_versions: versions,
            effective_models: models,
            preparation_budget: 100_000,
        },
    );
    let (state, load_effects) = respond_to_pipeline_window(state, effects);
    let (state, advance_effects) = update(state, Msg::PipelineRunAdvance);
    let effects = load_effects
        .into_iter()
        .chain(advance_effects)
        .collect::<Vec<_>>();
    let request_id = effects
        .iter()
        .find_map(|effect| match effect {
            Effect::RequestLlmCompletion {
                request_id,
                prompt_id: PromptId::ArticleSignalCandidate,
                ..
            } => Some(*request_id),
            _ => None,
        })
        .expect("one article scoring request");
    let samples = (0..DRAIN_COST_ITERATIONS).map(|_| {
        let copy = state.clone();
        let started = Instant::now();
        let (after, _) = update(
            copy,
            Msg::LlmCompleted {
                request_id,
                result: LlmResultKind::Failed {
                    reason: "cost probe".into(),
                },
                metadata: None,
            },
        );
        assert_eq!(
            known_window_articles(&after),
            PRODUCTION_SCALE_WINDOW_ARTICLES
        );
        started.elapsed().as_micros()
    });
    let hot_us = median(samples);
    let (mut versions, models) = production_metadata();
    versions.insert(PromptId::ArticleTriage, 2);
    let started = Instant::now();
    let (after, _) = update(
        state,
        Msg::LlmMetadataLoaded {
            active_versions: versions,
            effective_models: models,
        },
    );
    let full_pass_us = started.elapsed().as_micros();
    assert_eq!(
        known_window_articles(&after),
        PRODUCTION_SCALE_WINDOW_ARTICLES
    );
    println!("production-scale reducer cost: article_completion_us={hot_us} full_pass_prompt_version_us={full_pass_us} window_articles={PRODUCTION_SCALE_WINDOW_ARTICLES}");
    assert!(
        hot_us <= u128::from(HOST_DRAIN_BUDGET_MS) * 1_000,
        "article completion reducer took {hot_us} us; budget is {HOST_DRAIN_BUDGET_MS} ms"
    );
}

fn known_window_articles(state: &AppState) -> usize {
    match state.unfinished_work() {
        UnfinishedWork::Known(summary) => summary.window_articles(),
        UnfinishedWork::Unknown => panic!("production metadata should be loaded"),
    }
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
    let state = seed_populated_run_progress();
    let (state, _) = update(state, Msg::RestoreCompletedJobs(snapshots));
    // Restoration resets source state; keep a real source poll open for the active-run fixture.
    let (state, _) = update(state, Msg::PollSourcesClicked);
    let (state, _) = update(state, Msg::PollStarted { total: 1 });
    let (state, _) = update(
        state,
        Msg::BriefingCheckpointSet(Some("2026-09-05T12:00:00Z".into())),
    );
    seed_summary_titles_through_cache(state)
}

fn seed_populated_run_progress() -> AppState {
    const ACTIVITY_JOBS: usize = ACTIVITY_FEED_CAPACITY / 2;
    let (state, _) = update(AppState::new(), Msg::PollSourcesClicked);
    let (state, _) = update(state, Msg::PollStarted { total: 2 });
    let urls = (0..ACTIVITY_JOBS).map(job_url).collect::<Vec<_>>();
    let (mut state, effects) = update(
        state,
        Msg::SourcePollCompleted {
            source_id: harvester_engine::SourceId::new("drain-progress").expect("valid source id"),
            urls,
            kind: harvester_engine::SourceKind::Rss,
            parsed: ACTIVITY_JOBS,
            dedup_filtered: 0,
        },
    );
    let job_ids = effects
        .iter()
        .filter_map(|effect| match effect {
            Effect::EnqueueUrl { job_id, .. } => Some(*job_id),
            _ => None,
        })
        .collect::<Vec<_>>();
    assert_eq!(job_ids.len(), ACTIVITY_JOBS);
    // Keep intake open so the measured populated run has genuine pending work.
    for job_id in job_ids {
        state = update(
            state,
            Msg::JobProgress {
                job_id,
                stage: Stage::Downloading,
                tokens: None,
                bytes: Some(32_768),
                content_preview: None,
            },
        )
        .0;
        state = update(
            state,
            Msg::JobDone {
                job_id,
                result: JobResultKind::Success,
                content_preview: None,
                extracted_links: Vec::new(),
                fetched_utc: Some("2026-09-05T12:00:00Z".into()),
            },
        )
        .0;
    }
    assert_eq!(
        state.view().run_progress.activity.len(),
        ACTIVITY_FEED_CAPACITY
    );
    state
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
    let (active_versions, effective_models) = production_metadata();
    state = update(
        state,
        Msg::LlmMetadataLoaded {
            active_versions,
            effective_models,
        },
    )
    .0;
    let (next, effects) = update(
        state,
        Msg::PipelineRunRequested {
            scope: harvester_core::PipelineRunScope::Resume,
        },
    );
    state = harvester_core::fixture_support::complete_processing_start(next, effects, 100_000).0;
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
    let request_id = (0..20)
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
            delta: harvester_engine::TriageArticleDelta::full_window(articles.clone(), 100_000),
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

fn load_production_scale_window() -> AppState {
    let mut state = AppState::new();
    let (active_versions, effective_models) = production_metadata();
    let contexts = production_contexts();
    state = update(
        state,
        Msg::LlmMetadataLoaded {
            active_versions,
            effective_models,
        },
    )
    .0;
    state = update(
        state,
        Msg::PromptContextsLoaded {
            contexts: contexts.clone(),
        },
    )
    .0;
    let mut triage_cache = TriageCache::new();
    let mut summary_cache = SummaryCache::new();
    for index in 0usize..10_000 {
        let content_hash = format!("drain-cost-content-{index}");
        let triage_key = TriageCacheKey::try_new(
            &content_hash,
            PromptId::ArticleTriage,
            Some(1),
            Some("drain-cost-triage-model"),
            contexts.get(&PromptId::ArticleTriage).unwrap(),
        )
        .unwrap();
        triage_cache.insert(
            triage_key,
            ArticleTriageResult {
                category: "news".into(),
                priority: if index.is_multiple_of(3) { 3 } else { 1 },
                tags: vec!["topic".into()],
                rationale: "representative triage".into(),
                input_tokens: 100,
                output_tokens: 30,
            },
        );
        if index.is_multiple_of(3) {
            let key = SummaryCacheKey::try_new(
                &content_hash,
                PromptId::ArticleSummary,
                Some(1),
                Some("drain-cost-model"),
                contexts.get(&PromptId::ArticleSummary).unwrap(),
            )
            .unwrap();
            summary_cache.insert(
                key,
                SummaryCacheEntry {
                    result: ArticleSummaryResult {
                        title: format!("Production summary {index}"),
                        summary: "Representative summary text".repeat(8),
                        key_points: vec!["Representative point".into()],
                        input_tokens: 200,
                        output_tokens: 80,
                        entities: SummaryEntities::default(),
                    },
                    created_at_utc: "2026-09-06T12:00:00Z".into(),
                },
            );
        }
    }
    state = update(
        state,
        Msg::TriageCacheHydrated {
            cache: triage_cache,
        },
    )
    .0;
    state = update(
        state,
        Msg::SummaryCacheHydrated {
            cache: summary_cache,
        },
    )
    .0;
    state = update(
        state,
        Msg::SignalCandidateCacheLoaded {
            cache: Default::default(),
        },
    )
    .0;

    let completed_jobs = (0..PRODUCTION_SCALE_WINDOW_ARTICLES)
        .map(|index| CompletedJobSnapshot {
            url: production_window_url(index),
            tokens: Some(1_000 + (index % 900) as u32),
            bytes: Some(32_768 + index as u64),
            links: Vec::new(),
            fetched_utc: Some("2026-09-06T12:00:00Z".into()),
        })
        .collect();
    state = update(state, Msg::RestoreCompletedJobs(completed_jobs)).0;

    let urls = (0..PRODUCTION_SCALE_WINDOW_ARTICLES)
        .map(production_window_url)
        .collect::<Vec<_>>();
    state = update(
        state,
        Msg::EvaluatePreTriageRefresh {
            ordered_urls: urls,
            triggered_by_job_done: false,
        },
    )
    .0;
    let request_id = (0..100)
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
        .expect("production-scale window load should be dispatched");
    let articles = (0..PRODUCTION_SCALE_WINDOW_ARTICLES)
        .map(production_loaded_article)
        .collect::<Vec<_>>();
    update(
        state,
        Msg::TriageArticlesLoaded {
            request_id,
            delta: harvester_engine::TriageArticleDelta::full_window(articles, 100_000),
        },
    )
    .0
}

fn load_production_scale_triage_session() -> AppState {
    let state = load_production_scale_window();
    let (state, effects) = update(
        state,
        Msg::PipelineRunRequested {
            scope: harvester_core::PipelineRunScope::Resume,
        },
    );
    let request_id = effects
        .iter()
        .find_map(|effect| match effect {
            Effect::LoadProcessingConfiguration { request_id, .. } => Some(*request_id),
            _ => None,
        })
        .expect("triage configuration request");
    let (active_versions, effective_models) = production_metadata();
    let (state, effects) = update(
        state,
        Msg::ProcessingConfigurationLoaded {
            request_id,
            contexts: production_contexts(),
            active_versions,
            effective_models,
            preparation_budget: 100_000,
        },
    );
    let (state, _) = respond_to_pipeline_window(state, effects);
    update(state, Msg::PipelineRunAdvance).0
}

fn respond_to_pipeline_window(state: AppState, effects: Vec<Effect>) -> (AppState, Vec<Effect>) {
    let Some(request_id) = effects.iter().find_map(|effect| match effect {
        Effect::LoadArticlesForTriage { request_id, .. } => Some(*request_id),
        _ => None,
    }) else {
        return (state, effects);
    };
    let articles = (0..PRODUCTION_SCALE_WINDOW_ARTICLES)
        .map(production_loaded_article)
        .collect();
    update(
        state,
        Msg::TriageArticlesLoaded {
            request_id,
            delta: harvester_engine::TriageArticleDelta::full_window(articles, 100_000),
        },
    )
}

fn load_production_scale_complete_session() -> AppState {
    let mut state = load_production_scale_window();
    let mut triage = TriageSession::new_loading(None);
    triage.set_articles(
        (0..PRODUCTION_SCALE_WINDOW_ARTICLES)
            .map(production_loaded_article)
            .collect(),
    );
    triage.transition_to_triaging();
    for index in 0..PRODUCTION_SCALE_WINDOW_ARTICLES {
        if index < 10_000 {
            triage.complete_article(
                index,
                ArticleTriageResult {
                    category: "news".into(),
                    priority: if index.is_multiple_of(3) { 3 } else { 1 },
                    tags: vec!["topic".into()],
                    rationale: "representative triage".into(),
                    input_tokens: 100,
                    output_tokens: 30,
                },
            );
        } else {
            triage.fail_article(index, "cache miss in completed session".into());
        }
    }
    triage.complete();
    state.set_complete_triage_for_host_drain_fixture(triage);
    state
}

fn production_loaded_article(index: usize) -> LoadedArticle {
    LoadedArticle {
        url: production_window_url(index),
        source_title: Some(format!("Production-scale source {index}")),
        prepared_text: "word ".repeat(600),
        content_hash: format!("drain-cost-content-{index}"),
        fetched_utc: Some(if index < PROBE_CORPUS_JOBS {
            "2026-09-06T12:00:00Z".into()
        } else {
            "2026-09-05T12:01:00Z".into()
        }),
    }
}

fn production_contexts() -> HashMap<PromptId, Vec<(String, String)>> {
    [
        PromptId::ArticleTriage,
        PromptId::ArticleSummary,
        PromptId::ArticleSignalCandidate,
    ]
    .into_iter()
    .map(|prompt_id| (prompt_id, vec![("policy".into(), "context ".repeat(375))]))
    .collect()
}

fn production_metadata() -> (HashMap<PromptId, u32>, HashMap<PromptId, String>) {
    let mut active_versions = HashMap::new();
    let mut effective_models = HashMap::new();
    for prompt_id in [
        PromptId::ArticleTriage,
        PromptId::ArticleSummary,
        PromptId::ArticleSignalCandidate,
        PromptId::BriefingExecutiveSummary,
        PromptId::BriefingNextItem,
    ] {
        active_versions.insert(prompt_id, 1);
    }
    effective_models.insert(PromptId::ArticleTriage, "drain-cost-triage-model".into());
    effective_models.insert(PromptId::ArticleSummary, "drain-cost-model".into());
    effective_models.insert(
        PromptId::ArticleSignalCandidate,
        "drain-cost-scoring-model".into(),
    );
    effective_models.insert(
        PromptId::BriefingExecutiveSummary,
        "drain-cost-briefing-model".into(),
    );
    effective_models.insert(
        PromptId::BriefingNextItem,
        "drain-cost-briefing-model".into(),
    );
    (active_versions, effective_models)
}

fn production_window_url(index: usize) -> String {
    job_url(index)
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
    assert_eq!(view.run_progress.stages.len(), 6);
    assert_eq!(view.run_progress.activity.len(), ACTIVITY_FEED_CAPACITY);
    assert!(view.run_progress.run_active);
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
            let view = state.view();
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
