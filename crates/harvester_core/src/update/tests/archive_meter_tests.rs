//! Archive meter contracts through production reducer messages; no view/IPC changes.
use crate::{
    update, AppState, ArchiveMeterStatus as Status, ArticleSummaryResult, ArticleTriageResult,
    CompletedJobSnapshot, Effect, JobListMode, LoadedArticle, Msg, OverrideKey, SavedResult,
    SignalCandidateCache, SignalCandidateCacheEntry, SignalCandidateCacheKey,
    SignalCandidateDialogDefault as DialogDefault, StartupReadinessStatus as Readiness,
    SummaryCache, SummaryCacheEntry, TriageCache, TriageCacheEntry, UnfinishedWork,
};
use harvester_engine::{
    llm::{
        dto::{Confidence, SignalCandidateResult, SourceTier},
        prompt::PromptId,
    },
    WindowArticle,
};
use std::collections::{HashMap, HashSet};

const DATE: &str = "2026-10-03T09:00:00Z";
fn now() -> chrono::DateTime<chrono::Utc> {
    chrono::DateTime::parse_from_rfc3339("2026-10-03T12:00:00Z")
        .unwrap()
        .with_timezone(&chrono::Utc)
}
fn article(name: &str) -> LoadedArticle {
    LoadedArticle {
        url: format!("https://meter.example/{name}"),
        content_hash: format!("hash-{name}"),
        source_title: Some(name.into()),
        fetched_utc: Some(DATE.into()),
        prepared_text: "body ".repeat(220),
    }
}
fn triage(priority: u8) -> ArticleTriageResult {
    ArticleTriageResult {
        category: "news".into(),
        priority,
        tags: vec![],
        rationale: "fixture".into(),
        input_tokens: 100,
        output_tokens: 5,
    }
}
fn summary() -> ArticleSummaryResult {
    ArticleSummaryResult {
        title: "Article".into(),
        summary: "summary".into(),
        key_points: vec!["point".into()],
        entities: Default::default(),
        input_tokens: 100,
        output_tokens: 20,
    }
}
fn score(cluster: &str, value: u8) -> SignalCandidateResult {
    SignalCandidateResult {
        signal_key: cluster.into(),
        signal_score: value,
        themes: vec!["ai".into()],
        draft_gist: "An outlet reports a concrete infrastructure event for the fixture.".into(),
        source_tier: SourceTier::Tier1,
        confidence: Confidence::High,
        reasoning: "fixture".into(),
        input_tokens: 100,
        output_tokens: 10,
    }
}
fn metadata() -> Msg {
    Msg::LlmMetadataLoaded {
        active_versions: [
            PromptId::ArticleTriage,
            PromptId::ArticleSummary,
            PromptId::ArticleSignalCandidate,
        ]
        .into_iter()
        .map(|id| (id, 1))
        .collect(),
        effective_models: [
            (PromptId::ArticleTriage, "triage-model"),
            (PromptId::ArticleSummary, "summary-model"),
            (PromptId::ArticleSignalCandidate, "score-model"),
        ]
        .into_iter()
        .map(|(id, model)| (id, model.into()))
        .collect(),
    }
}
fn exclusion(cluster: &str) -> OverrideKey {
    OverrideKey {
        signal_key: cluster.into(),
        prompt_id: PromptId::ArticleSignalCandidate.to_string(),
        prompt_version: 1,
    }
}
fn members(articles: &[LoadedArticle]) -> Vec<WindowArticle> {
    articles
        .iter()
        .map(|a| WindowArticle {
            url: a.url.clone(),
            content_hash: a.content_hash.clone(),
            source_title: a.source_title.clone(),
            fetched_utc: a.fetched_utc.clone(),
        })
        .collect()
}
fn scan(state: AppState, request_id: u64, articles: &[LoadedArticle]) -> AppState {
    update(
        state,
        Msg::SavedArticlesLoaded {
            request_id,
            articles: members(articles),
        },
    )
    .0
}
fn resolve(state: AppState, request_id: u64, articles: &[LoadedArticle]) -> AppState {
    let window = articles
        .iter()
        .filter(|a| {
            a.fetched_utc
                .as_deref()
                .and_then(|s| chrono::DateTime::parse_from_rfc3339(s).ok())
                .is_none_or(|date| state.briefing_since_utc().is_none_or(|since| date >= since))
        })
        .cloned()
        .collect();
    update(
        state,
        Msg::TriageArticlesLoaded {
            request_id,
            delta: harvester_engine::TriageArticleDelta::full_window(window, 100_000),
        },
    )
    .0
}
fn request_load(mut state: AppState, articles: &[LoadedArticle]) -> (AppState, u64) {
    state = update(
        state,
        Msg::EvaluatePreTriageRefresh {
            ordered_urls: articles.iter().map(|a| a.url.clone()).collect(),
            triggered_by_job_done: false,
        },
    )
    .0;
    for _ in 0..30 {
        let (next, effects) = update(state, Msg::tick_at(now()));
        state = next;
        if let Some(id) = effects.iter().find_map(|e| match e {
            Effect::LoadArticlesForTriage { request_id, .. } => Some(*request_id),
            _ => None,
        }) {
            return (state, id);
        }
    }
    panic!("article request did not dispatch")
}
fn load(state: AppState, articles: &[LoadedArticle]) -> AppState {
    let (state, id) = request_load(state, articles);
    resolve(scan(state, id, articles), id, articles)
}
fn replies(mut state: AppState) -> AppState {
    for msg in [
        Msg::BriefingCheckpointLoaded { since_utc: None },
        Msg::PromptTemplateFilesLoaded,
        metadata(),
        Msg::PromptContextsLoaded {
            contexts: HashMap::new(),
        },
    ] {
        state = update(state, msg).0;
    }
    state
}

#[derive(Clone)]
struct Fixture {
    articles: Vec<LoadedArticle>,
    triages: TriageCache,
    summaries: SummaryCache,
    scores: SignalCandidateCache,
    exclusions: HashSet<OverrideKey>,
}
type FixtureRow<'a> = (&'a str, Option<u8>, bool, Option<(u8, &'a str)>);
impl Fixture {
    // name, priority (None = missing triage), summary present, score/cluster
    fn new(rows: &[FixtureRow<'_>]) -> Self {
        let mut f = Self {
            articles: rows.iter().map(|row| article(row.0)).collect(),
            triages: TriageCache::new(),
            summaries: SummaryCache::new(),
            scores: SignalCandidateCache::default(),
            exclusions: HashSet::new(),
        };
        let configured = replies(AppState::new());
        for (name, priority, has_summary, signal) in rows {
            let a = article(name);
            let tkey = configured
                .current_triage_cache_key(&a.content_hash)
                .unwrap();
            if let Some(priority) = priority {
                f.triages.insert_entry(
                    tkey,
                    TriageCacheEntry {
                        result: triage(*priority),
                        created_at_utc: DATE.into(),
                    },
                );
            }
            let skey = configured
                .current_summary_cache_key(&a.content_hash)
                .unwrap();
            if *has_summary {
                f.summaries.insert(
                    skey.clone(),
                    SummaryCacheEntry {
                        result: summary(),
                        created_at_utc: DATE.into(),
                    },
                );
            }
            if let Some((value, cluster)) = signal {
                let key = crate::update::signal_candidate::input_key_for_current_results(
                    &configured,
                    &a,
                    &triage(priority.unwrap()),
                    &skey,
                    &summary(),
                )
                .unwrap();
                f.scores.insert(
                    key,
                    SignalCandidateCacheEntry {
                        result: score(cluster, *value),
                        created_at_utc: DATE.into(),
                    },
                );
            }
        }
        f
    }
    fn selected() -> Self {
        let mut f = Self::new(&[
            ("a", Some(5), true, Some((95, "duplicate"))),
            ("b", Some(5), true, Some((90, "duplicate"))),
            ("c", Some(5), true, Some((85, "excluded"))),
            ("d", Some(5), true, Some((80, "other"))),
        ]);
        f.exclusions.insert(exclusion("excluded"));
        f
    }
    fn hydrate(&self) -> AppState {
        self.hydrate_with_store_failure(None)
    }
    fn hydrate_with_store_failure(&self, failure: Option<&str>) -> AppState {
        let mut state = update(AppState::new(), Msg::StartupHydrationRequested).0;
        if let Some(reason) = failure {
            state = update(
                state,
                Msg::ResultStoreUnavailable {
                    reason: reason.into(),
                },
            )
            .0;
        }
        for msg in [
            Msg::RestoreCompletedJobs(
                self.articles
                    .iter()
                    .map(|a| CompletedJobSnapshot {
                        url: a.url.clone(),
                        fetched_utc: a.fetched_utc.clone(),
                        tokens: Some(1000),
                        bytes: Some(10000),
                        links: vec![],
                    })
                    .collect(),
            ),
            Msg::SummaryCacheHydrated {
                cache: self.summaries.clone(),
            },
            Msg::TriageCacheHydrated {
                cache: self.triages.clone(),
            },
            Msg::SignalCandidateCacheLoaded {
                cache: self.scores.clone(),
            },
            Msg::SignalCandidateOverridesLoaded {
                overrides: self.exclusions.clone(),
            },
            Msg::RestoreDesktopView {
                mode: None,
                selected_article_url: None,
                now: now(),
            },
        ] {
            state = update(state, msg).0;
        }
        state
    }
    fn ready(&self) -> AppState {
        load(replies(self.hydrate()), &self.articles)
    }
}
fn assert_zero(state: &AppState, status: Status) {
    let meter = state.archive_meter_view();
    assert_eq!(meter.status, status);
    assert_eq!(
        (
            meter.selected_count,
            meter.token_estimate,
            meter.unsettled_count,
            meter.target
        ),
        (0, 0, 0, 150)
    );
}
fn dialog(state: AppState, default: DialogDefault) -> AppState {
    let meter = state.archive_meter_view();
    let (state, effects) = update(state, Msg::ArchiveClicked);
    assert!(effects.iter().any(|e| matches!(e, Effect::OpenArchiveDialog { signal_candidate_default, signal_candidate_count, signal_candidate_token_estimates, .. }
        if *signal_candidate_default == default && *signal_candidate_count == meter.selected_count && signal_candidate_token_estimates.summary_tokens == meter.token_estimate)));
    state
}
fn key_for(state: &AppState, a: &LoadedArticle) -> SignalCandidateCacheKey {
    let summary_key = state.current_summary_cache_key(&a.content_hash).unwrap();
    crate::update::signal_candidate::input_key_for_current_results(
        state,
        a,
        state
            .triage_cache()
            .lookup(&state.current_triage_cache_key(&a.content_hash).unwrap())
            .unwrap()
            .1,
        &summary_key,
        state.try_reuse_summary(&summary_key).unwrap(),
    )
    .unwrap()
}
fn save_score(state: AppState, a: &LoadedArticle, cluster: &str) -> AppState {
    let key = key_for(&state, a);
    update(
        state,
        Msg::ValidatedResultReceived {
            record: Box::new(SavedResult::SignalCandidate(
                key,
                SignalCandidateCacheEntry {
                    result: score(cluster, 90),
                    created_at_utc: DATE.into(),
                },
            )),
        },
    )
    .0
}

#[test]
fn idle_meter_count_equals_dialog_default_export_count() {
    let state = Fixture::selected().ready();
    assert_eq!(state.archive_meter_view().selected_count, 2);
    assert_eq!(state.archive_meter_view().unsettled_count, 0);
    dialog(state, DialogDefault::OnAllSettled);
}
#[test]
fn mid_session_result_write_failure_keeps_meter_and_dialog_selection() {
    let state = Fixture::selected().ready();
    let meter = state.archive_meter_view();
    assert_eq!(meter.selected_count, 2);
    assert!(meter.token_estimate > 0);
    let state = update(
        state,
        Msg::ResultStoreUnavailable {
            reason: "result write failed: disk full".into(),
        },
    )
    .0;
    assert_eq!(state.startup_readiness(), Readiness::Ready);
    assert_eq!(state.archive_meter_view(), meter);
    dialog(state, DialogDefault::OnAllSettled);
}
#[test]
fn restart_without_run_shows_saved_selection() {
    let state = Fixture::selected().ready();
    assert!(state.run_progress().is_none());
    assert_eq!(state.archive_meter_view().status, Status::Scored);
    assert_eq!(state.archive_meter_view().selected_count, 2);
    assert!(state.archive_meter_view().token_estimate > 0);
    dialog(state, DialogDefault::OnAllSettled);
}
#[test]
fn scored_but_none_selected_reads_zero_while_dialog_defaults_to_full_set() {
    let state = Fixture::new(&[("a", Some(5), true, Some((1, "low")))]).ready();
    assert_zero(&state, Status::Scored);
    let state = dialog(state, DialogDefault::OffEmpty);
    assert_eq!(state.archive_corpus().count(), 1);
}
#[test]
fn empty_output_folder_startup_reaches_ready_with_zero() {
    let (mut state, effects) = update(
        Fixture::new(&[]).hydrate(),
        Msg::RestoreCompletedJobs(vec![]),
    );
    assert!(effects.is_empty());
    assert!(state.take_pre_triage_refresh_evaluation_request().is_none());
    let state = replies(state);
    assert_eq!(state.startup_readiness(), Readiness::Ready);
    assert_zero(&state, Status::NotScoredYet);
}
#[test]
fn prompt_context_failure_reads_unavailable_with_zero() {
    let f = Fixture::selected();
    let mut state = update(
        f.hydrate(),
        Msg::PromptContextsLoadFailed {
            reason: "required context missing".into(),
        },
    )
    .0;
    assert_zero(&state, Status::Unavailable);
    for msg in [
        Msg::BriefingCheckpointLoaded { since_utc: None },
        metadata(),
        Msg::PromptTemplateFilesLoaded,
    ] {
        state = update(state, msg).0;
        assert_zero(&state, Status::Unavailable);
    }
    let state = load(state, &f.articles);
    assert!(!state.triage_metadata_ready());
    assert_eq!(state.startup_readiness(), Readiness::Unavailable);
    assert_zero(&state, Status::Unavailable);
}
#[test]
fn refused_result_store_reads_unavailable() {
    // Failure takes precedence over pending and cannot be erased by another store's success.
    let f = Fixture::selected();
    let state = f.hydrate_with_store_failure(Some("refused"));
    assert_zero(&state, Status::Unavailable);
    let state = load(replies(state), &f.articles);
    assert_eq!(state.signal_candidate_selection().selected_urls.len(), 2);
    assert_eq!(state.startup_readiness(), Readiness::Unavailable);
    assert_zero(&state, Status::Unavailable);
}
#[test]
fn startup_article_load_failure_reads_unavailable_then_recovers() {
    let f = Fixture::selected();
    let (state, id) = request_load(replies(f.hydrate()), &f.articles);
    let state = update(
        state,
        Msg::TriageArticlesLoadFailed {
            request_id: id,
            reason: "scan failed".into(),
        },
    )
    .0;
    assert_zero(&state, Status::Unavailable);
    let state = load(state, &f.articles);
    assert_eq!(state.startup_readiness(), Readiness::Ready);
    assert_eq!(state.archive_meter_view().selected_count, 2);
}
#[test]
fn excluded_scored_article_is_not_counted_between_scan_and_pre_triage() {
    let mut f = Fixture::new(&[("a", Some(5), true, Some((90, "a")))]);
    f.articles[0].prepared_text = "tiny".into();
    let (state, id) = request_load(replies(f.hydrate()), &f.articles);
    let state = scan(state, id, &f.articles);
    assert!(state.pre_triage().is_resolved_included(&f.articles[0].url));
    assert_zero(&state, Status::Loading);
    let state = resolve(state, id, &f.articles);
    assert_eq!(state.startup_readiness(), Readiness::Ready);
    assert!(!state.pre_triage().is_resolved_included(&f.articles[0].url));
    assert_zero(&state, Status::NotScoredYet);
}
#[test]
fn selection_restore_waits_for_initial_pre_triage_resolution() {
    let f = Fixture::selected();
    let state = update(
        replies(f.hydrate()),
        Msg::RestoreDesktopView {
            mode: Some(JobListMode::Results),
            selected_article_url: Some(f.articles[0].url.clone()),
            now: now(),
        },
    )
    .0;
    let (state, id) = request_load(state, &f.articles);
    let state = scan(state, id, &f.articles);
    assert!(state.selected_job_id().is_none());
    assert_eq!(
        state.remembered_article_url(),
        Some(f.articles[0].url.clone())
    );
    let state = resolve(state, id, &f.articles);
    assert_eq!(state.selected_job_id(), Some(1));
}
#[test]
fn selection_restores_after_refused_startup_result_store() {
    let f = Fixture::selected();
    let state = update(
        f.hydrate_with_store_failure(Some("refused")),
        Msg::RestoreDesktopView {
            mode: Some(JobListMode::SinceCheckpoint),
            selected_article_url: Some(f.articles[0].url.clone()),
            now: now(),
        },
    )
    .0;
    let state = load(replies(state), &f.articles);
    assert_eq!(state.startup_readiness(), Readiness::Unavailable);
    assert!(state.startup_settled_with_articles());
    assert_eq!(state.selected_job_id(), Some(1));
}
#[test]
fn selection_restores_after_missing_or_blank_startup_model() {
    for blank in [false, true] {
        let f = Fixture::selected();
        let Msg::LlmMetadataLoaded {
            active_versions,
            mut effective_models,
        } = metadata()
        else {
            unreachable!()
        };
        if blank {
            effective_models.insert(PromptId::ArticleTriage, "  ".into());
        } else {
            effective_models.remove(&PromptId::ArticleTriage);
        }
        let mut state = f.hydrate();
        for msg in [
            Msg::RestoreDesktopView {
                mode: Some(JobListMode::SinceCheckpoint),
                selected_article_url: Some(f.articles[0].url.clone()),
                now: now(),
            },
            Msg::LlmMetadataLoaded {
                active_versions,
                effective_models,
            },
            Msg::BriefingCheckpointLoaded { since_utc: None },
            Msg::PromptTemplateFilesLoaded,
            Msg::PromptContextsLoaded {
                contexts: HashMap::new(),
            },
        ] {
            state = update(state, msg).0;
        }
        let state = load(state, &f.articles);
        assert_eq!(state.startup_readiness(), Readiness::Unavailable);
        assert!(state.startup_settled_with_articles());
        assert_eq!(state.selected_job_id(), Some(1));
    }
}
#[test]
fn selection_restores_after_startup_prompt_context_failure() {
    let f = Fixture::selected();
    let mut state = f.hydrate();
    for msg in [
        Msg::RestoreDesktopView {
            mode: Some(JobListMode::SinceCheckpoint),
            selected_article_url: Some(f.articles[0].url.clone()),
            now: now(),
        },
        Msg::PromptContextsLoadFailed {
            reason: "required context missing".into(),
        },
        Msg::BriefingCheckpointLoaded { since_utc: None },
        Msg::PromptTemplateFilesLoaded,
        metadata(),
    ] {
        state = update(state, msg).0;
    }
    let state = load(state, &f.articles);
    assert_eq!(state.startup_readiness(), Readiness::Unavailable);
    assert!(state.startup_settled_with_articles());
    assert_eq!(state.selected_job_id(), Some(1));
}
#[test]
fn selection_restore_waits_for_every_pending_startup_input_even_after_failure() {
    let f = Fixture::selected();
    for pending in 0..4 {
        let mut state = update(AppState::new(), Msg::StartupHydrationRequested).0;
        state = update(
            state,
            Msg::RestoreCompletedJobs(vec![CompletedJobSnapshot {
                url: f.articles[0].url.clone(),
                fetched_utc: f.articles[0].fetched_utc.clone(),
                tokens: Some(1000),
                bytes: Some(10000),
                links: vec![],
            }]),
        )
        .0;
        state = update(
            state,
            Msg::RestoreDesktopView {
                mode: Some(JobListMode::SinceCheckpoint),
                selected_article_url: Some(f.articles[0].url.clone()),
                now: now(),
            },
        )
        .0;
        let mut delayed = None;
        for (index, msg) in [
            Msg::BriefingCheckpointLoaded { since_utc: None },
            metadata(),
            Msg::PromptContextsLoaded {
                contexts: HashMap::new(),
            },
            Msg::ResultStoreUnavailable {
                reason: "refused".into(),
            },
        ]
        .into_iter()
        .enumerate()
        {
            if index == pending {
                delayed = Some(msg);
            } else {
                state = update(state, msg).0;
            }
        }
        state = update(state, Msg::PromptTemplateFilesLoaded).0;
        state = load(state, &f.articles);
        assert!(!state.startup_settled_with_articles());
        assert!(state.selected_job_id().is_none());
        assert_eq!(
            state.remembered_article_url(),
            Some(f.articles[0].url.clone())
        );
        let state = update(state, delayed.unwrap()).0;
        assert!(state.startup_settled_with_articles());
        assert_eq!(state.selected_job_id(), Some(1));
    }
}
#[test]
fn late_exclusions_lower_the_count_like_a_toggle() {
    let mut f = Fixture::selected();
    let overrides = std::mem::take(&mut f.exclusions);
    let state = f.ready();
    assert_eq!(state.archive_meter_view().selected_count, 3);
    let state = update(state, Msg::SignalCandidateOverridesLoaded { overrides }).0;
    assert_eq!(state.archive_meter_view().selected_count, 2);
    let state = update(
        state,
        Msg::ToggleSignalCandidateExclusion {
            signal_key: "excluded".into(),
        },
    )
    .0;
    assert_eq!(state.archive_meter_view().selected_count, 3);
}
#[test]
fn meter_does_not_read_scoring_session_state() {
    let mut state = Fixture::selected().ready();
    let meter = state.archive_meter_view();
    for a in Fixture::selected().articles {
        state
            .signal_candidate_mut()
            .enqueue(a.url.clone(), "input".into());
        assert_eq!(state.archive_meter_view(), meter);
        state.signal_candidate_mut().mark_scoring(&a.url, 99);
        assert_eq!(state.archive_meter_view(), meter);
        state.signal_candidate_mut().fail(&a.url, "failed");
        assert_eq!(state.archive_meter_view(), meter);
    }
}
#[test]
fn scoring_key_invalidation_drops_to_zero_without_triage_fallback() {
    for case in 0..3 {
        let state = Fixture::selected().ready();
        // Re-sending startup replies is a test shortcut for a configuration change.
        let msg = if case == 1 {
            Msg::PromptContextsLoaded {
                contexts: [(
                    PromptId::ArticleSignalCandidate,
                    vec![("topic".into(), "changed".into())],
                )]
                .into_iter()
                .collect(),
            }
        } else {
            let Msg::LlmMetadataLoaded {
                mut active_versions,
                mut effective_models,
            } = metadata()
            else {
                unreachable!()
            };
            if case == 0 {
                active_versions.insert(PromptId::ArticleSignalCandidate, 2);
            } else {
                effective_models.insert(PromptId::ArticleSummary, "changed-model".into());
            }
            Msg::LlmMetadataLoaded {
                active_versions,
                effective_models,
            }
        };
        let state = update(state, msg).0;
        let meter = state.archive_meter_view();
        assert_eq!(meter.status, Status::NotScoredYet);
        assert_eq!(meter.selected_count, 0);
        assert_eq!(meter.token_estimate, 0);
        assert_eq!(meter.unsettled_count, 4);
        assert_eq!(state.archive_corpus().count(), 4);
    }
}
#[test]
fn meter_ignores_out_of_window_and_non_actionable_scores() {
    let mut f = Fixture::new(&[
        ("a", Some(5), true, Some((90, "a"))),
        ("old", Some(5), true, Some((90, "old"))),
        ("excluded", Some(5), true, Some((90, "excluded"))),
    ]);
    f.articles[1].fetched_utc = Some("2026-10-03T01:00:00Z".into());
    f.articles[2].prepared_text = "tiny".into();
    // Fetch date is part of the score input, so rebuild that entry's saved score under its actual identity.
    let configured = replies(AppState::new());
    let a = &f.articles[1];
    let skey = configured
        .current_summary_cache_key(&a.content_hash)
        .unwrap();
    let key = crate::update::signal_candidate::input_key_for_current_results(
        &configured,
        a,
        &triage(5),
        &skey,
        &summary(),
    )
    .unwrap();
    f.scores.insert(
        key,
        SignalCandidateCacheEntry {
            result: score("old", 90),
            created_at_utc: DATE.into(),
        },
    );
    let state = update(
        replies(f.hydrate()),
        Msg::BriefingCheckpointLoaded {
            since_utc: Some("2026-10-03T06:00:00Z".into()),
        },
    )
    .0;
    let state = load(state, &f.articles);
    assert_eq!(state.archive_meter_view().selected_count, 1);
    for a in &f.articles {
        assert!(state
            .saved_results_for_url(&a.url)
            .unwrap()
            .signal
            .is_some());
    }
    assert_eq!(
        state
            .view()
            .signal_candidate_rows
            .iter()
            .filter(|r| r.outcome == Some(crate::SignalCandidateOutcome::Selected))
            .count(),
        3
    );
}
#[test]
fn unsettled_count_counts_only_unsettled_window_articles() {
    let cutoff = AppState::new().briefing_triage_policy().cutoff_exclusive;
    let scoring_cutoff = crate::update::signal_candidate::PRIORITY_CUTOFF_INCLUSIVE;
    let mut f = Fixture::new(&[
        ("triage", None, false, None),
        ("summary", Some(5), false, None),
        ("scoring", Some(5), true, None),
        ("failed", Some(5), true, None),
        ("below-summary", Some(cutoff), false, None),
        ("below-scoring", Some(scoring_cutoff - 1), true, None),
        ("scored", Some(5), true, Some((90, "scored"))),
        ("excluded", None, false, None),
        ("outside", None, false, None),
    ]);
    f.articles[7].prepared_text = "tiny".into();
    f.articles[8].fetched_utc = Some("2026-10-03T01:00:00Z".into());
    let state = update(
        replies(f.hydrate()),
        Msg::BriefingCheckpointLoaded {
            since_utc: Some("2026-10-03T06:00:00Z".into()),
        },
    )
    .0;
    let mut state = load(state, &f.articles);
    let url = &f.articles[3].url;
    state
        .signal_candidate_mut()
        .enqueue(url.clone(), "input".into());
    state
        .signal_candidate_mut()
        .fail(url, "last attempt failed");
    let state = update(state, Msg::InputChanged(String::new())).0;
    assert_eq!(state.archive_meter_view().unsettled_count, 4);
    let UnfinishedWork::Known(work) = state.unfinished_work() else {
        panic!("unfinished work unknown")
    };
    assert_eq!(
        state.archive_meter_view().unsettled_count,
        work.in_progress + work.articles_with_work
    );
}
#[test]
fn startup_message_orders_never_over_count() {
    fn permutations(
        prefix: &mut Vec<usize>,
        remaining: &mut Vec<usize>,
        output: &mut Vec<Vec<usize>>,
    ) {
        if remaining.is_empty() {
            output.push(prefix.clone());
            return;
        }
        for index in 0..remaining.len() {
            let value = remaining.remove(index);
            prefix.push(value);
            permutations(prefix, remaining, output);
            prefix.pop();
            remaining.insert(index, value);
        }
    }
    let f = Fixture::selected();
    let mut orders = vec![];
    permutations(&mut vec![], &mut (0..7).collect(), &mut orders);
    let mut checked = 0;
    for order in orders.into_iter().filter(|order| {
        let position = |value| order.iter().position(|x| *x == value).unwrap();
        position(4) < position(5) && position(5) < position(6)
    }) {
        // Tick dispatch can occur before or after any independent reply; the
        // worker pair must follow the requesting effect, in scan/resolution order.
        let mut state = f.hydrate();
        let mut request_id = None;
        for step in order {
            state = match step {
                0 => update(state, Msg::BriefingCheckpointLoaded { since_utc: None }).0,
                1 => update(state, Msg::PromptTemplateFilesLoaded).0,
                2 => update(state, metadata()).0,
                3 => {
                    update(
                        state,
                        Msg::PromptContextsLoaded {
                            contexts: HashMap::new(),
                        },
                    )
                    .0
                }
                4 => {
                    let (next, id) = request_load(state, &f.articles);
                    request_id = Some(id);
                    next
                }
                5 => scan(state, request_id.unwrap(), &f.articles),
                6 => resolve(state, request_id.unwrap(), &f.articles),
                _ => unreachable!(),
            };
            assert!(state.archive_meter_view().selected_count <= 2);
            if state.startup_readiness() == Readiness::Ready {
                assert_eq!(state.archive_meter_view().selected_count, 2);
            } else {
                assert_zero(&state, Status::Loading);
            }
        }
        assert_eq!(state.startup_readiness(), Readiness::Ready);
        checked += 1;
    }
    assert_eq!(checked, 840);
}
#[test]
fn fresh_window_after_checkpoint_archive_reads_zero_then_grows() {
    let f = Fixture::selected();
    let state = dialog(f.ready(), DialogDefault::OnAllSettled);
    let request_id = state.archive_request_id();
    let (state, effects) = update(
        state,
        Msg::ArchiveDialogSubmitted {
            request_id,
            basename: "archive.md".into(),
            set_checkpoint: true,
            submitted_at: now(),
            use_summaries: true,
            use_signal_candidates: true,
        },
    );
    assert!(state.signal_exclusions().excluded().is_empty());
    assert!(effects.iter().any(|e| matches!(e, Effect::PersistSignalCandidateOverrides { overrides } if overrides.is_empty())));
    let (state, effects) = update(
        state,
        Msg::ArchiveExportCompleted {
            request_id,
            path: "archive.md".into(),
            doc_count: 2,
            requested_checkpoint: Some(now()),
        },
    );
    let save_id = effects
        .iter()
        .find_map(|e| match e {
            Effect::SaveBriefingCheckpoint { save_id, .. } => Some(*save_id),
            _ => None,
        })
        .unwrap();
    let state = update(state, Msg::BriefingCheckpointSaveSucceeded { save_id }).0;
    assert_zero(&state, Status::NotScoredYet);
    let mut fresh = Fixture::new(&[
        ("new-a", Some(5), true, None),
        ("new-b", Some(5), true, None),
        ("new-c", Some(5), true, None),
    ]);
    for a in &mut fresh.articles {
        a.fetched_utc = Some("2026-10-03T13:00:00Z".into());
    }
    // Re-sending cache hydration is a test shortcut for a configuration change.
    let state = update(
        state,
        Msg::TriageCacheHydrated {
            cache: fresh.triages.clone(),
        },
    )
    .0;
    let state = update(
        state,
        Msg::SummaryCacheHydrated {
            cache: fresh.summaries.clone(),
        },
    )
    .0;
    let mut state = load(state, &fresh.articles);
    for (index, cluster) in ["one", "one", "two"].into_iter().enumerate() {
        state = save_score(state, &fresh.articles[index], cluster);
        assert_eq!(
            state.archive_meter_view().selected_count,
            if index < 2 { 1 } else { 2 }
        );
    }
}
#[test]
fn mid_run_count_grows_monotonically_while_scoring_is_pending() {
    let f = Fixture::new(&[
        ("a", Some(5), true, None),
        ("b", Some(5), true, None),
        ("c", Some(5), true, None),
        ("d", Some(5), true, None),
    ]);
    let state = f.ready();
    let (state, effects) = update(
        state,
        Msg::PipelineRunRequested {
            scope: crate::PipelineRunScope::Resume,
        },
    );
    let (mut state, mut effects) =
        crate::fixture_support::complete_processing_start(state, effects, 100_000);
    let mut previous = 0;
    let mut completed = 0;
    loop {
        assert!(state.archive_meter_view().selected_count >= previous);
        previous = state.archive_meter_view().selected_count;
        assert!(previous <= 2);
        assert!(previous < f.articles.len());
        let Some(id) =
            super::support::request_id_for_prompt(&effects, PromptId::ArticleSignalCandidate)
        else {
            break;
        };
        let url = state.signal_candidate().url_for_request(id).unwrap();
        assert!(
            state.saved_results_for_url(url).is_some(),
            "scoring must have an indexed article before completion"
        );
        let cluster = if completed < 2 {
            "first-event-cluster"
        } else {
            "second-event-cluster"
        };
        let result = serde_json::to_string(&score(cluster, 90)).unwrap();
        let mut metadata = super::support::summary_metadata("score-model", 100, 10);
        metadata.prompt_id = PromptId::ArticleSignalCandidate;
        (state, effects) = update(
            state,
            Msg::LlmCompleted {
                request_id: id,
                result: crate::LlmResultKind::Success {
                    output_json: result,
                    input_tokens: 100,
                    output_tokens: 10,
                    prompt_version: 1,
                    resolved_model: "score-model".into(),
                },
                metadata: Some(metadata),
            },
        );
        completed += 1;
        if completed == 1 {
            // A download joins an already active run. It gains index membership at
            // the resolved article load, before its asynchronous score can return.
            let downloaded = article("downloaded");
            for msg in [
                Msg::InputChanged(downloaded.url.clone()),
                Msg::UrlsSubmitted,
                Msg::JobDone {
                    job_id: 5,
                    result: crate::JobResultKind::Success,
                    extracted_links: vec![],
                    fetched_utc: Some(now().to_rfc3339()),
                },
            ] {
                state = update(state, msg).0;
                assert_eq!(state.archive_meter_view().selected_count, 1);
            }
            assert!(state.saved_results_for_url(&downloaded.url).is_none());
            let tkey = state
                .current_triage_cache_key(&downloaded.content_hash)
                .unwrap();
            let skey = state
                .current_summary_cache_key(&downloaded.content_hash)
                .unwrap();
            for record in [
                SavedResult::Triage(
                    tkey,
                    TriageCacheEntry {
                        result: triage(5),
                        created_at_utc: DATE.into(),
                    },
                ),
                SavedResult::Summary(
                    skey,
                    SummaryCacheEntry {
                        result: summary(),
                        created_at_utc: DATE.into(),
                    },
                ),
            ] {
                state = update(
                    state,
                    Msg::ValidatedResultReceived {
                        record: Box::new(record),
                    },
                )
                .0;
                assert_eq!(state.archive_meter_view().selected_count, 1);
            }
            let mut articles = f.articles.clone();
            articles.push(downloaded.clone());
            state = load(state, &articles);
            assert!(state.saved_results_for_url(&downloaded.url).is_some());
            assert_eq!(state.archive_meter_view().selected_count, 1);
        }
    }
    assert_eq!(completed, 5);
    assert_eq!(state.archive_meter_view().selected_count, 2);
    assert_eq!(state.archive_meter_view().unsettled_count, 0);
}

#[test]
fn missing_or_blank_startup_metadata_reads_unavailable() {
    for id in [
        PromptId::ArticleTriage,
        PromptId::ArticleSummary,
        PromptId::ArticleSignalCandidate,
    ] {
        for blank in [false, true] {
            let Msg::LlmMetadataLoaded {
                active_versions,
                mut effective_models,
            } = metadata()
            else {
                unreachable!()
            };
            if blank {
                effective_models.insert(id, "  ".into());
            } else {
                effective_models.remove(&id);
            }
            let state = Fixture::selected().ready();
            assert_eq!(state.archive_meter_view().selected_count, 2);
            let state = update(
                state,
                Msg::LlmMetadataLoaded {
                    active_versions,
                    effective_models,
                },
            )
            .0;
            assert_zero(&state, Status::Unavailable);
        }
        let Msg::LlmMetadataLoaded {
            mut active_versions,
            effective_models,
        } = metadata()
        else {
            unreachable!()
        };
        active_versions.remove(&id);
        let state = Fixture::selected().ready();
        assert_eq!(state.archive_meter_view().selected_count, 2);
        let state = update(
            state,
            Msg::LlmMetadataLoaded {
                active_versions,
                effective_models,
            },
        )
        .0;
        assert_zero(&state, Status::Unavailable);
    }
}
#[test]
fn stale_article_replies_do_not_complete_startup_readiness() {
    let f = Fixture::selected();
    let (state, id) = request_load(replies(f.hydrate()), &f.articles);
    let state = scan(state, id + 1, &f.articles);
    let state = resolve(state, id + 1, &f.articles);
    let state = update(
        state,
        Msg::TriageArticlesLoadFailed {
            request_id: id + 1,
            reason: "stale".into(),
        },
    )
    .0;
    assert_zero(&state, Status::Loading);
    assert_eq!(state.startup_readiness(), Readiness::Pending);
}

#[test]
fn startup_replies_record_outcomes_after_run_configuration_is_frozen() {
    let f = Fixture::new(&[("a", Some(5), true, None)]);
    let (state, effects) = update(
        f.ready(),
        Msg::PipelineRunRequested {
            scope: crate::PipelineRunScope::Resume,
        },
    );
    let (state, _) = crate::fixture_support::complete_processing_start(state, effects, 100_000);
    assert!(state.pipeline_ready());
    let state = update(
        state,
        Msg::PromptContextsLoadFailed {
            reason: "late startup failure".into(),
        },
    )
    .0;
    assert_zero(&state, Status::Unavailable);
    let state = update(
        state,
        Msg::PromptContextsLoaded {
            contexts: HashMap::new(),
        },
    )
    .0;
    assert_eq!(state.startup_readiness(), Readiness::Ready);
    let Msg::LlmMetadataLoaded {
        mut active_versions,
        effective_models,
    } = metadata()
    else {
        unreachable!()
    };
    active_versions.remove(&PromptId::ArticleSignalCandidate);
    let state = update(
        state,
        Msg::LlmMetadataLoaded {
            active_versions,
            effective_models,
        },
    )
    .0;
    assert_zero(&state, Status::Unavailable);
    let state = update(state, metadata()).0;
    assert_eq!(state.startup_readiness(), Readiness::Ready);
}
