use super::*;
use crate::briefing::{ArticleSummaryResult, LoadedArticle};
use crate::signal_candidate_cache::{SignalCandidateCache, SignalCandidateCacheEntry};
use crate::summary_cache::{SummaryCacheEntry, SummaryCacheKey};
use crate::triage::{ArticleTriageResult, ArticleTriageState};
use crate::{TriageCache, UnfinishedStageVerdict, UnfinishedWork};
use harvester_engine::llm::dto::{Confidence, SignalCandidateResult, SourceTier};
use harvester_engine::llm::prompt::PromptId;
use std::collections::HashMap;

const PREPARATION_BUDGET: usize = 100_000;

fn reduce(state: AppState, msg: Msg) -> (AppState, Vec<Effect>) {
    crate::update::update(state, msg)
}

fn configured_state(state: AppState, version: u32, model: &str, context: &str) -> AppState {
    let mut active_versions = HashMap::new();
    let mut effective_models = HashMap::new();
    for prompt_id in [
        PromptId::ArticleTriage,
        PromptId::ArticleSummary,
        PromptId::ArticleSignalCandidate,
    ] {
        active_versions.insert(prompt_id, version);
    }
    active_versions.insert(PromptId::BriefingExecutiveSummary, 1);
    active_versions.insert(PromptId::BriefingNextItem, 1);
    effective_models.insert(PromptId::ArticleTriage, format!("{model}-triage"));
    effective_models.insert(PromptId::ArticleSummary, format!("{model}-summary"));
    effective_models.insert(PromptId::ArticleSignalCandidate, format!("{model}-scoring"));
    effective_models.insert(
        PromptId::BriefingExecutiveSummary,
        "test-briefing-model".to_string(),
    );
    effective_models.insert(
        PromptId::BriefingNextItem,
        "test-briefing-model".to_string(),
    );

    let (state, _) = reduce(
        state,
        Msg::LlmMetadataLoaded {
            active_versions,
            effective_models,
        },
    );
    reduce(
        state,
        Msg::PromptContextsLoaded {
            contexts: contexts_for(context),
        },
    )
    .0
}

fn contexts_for(value: &str) -> HashMap<PromptId, Vec<(String, String)>> {
    let context = vec![("policy".to_string(), value.to_string())];
    [
        PromptId::ArticleTriage,
        PromptId::ArticleSummary,
        PromptId::ArticleSignalCandidate,
    ]
    .into_iter()
    .map(|prompt_id| (prompt_id, context.clone()))
    .collect()
}

fn article(url: &str, content_hash: &str) -> LoadedArticle {
    LoadedArticle {
        url: url.to_string(),
        source_title: Some("Completeness test article".to_string()),
        prepared_text: std::iter::repeat_n("article content", 220)
            .collect::<Vec<_>>()
            .join(" "),
        content_hash: content_hash.to_string(),
        fetched_utc: Some("2026-09-20T12:00:00Z".to_string()),
    }
}

fn load_window(mut state: AppState, articles: Vec<LoadedArticle>) -> AppState {
    for article in &articles {
        state = support::add_completed_job_for_test(state, &article.url);
    }
    let (state, request_id) = support::tick_until_dispatch(state);
    reduce(
        state,
        Msg::TriageArticlesLoaded {
            request_id,
            delta: harvester_engine::TriageArticleDelta::full_window(articles, PREPARATION_BUDGET),
        },
    )
    .0
}

fn triage_result(priority: u8) -> ArticleTriageResult {
    ArticleTriageResult {
        category: "news".to_string(),
        priority,
        tags: vec!["topic".to_string()],
        rationale: "current-key test result".to_string(),
        input_tokens: 0,
        output_tokens: 0,
    }
}

fn summary_result(title: &str) -> ArticleSummaryResult {
    ArticleSummaryResult {
        title: title.to_string(),
        summary: format!("Summary for {title}"),
        key_points: vec!["current point".to_string()],
        input_tokens: 0,
        output_tokens: 0,
        entities: Default::default(),
    }
}

fn scoring_result() -> SignalCandidateResult {
    SignalCandidateResult {
        signal_score: 80,
        signal_key: "current-signal".to_string(),
        themes: vec!["topic".to_string()],
        draft_gist: "gist".to_string(),
        source_tier: SourceTier::Tier1,
        confidence: Confidence::High,
        reasoning: "current-key test result".to_string(),
        input_tokens: 0,
        output_tokens: 0,
    }
}

fn current_summary_key(state: &AppState, article: &LoadedArticle) -> SummaryCacheKey {
    state
        .current_summary_cache_key(&article.content_hash)
        .expect("current summary key")
}

fn seed_current_summary_cache(
    state: AppState,
    articles: &[(&LoadedArticle, ArticleSummaryResult)],
) -> AppState {
    let mut cache = crate::SummaryCache::new();
    for (article, result) in articles {
        cache.insert(
            current_summary_key(&state, article),
            SummaryCacheEntry {
                result: result.clone(),
                created_at_utc: "2026-09-20T12:00:00Z".to_string(),
            },
        );
    }
    reduce(state, Msg::SummaryCacheHydrated { cache }).0
}

fn seed_all_current_caches(mut state: AppState, article: &LoadedArticle, priority: u8) -> AppState {
    let triage = triage_result(priority);
    let summary = summary_result("Complete article");
    let triage_key = state
        .current_triage_cache_key(&article.content_hash)
        .expect("current triage key");
    let summary_key = current_summary_key(&state, article);
    let signal_key = crate::update::signal_candidate::input_key_for_current_results(
        &state,
        article,
        &triage,
        &summary_key,
        &summary,
    )
    .expect("current scoring key");

    let mut triage_cache = TriageCache::new();
    triage_cache.insert(triage_key, triage);
    let mut summary_cache = crate::SummaryCache::new();
    summary_cache.insert(
        summary_key,
        SummaryCacheEntry {
            result: summary,
            created_at_utc: "2026-09-20T12:00:00Z".to_string(),
        },
    );
    let mut scoring_cache = SignalCandidateCache::default();
    scoring_cache.insert(
        signal_key,
        SignalCandidateCacheEntry {
            result: scoring_result(),
            created_at_utc: "2026-09-20T12:00:00Z".to_string(),
        },
    );

    state = reduce(
        state,
        Msg::TriageCacheHydrated {
            cache: triage_cache,
        },
    )
    .0;
    state = reduce(
        state,
        Msg::SummaryCacheHydrated {
            cache: summary_cache,
        },
    )
    .0;
    reduce(
        state,
        Msg::SignalCandidateCacheLoaded {
            cache: scoring_cache,
        },
    )
    .0
}

fn known(state: &AppState) -> &crate::UnfinishedWorkSummary {
    match state.unfinished_work() {
        UnfinishedWork::Known(summary) => summary,
        UnfinishedWork::Unknown => panic!("expected a known unfinished-work summary"),
    }
}

fn start_triage(state: AppState) -> (AppState, u64) {
    let (state, effects) = reduce(
        state,
        Msg::PipelineRunRequested {
            scope: crate::PipelineRunScope::Resume,
        },
    );
    let configuration_request = effects
        .iter()
        .find_map(|effect| match effect {
            Effect::LoadProcessingConfiguration { request_id, .. } => Some(*request_id),
            _ => None,
        })
        .expect("triage start should load its frozen configuration");

    let mut active_versions = HashMap::new();
    for prompt_id in [
        PromptId::ArticleTriage,
        PromptId::ArticleSummary,
        PromptId::ArticleSignalCandidate,
    ] {
        active_versions.insert(prompt_id, 1);
    }
    active_versions.insert(PromptId::BriefingExecutiveSummary, 1);
    active_versions.insert(PromptId::BriefingNextItem, 1);
    let mut effective_models = HashMap::new();
    effective_models.insert(PromptId::ArticleTriage, "model-old-triage".to_string());
    effective_models.insert(PromptId::ArticleSummary, "model-old-summary".to_string());
    effective_models.insert(
        PromptId::ArticleSignalCandidate,
        "model-old-scoring".to_string(),
    );
    effective_models.insert(
        PromptId::BriefingExecutiveSummary,
        "test-briefing-model".to_string(),
    );
    effective_models.insert(
        PromptId::BriefingNextItem,
        "test-briefing-model".to_string(),
    );
    let (state, effects) = reduce(
        state,
        Msg::ProcessingConfigurationLoaded {
            request_id: configuration_request,
            contexts: contexts_for("old-context"),
            active_versions,
            effective_models,
            preparation_budget: PREPARATION_BUDGET,
        },
    );
    let load_request = effects
        .iter()
        .find_map(|effect| match effect {
            Effect::LoadArticlesForTriage { request_id, .. } => Some(*request_id),
            _ => None,
        })
        .expect("Resume refreshes the current window before triage");
    let articles = state.pre_triage().resolved_included_articles();
    let (state, effects) = reduce(
        state,
        Msg::TriageArticlesLoaded {
            request_id: load_request,
            delta: harvester_engine::TriageArticleDelta::full_window(articles, PREPARATION_BUDGET),
        },
    );
    let request_id = effects
        .iter()
        .find_map(|effect| match effect {
            Effect::RequestLlmCompletion {
                request_id,
                prompt_id: PromptId::ArticleTriage,
                ..
            } => Some(*request_id),
            _ => None,
        })
        .expect("triage should dispatch an article request");
    (state, request_id)
}

#[test]
fn hydrated_current_key_caches_complete_a_fresh_window_delta() {
    let article = article("https://example.com/reloaded", "reload-hash");
    let state = configured_state(AppState::new(), 1, "model-old", "old-context");
    let state = seed_all_current_caches(state, &article, 3);
    let state = load_window(state, vec![article]);

    let summary = known(&state);
    assert_eq!(summary.complete, 1);
    assert_eq!(summary.articles_with_work, 0);
    assert_eq!(summary.estimated_calls, 0);
}

#[test]
fn prompt_model_and_context_changes_invalidate_previously_complete_triage() {
    let article = article("https://example.com/reconfigured", "reconfigured-hash");
    for (version, model, context) in [
        (2, "model-old", "old-context"),
        (1, "model-new", "old-context"),
        (1, "model-old", "new-context"),
    ] {
        let state = configured_state(AppState::new(), 1, "model-old", "old-context");
        let state = seed_all_current_caches(state, &article, 3);
        let state = load_window(state, vec![article.clone()]);
        assert_eq!(known(&state).complete, 1);

        let state = configured_state(state, version, model, context);
        let summary = known(&state);
        assert_eq!(summary.needs_triage, 1);
        assert_eq!(summary.estimated_calls, 3);
    }
}

#[test]
fn failed_triage_needs_triage_and_deferred_triage_is_in_progress() {
    let article = article("https://example.com/triage-state", "triage-state-hash");
    let base = load_window(
        configured_state(AppState::new(), 1, "model-old", "old-context"),
        vec![article],
    );

    let (state, request_id) = start_triage(base.clone());
    assert!(matches!(
        state.triage().articles()[0].triage_state,
        ArticleTriageState::InProgress { .. }
    ));
    let (state, _) = reduce(
        state,
        Msg::LlmCompleted {
            request_id,
            result: crate::LlmResultKind::Failed {
                reason: "test failure".to_string(),
            },
            metadata: None,
        },
    );
    assert_eq!(known(&state).needs_triage, 1);

    let (state, request_id) = start_triage(base);
    let (state, _) = reduce(
        state,
        Msg::LlmCompleted {
            request_id,
            result: crate::LlmResultKind::DeferredToBatch,
            metadata: None,
        },
    );
    assert!(matches!(
        state.triage().articles()[0].triage_state,
        ArticleTriageState::Deferred
    ));
    assert_eq!(known(&state).in_progress, 1);
    assert_eq!(known(&state).articles_with_work, 0);
}

#[test]
fn admitted_pending_triage_articles_are_in_progress_before_dispatch() {
    let articles = (0..3)
        .map(|index| {
            article(
                &format!("https://example.com/pending/{index}"),
                &format!("pending-hash-{index}"),
            )
        })
        .collect();
    let state = load_window(
        configured_state(AppState::new(), 1, "model-old", "old-context"),
        articles,
    );

    let (state, _) = start_triage(state);

    assert_eq!(known(&state).in_progress, 3);
    assert_eq!(known(&state).needs_triage, 0);
    assert_eq!(known(&state).articles_with_work, 0);
    assert_eq!(known(&state).estimated_calls, 0);
}

#[test]
fn rearmed_pending_triage_with_old_snapshot_remains_in_progress() {
    let candidate = article("https://example.com/rearmed", "rearmed-hash");
    let state = load_window(
        configured_state(AppState::new(), 1, "model-old", "old-context"),
        vec![candidate],
    );
    let (state, request_id) = start_triage(state);
    let (state, _) = reduce(
        state,
        Msg::LlmCompleted {
            request_id,
            result: crate::LlmResultKind::DeferredToBatch,
            metadata: None,
        },
    );
    let mut state = configured_state(state, 2, "model-old", "old-context");
    assert_eq!(known(&state).needs_triage, 1);
    state.triage_mut().rearm_deferred();
    state.recompute_unfinished_work();
    assert_eq!(known(&state).in_progress, 1);
    assert_eq!(known(&state).articles_with_work, 0);
}

#[test]
fn admitted_pending_and_deferred_summaries_are_in_progress() {
    let articles = (0..3)
        .map(|index| {
            article(
                &format!("https://example.com/summary/{index}"),
                &format!("summary-hash-{index}"),
            )
        })
        .collect::<Vec<_>>();
    let state = load_window(
        configured_state(AppState::new(), 1, "model-old", "old-context"),
        articles.clone(),
    );
    let state = support::start_briefing_after_triage(state, articles.clone());
    let state = support::with_signal_candidate_metadata(state);

    let mut triage_cache = TriageCache::new();
    for article in &articles {
        triage_cache.insert(
            state
                .current_triage_cache_key(&article.content_hash)
                .expect("current triage key"),
            triage_result(3),
        );
    }
    let state = reduce(
        state,
        Msg::TriageCacheHydrated {
            cache: triage_cache,
        },
    )
    .0;
    let (state, effects) = reduce(
        state,
        Msg::ArticlesLoaded {
            articles,
            collection_text: "summary test collection".to_string(),
        },
    );
    assert_eq!(known(&state).in_progress, 3);
    let request_id = effects
        .iter()
        .find_map(|effect| match effect {
            Effect::RequestLlmCompletion {
                request_id,
                prompt_id: PromptId::ArticleSummary,
                ..
            } => Some(*request_id),
            _ => None,
        })
        .expect("summary should be dispatched");

    let (state, _) = reduce(
        state,
        Msg::LlmCompleted {
            request_id,
            result: crate::LlmResultKind::DeferredToBatch,
            metadata: None,
        },
    );
    assert_eq!(known(&state).in_progress, 3);
    assert_eq!(known(&state).needs_summary, 0);
}

#[test]
fn stale_summaries_need_work_and_current_upstream_results_need_scoring() {
    let high = article("https://example.com/high", "high-hash");
    let low = article("https://example.com/low", "low-hash");
    let excluded = article("https://youtube.com/watch/123", "excluded-hash");
    let state = configured_state(AppState::new(), 1, "model-old", "old-context");
    let state = load_window(state, vec![high.clone(), low.clone(), excluded.clone()]);

    let mut triage_cache = TriageCache::new();
    for (article, priority) in [(&high, 3), (&low, 1)] {
        triage_cache.insert(
            state
                .current_triage_cache_key(&article.content_hash)
                .expect("current triage key"),
            triage_result(priority),
        );
    }
    let state = reduce(
        state,
        Msg::TriageCacheHydrated {
            cache: triage_cache,
        },
    )
    .0;

    let stale_summary_key = SummaryCacheKey::try_new(
        &high.content_hash,
        PromptId::ArticleSummary,
        Some(0),
        Some("model-stale-summary"),
        state.context_for(PromptId::ArticleSummary),
    )
    .expect("stale summary key");
    let mut stale_summaries = crate::SummaryCache::new();
    stale_summaries.insert(
        stale_summary_key,
        SummaryCacheEntry {
            result: summary_result("Stale summary"),
            created_at_utc: "2026-09-19T12:00:00Z".to_string(),
        },
    );
    let state = reduce(
        state,
        Msg::SummaryCacheHydrated {
            cache: stale_summaries,
        },
    )
    .0;
    let summary = known(&state);
    assert_eq!(summary.needs_summary, 1);
    assert_eq!(summary.not_eligible, 2);

    let state = seed_current_summary_cache(
        state,
        &[
            (&high, summary_result("Current high summary")),
            (&low, summary_result("Current low summary")),
        ],
    );
    let summary = known(&state);
    assert_eq!(summary.needs_scoring, 1);
    assert_eq!(summary.needs_summary, 0);
    assert_eq!(summary.not_eligible, 2);
    assert_eq!(summary.estimated_calls, 1);
    let stages = state
        .unfinished_stage_verdicts(&high.url, &high.content_hash)
        .unwrap();
    assert_eq!(stages.triage, UnfinishedStageVerdict::Complete);
    assert_eq!(stages.summary, UnfinishedStageVerdict::Complete);
    assert_eq!(stages.scoring, UnfinishedStageVerdict::NeedsWork);
    // Priority 1 is below both current cutoffs (>1 for summary, >=2 for scoring).
}

#[test]
fn missing_metadata_keeps_even_a_loaded_window_unknown() {
    let state = load_window(
        AppState::new(),
        vec![article(
            "https://example.com/no-metadata",
            "no-metadata-hash",
        )],
    );
    assert_eq!(state.unfinished_work(), &UnfinishedWork::Unknown);
}

#[test]
fn window_metadata_session_and_cache_changes_refresh_the_stored_summary() {
    let article = article("https://example.com/recomputed", "recomputed-hash");
    let unknown = load_window(AppState::new(), vec![article.clone()]);
    assert_eq!(unknown.unfinished_work(), &UnfinishedWork::Unknown);
    let metadata_loaded = configured_state(unknown, 1, "model-old", "old-context");
    assert_eq!(known(&metadata_loaded).needs_triage, 1);

    let state = configured_state(AppState::new(), 1, "model-old", "old-context");
    assert_eq!(known(&state).window_articles(), 0);
    let state = load_window(state, vec![article]);
    assert_eq!(known(&state).needs_triage, 1);

    let (state, request_id) = start_triage(state);
    assert_eq!(known(&state).in_progress, 1);
    let (state, _) = reduce(
        state,
        Msg::LlmCompleted {
            request_id,
            result: crate::LlmResultKind::Success {
                output_json: support::triage_json(),
                input_tokens: 10,
                output_tokens: 5,
                prompt_version: 1,
                resolved_model: "model-old-triage".to_string(),
            },
            metadata: None,
        },
    );
    assert_eq!(known(&state).needs_triage, 0);
    assert_eq!(known(&state).needs_summary, 0);
    assert_eq!(known(&state).in_progress, 1);
    assert_eq!(known(&state).estimated_calls, 0);
}

#[test]
fn views_and_persistence_snapshots_leave_the_stored_summary_unchanged() {
    let article = article("https://example.com/view", "view-hash");
    let state = load_window(
        configured_state(AppState::new(), 1, "model-old", "old-context"),
        vec![article],
    );
    let before = state.unfinished_work().clone();
    let _view = state.view();
    let _snapshot = crate::PersistenceSnapshot::capture(&state);
    assert_eq!(state.unfinished_work(), &before);
}

#[test]
fn briefing_quota_exhaustion_reclassifies_all_pending_article_work() {
    let articles = (0..3)
        .map(|index| {
            article(
                &format!("https://example.com/quota/{index}"),
                &format!("quota-hash-{index}"),
            )
        })
        .collect();
    let state = load_window(
        configured_state(AppState::new(), 1, "model-old", "old-context"),
        articles,
    );
    let (mut state, _) = start_triage(state);
    assert_eq!(known(&state).in_progress, 3);
    state.briefing_mut().set_briefing_request_id(9_999);
    let (state, _) = reduce(
        state,
        Msg::LlmCompleted {
            request_id: 9_999,
            result: crate::LlmResultKind::QuotaExhausted {
                reason: "session quota exhausted".to_string(),
                origin: harvester_engine::llm::QuotaOrigin::SessionBudget,
            },
            metadata: None,
        },
    );
    assert_eq!(known(&state).in_progress, 1);
    assert_eq!(known(&state).needs_triage, 2);
    assert_eq!(known(&state).articles_with_work, 2);
}

#[test]
fn restoring_completed_jobs_clears_the_old_window_summary() {
    let candidate = article("https://example.com/restored", "restored-hash");
    let state = load_window(
        configured_state(AppState::new(), 1, "model-old", "old-context"),
        vec![candidate],
    );
    assert_eq!(known(&state).needs_triage, 1);
    let (state, _) = reduce(
        state,
        Msg::RestoreCompletedJobs(vec![crate::CompletedJobSnapshot {
            url: "https://example.com/restored".into(),
            tokens: Some(100),
            bytes: Some(1_000),
            links: Vec::new(),
            fetched_utc: Some("2026-09-20T12:00:00Z".into()),
        }]),
    );
    assert_eq!(known(&state).window_articles(), 0);
}

#[test]
fn deferred_scoring_is_in_progress_under_its_current_key() {
    let candidate = article(
        "https://example.com/deferred-scoring",
        "deferred-scoring-hash",
    );
    let state = load_window(
        configured_state(AppState::new(), 1, "model-old", "old-context"),
        vec![candidate.clone()],
    );
    let triage_key = state
        .current_triage_cache_key(&candidate.content_hash)
        .unwrap();
    let mut triage_cache = TriageCache::new();
    triage_cache.insert(triage_key, triage_result(3));
    let state = reduce(
        state,
        Msg::TriageCacheHydrated {
            cache: triage_cache,
        },
    )
    .0;
    let mut state =
        seed_current_summary_cache(state, &[(&candidate, summary_result("Deferred scoring"))]);
    assert_eq!(known(&state).needs_scoring, 1);
    let summary_key = current_summary_key(&state, &candidate);
    let scoring_key = crate::update::signal_candidate::input_key_for_current_results(
        &state,
        &candidate,
        &triage_result(3),
        &summary_key,
        &summary_result("Deferred scoring"),
    )
    .unwrap();
    state
        .signal_candidate_mut()
        .enqueue(candidate.url.clone(), scoring_key.digest());
    state.signal_candidate_mut().defer(&candidate.url);
    state.recompute_unfinished_work();
    assert_eq!(known(&state).in_progress, 1);
    assert_eq!(known(&state).articles_with_work, 0);
    assert_eq!(
        state
            .unfinished_stage_verdicts(&candidate.url, &candidate.content_hash)
            .unwrap()
            .scoring,
        UnfinishedStageVerdict::InProgress
    );
}

#[test]
fn cache_eviction_refreshes_the_other_affected_identity() {
    let old = article("https://example.com/old-cache", "old-cache-hash");
    let new = article("https://example.com/new-cache", "new-cache-hash");
    let state = configured_state(AppState::new(), 1, "model-old", "old-context");
    let mut cache = TriageCache::new();
    for index in 0..10_000 {
        let hash = if index == 0 {
            old.content_hash.clone()
        } else {
            format!("filler-{index}")
        };
        let key = state.current_triage_cache_key(&hash).unwrap();
        cache.insert_entry(
            key,
            crate::TriageCacheEntry {
                result: triage_result(3),
                created_at_utc: if index == 0 {
                    "2000-01-01T00:00:00Z"
                } else {
                    "2026-01-01T00:00:00Z"
                }
                .to_string(),
            },
        );
    }
    let state = reduce(state, Msg::TriageCacheHydrated { cache }).0;
    let mut state = load_window(state, vec![old.clone(), new.clone()]);
    assert_eq!(known(&state).needs_summary, 1);
    assert_eq!(known(&state).needs_triage, 1);
    state.store_triage_result_with_model(&new.content_hash, triage_result(3));
    state.refresh_unfinished_identity(&new.url, &new.content_hash);
    state.refresh_unfinished_evictions();
    assert_eq!(known(&state).needs_summary, 1);
    assert_eq!(known(&state).needs_triage, 1);
    assert_eq!(
        state
            .unfinished_stage_verdicts(&old.url, &old.content_hash)
            .unwrap()
            .triage,
        UnfinishedStageVerdict::NeedsWork
    );
    assert_eq!(
        state
            .unfinished_stage_verdicts(&new.url, &new.content_hash)
            .unwrap()
            .triage,
        UnfinishedStageVerdict::Complete
    );
}
