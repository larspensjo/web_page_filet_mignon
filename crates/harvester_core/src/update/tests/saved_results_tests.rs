//! Restart contracts use production messages, with no run or live provider.
use crate::fixture_support::ManualPreTriageDecisions;
use crate::{
    update, AppState, ArticleSummaryResult, ArticleTriageResult, CompletedJobSnapshot, Effect,
    JobListMode, LoadedArticle, Msg, SignalCandidateCache, SummaryCache, SummaryCacheEntry,
    SummaryCacheKey, TriageCache, TriageCacheKey,
};
use harvester_engine::{
    llm::{
        dto::{Confidence, SignalCandidateResult, SourceTier},
        prompt::PromptId,
    },
    WindowArticle,
};
use std::collections::HashMap;

fn now() -> chrono::DateTime<chrono::Utc> {
    chrono::DateTime::parse_from_rfc3339("2026-10-03T12:00:00Z")
        .unwrap()
        .with_timezone(&chrono::Utc)
}
fn article(name: &str) -> LoadedArticle {
    LoadedArticle {
        url: format!("https://restart.example/{name}"),
        content_hash: format!("hash-{name}"),
        source_title: Some(format!("Article {name}")),
        fetched_utc: Some("2026-10-03T09:00:00Z".into()),
        prepared_text: "body ".repeat(220),
    }
}
fn summary(name: &str, tokens: u32) -> ArticleSummaryResult {
    ArticleSummaryResult {
        title: name.into(),
        summary: format!("{name} summary"),
        key_points: vec!["point".into()],
        entities: Default::default(),
        input_tokens: 100,
        output_tokens: tokens,
    }
}
fn triage(priority: u8) -> ArticleTriageResult {
    ArticleTriageResult {
        category: "news".into(),
        priority,
        tags: vec!["ai".into()],
        rationale: "fixture".into(),
        input_tokens: 100,
        output_tokens: 5,
    }
}
fn score(name: &str, value: u8) -> SignalCandidateResult {
    SignalCandidateResult {
        signal_key: name.into(),
        signal_score: value,
        themes: vec!["infrastructure".into()],
        draft_gist: name.into(),
        source_tier: SourceTier::Tier1,
        confidence: Confidence::High,
        reasoning: "fixture".into(),
        input_tokens: 100,
        output_tokens: 10,
    }
}
fn metadata(version: u32) -> Msg {
    Msg::LlmMetadataLoaded {
        active_versions: [
            PromptId::ArticleTriage,
            PromptId::ArticleSummary,
            PromptId::ArticleSignalCandidate,
        ]
        .into_iter()
        .map(|id| (id, version))
        .collect(),
        effective_models: [
            (PromptId::ArticleTriage, "gpt-5.4-mini"),
            (PromptId::ArticleSummary, "summary-model"),
            (PromptId::ArticleSignalCandidate, "score-model"),
        ]
        .into_iter()
        .map(|(id, model)| (id, model.into()))
        .collect(),
    }
}
fn loaded(mut state: AppState, articles: &[LoadedArticle]) -> AppState {
    let request_id = state.alloc_triage_request_id();
    state.set_triage_in_flight(request_id);
    let members = articles
        .iter()
        .map(|a| WindowArticle {
            url: a.url.clone(),
            content_hash: a.content_hash.clone(),
            source_title: a.source_title.clone(),
            fetched_utc: a.fetched_utc.clone(),
        })
        .collect();
    state = update(
        state,
        Msg::SavedArticlesLoaded {
            request_id,
            articles: members,
        },
    )
    .0;
    let since = state.briefing_since_utc().unwrap();
    update(
        state,
        Msg::TriageArticlesLoaded {
            request_id,
            delta: harvester_engine::TriageArticleDelta::full_window(
                articles
                    .iter()
                    .filter(|a| {
                        a.fetched_utc
                            .as_deref()
                            .and_then(|raw| chrono::DateTime::parse_from_rfc3339(raw).ok())
                            .is_none_or(|date| date >= since)
                    })
                    .cloned()
                    .collect(),
                100_000,
            ),
        },
    )
    .0
}
fn fixture() -> AppState {
    let articles: Vec<_> = ["high", "low", "stale", "old-summary", "raw"]
        .into_iter()
        .map(article)
        .collect();
    let mut state = update(
        AppState::new(),
        Msg::RestoreCompletedJobs(
            articles
                .iter()
                .map(|a| CompletedJobSnapshot {
                    url: a.url.clone(),
                    fetched_utc: a.fetched_utc.clone(),
                    tokens: Some(100),
                    bytes: Some(1000),
                    links: vec![],
                })
                .collect(),
        ),
    )
    .0;
    state = update(
        state,
        Msg::RestoreDesktopView {
            mode: None,
            selected_article_url: None,
            now: now(),
        },
    )
    .0;
    state = update(
        state,
        Msg::BriefingCheckpointLoaded {
            since_utc: Some("2026-10-02T00:00:00Z".into()),
        },
    )
    .0;
    state = update(state, metadata(1)).0;
    state = update(
        state,
        Msg::PromptContextsLoaded {
            contexts: HashMap::new(),
        },
    )
    .0;
    let mut triages = TriageCache::new();
    let mut summaries = SummaryCache::new();
    for (name, priority, version) in [
        ("high", 5, 1),
        ("low", 2, 1),
        ("stale", 5, 0),
        ("old-summary", 4, 1),
        ("raw", 3, 1),
    ] {
        triages.insert_entry(
            TriageCacheKey::try_new(
                &format!("hash-{name}"),
                PromptId::ArticleTriage,
                Some(version),
                Some("gpt-5.4-mini"),
                &[],
            )
            .unwrap(),
            crate::TriageCacheEntry {
                result: triage(priority),
                created_at_utc: "2026-10-03T10:00:00Z".into(),
            },
        );
    }
    for (name, version, tokens) in [
        ("high", 1, 20),
        ("low", 1, 10),
        ("stale", 0, 50),
        ("old-summary", 0, 40),
    ] {
        summaries.insert(
            SummaryCacheKey::try_new(
                &format!("hash-{name}"),
                PromptId::ArticleSummary,
                Some(version),
                Some("summary-model"),
                &[],
            )
            .unwrap(),
            SummaryCacheEntry {
                result: summary(name, tokens),
                created_at_utc: "2026-10-03T10:00:00Z".into(),
            },
        );
    }
    state = update(state, Msg::TriageCacheHydrated { cache: triages }).0;
    state = update(state, Msg::SummaryCacheHydrated { cache: summaries }).0;
    state = loaded(state, &articles);
    let mut scores = SignalCandidateCache::default();
    for (name, value) in [("high", 90), ("low", 30)] {
        let a = article(name);
        let key = state.current_summary_cache_key(&a.content_hash).unwrap();
        let triage = state
            .triage_cache()
            .lookup(&state.current_triage_cache_key(&a.content_hash).unwrap())
            .unwrap()
            .1;
        let key = crate::update::signal_candidate::input_key_for_current_results(
            &state,
            &a,
            triage,
            &key,
            state.try_reuse_summary(&key).unwrap(),
        )
        .unwrap();
        scores.insert(
            key,
            crate::SignalCandidateCacheEntry {
                result: score(name, value),
                created_at_utc: "2026-10-03T10:00:00Z".into(),
            },
        );
    }
    update(state, Msg::SignalCandidateCacheLoaded { cache: scores }).0
}
fn export(state: AppState, candidates: bool) -> Effect {
    let (state, _) = update(state, Msg::ArchiveClicked);
    let id = state.archive_request_id();
    update(
        state,
        Msg::ArchiveDialogSubmitted {
            request_id: id,
            basename: "archive.md".into(),
            set_checkpoint: false,
            submitted_at: now(),
            use_summaries: true,
            use_signal_candidates: candidates,
        },
    )
    .1
    .into_iter()
    .find(|effect| matches!(effect, Effect::ArchiveRequested { .. }))
    .unwrap()
}

#[test]
fn restart_without_run_restores_current_priority_order_summary_results_and_meter() {
    let state = fixture();
    assert!(state.run_progress().is_none());
    assert_eq!(state.triage().completed_count(), 0);
    let view = state.view();
    assert_eq!(
        view.desktop_job_list
            .rows
            .iter()
            .map(|row| row.url.clone())
            .collect::<Vec<_>>(),
        ["high", "old-summary", "raw", "low", "stale"].map(|name| article(name).url)
    );
    assert_eq!(view.signal_candidate_rows.len(), 2);
    assert_eq!(
        state.startup_readiness(),
        crate::StartupReadinessStatus::Ready
    );
    assert_eq!(view.archive_meter.selected_count, 1);
    assert_eq!(view.archive_meter.token_estimate, 20);
    assert_eq!(view.archive_meter.status, crate::ArchiveMeterStatus::Scored);
    assert_eq!(view.archive_meter.unsettled_count, 3);
    assert!(view.archive_enabled);
    let id = view.desktop_job_list.rows[0].job_id;
    let state = update(state, Msg::JobSelected { job_id: id }).0;
    assert!(state
        .view()
        .right_pane
        .summary_markdown
        .unwrap()
        .contains("high summary"));
    assert_eq!(
        update(
            state,
            Msg::JobListModeSet {
                mode: JobListMode::Results
            }
        )
        .0
        .view()
        .signal_candidate_rows
        .len(),
        2
    );
}

#[test]
fn restart_stale_keys_are_invisible_to_rows_reading_selection_and_annotations() {
    let state = fixture();
    let row = state
        .view()
        .desktop_job_list
        .rows
        .into_iter()
        .find(|row| row.url == article("stale").url)
        .unwrap();
    assert!(row.triage_annotation.is_none());
    assert!(!row.has_summary);
    let state = update(state, Msg::JobSelected { job_id: row.job_id }).0;
    assert!(state.view().right_pane.summary_markdown.is_none());
    if let Effect::ArchiveRequested {
        ordered_urls,
        priority_snapshot,
        annotations,
        ..
    } = export(state, false)
    {
        let key = harvester_engine::archive_url_key(&article("stale").url);
        assert!(!ordered_urls.contains(&article("stale").url));
        assert!(!priority_snapshot.contains_key(&key));
        assert!(!annotations.contains_key(&key));
    } else {
        unreachable!()
    }
}

#[test]
fn restart_last_24_hours_retains_results_before_checkpoint_without_processing_or_archiving() {
    let mut state = fixture();
    state = update(
        state,
        Msg::BriefingCheckpointLoaded {
            since_utc: Some("2026-10-03T10:00:00Z".into()),
        },
    )
    .0;
    state = loaded(
        state,
        &[
            article("high"),
            article("low"),
            article("stale"),
            article("old-summary"),
            article("raw"),
        ],
    );
    assert!(state.view().desktop_job_list.rows.is_empty());
    assert!(state.archive_corpus().is_empty());
    assert_eq!(
        state.unfinished_work().clone(),
        crate::UnfinishedWork::Known(Default::default())
    );
    state = update(
        state,
        Msg::JobListModeSet {
            mode: JobListMode::Last24Hours,
        },
    )
    .0;
    let view = state.view();
    assert_eq!(view.desktop_job_list.rows.len(), 5);
    assert_eq!(
        view.desktop_job_list.rows[0]
            .triage_annotation
            .as_ref()
            .unwrap()
            .priority,
        5
    );
    state = update(
        state,
        Msg::JobSelected {
            job_id: view.desktop_job_list.rows[0].job_id,
        },
    )
    .0;
    assert!(state
        .view()
        .right_pane
        .summary_markdown
        .unwrap()
        .contains("high summary"));
    assert!(state.archive_final_selection().ordered_urls.is_empty());
}

#[test]
fn saved_index_checkpoint_changes_restrict_selection_and_priority_snapshot() {
    let state = update(
        fixture(),
        Msg::BriefingCheckpointSet(Some("2026-10-03T10:00:00Z".into())),
    )
    .0;
    if let Effect::ArchiveRequested {
        ordered_urls,
        priority_snapshot,
        ..
    } = export(state, false)
    {
        assert!(ordered_urls.is_empty());
        assert!(priority_snapshot.is_empty());
    } else {
        unreachable!()
    }
}

#[test]
fn saved_index_expires_articles_outside_window_when_last_24_hour_clock_moves() {
    let state = update(
        fixture(),
        Msg::BriefingCheckpointLoaded {
            since_utc: Some("2026-10-03T10:00:00Z".into()),
        },
    )
    .0;
    let state = update(state, Msg::tick_at(now() + chrono::Duration::hours(25))).0;
    let state = update(
        state,
        Msg::JobListModeSet {
            mode: JobListMode::Last24Hours,
        },
    )
    .0;
    assert!(state.view().desktop_job_list.rows.is_empty());
    assert!(state.view().signal_candidate_rows.is_empty());
}

#[test]
fn saved_completion_updates_current_summary_and_invalidates_scoring_upstream_digest() {
    let state = fixture();
    let key = state.current_summary_cache_key("hash-high").unwrap();
    let state = update(
        state,
        Msg::ValidatedResultReceived {
            record: Box::new(crate::SavedResult::Summary(
                key,
                SummaryCacheEntry {
                    result: summary("replacement", 70),
                    created_at_utc: "2026-10-03T11:00:00Z".into(),
                },
            )),
        },
    )
    .0;
    assert_eq!(
        state
            .current_summary_for_url(&article("high").url)
            .unwrap()
            .title,
        "replacement"
    );
    assert!(state
        .saved_results_for_url(&article("high").url)
        .unwrap()
        .signal
        .is_none());
    assert_eq!(state.view().signal_candidate_rows.len(), 1);
}

#[test]
fn frozen_run_configuration_rebuilds_saved_current_keys() {
    let (state, effects) = update(
        fixture(),
        Msg::PipelineRunRequested {
            scope: crate::PipelineRunScope::Resume,
        },
    );
    let id = effects
        .iter()
        .find_map(|e| match e {
            Effect::LoadProcessingConfiguration { request_id, .. } => Some(*request_id),
            _ => None,
        })
        .unwrap();
    let Msg::LlmMetadataLoaded {
        active_versions,
        effective_models,
    } = metadata(2)
    else {
        unreachable!()
    };
    let state = update(
        state,
        Msg::ProcessingConfigurationLoaded {
            request_id: id,
            contexts: HashMap::new(),
            active_versions,
            effective_models,
            preparation_budget: 100_000,
        },
    )
    .0;
    assert!(state
        .view()
        .desktop_job_list
        .rows
        .iter()
        .all(|row| row.triage_annotation.is_none() && !row.has_summary));
    assert!(state.view().signal_candidate_rows.is_empty());
}

#[test]
fn restart_summary_export_uses_stale_only_and_newest_any_key_but_reading_uses_current() {
    let mut state = fixture();
    let current = state
        .current_summary_for_url(&article("high").url)
        .unwrap()
        .clone();
    let key = SummaryCacheKey::try_new(
        "hash-high",
        PromptId::ArticleSummary,
        Some(0),
        Some("old-model"),
        &[],
    )
    .unwrap();
    state = update(
        state,
        Msg::ValidatedResultReceived {
            record: Box::new(crate::SavedResult::Summary(
                key,
                SummaryCacheEntry {
                    result: summary("newest stale", 80),
                    created_at_utc: "2026-10-03T11:00:00Z".into(),
                },
            )),
        },
    )
    .0;
    assert_eq!(
        state.current_summary_for_url(&article("high").url),
        Some(&current)
    );
    assert!(state
        .current_summary_for_url(&article("old-summary").url)
        .is_none());
    if let Effect::ArchiveRequested { summaries, .. } = export(state, false) {
        assert!(summaries[&article("high").url].contains("newest stale summary"));
        assert!(summaries[&article("old-summary").url].contains("old-summary summary"));
    } else {
        unreachable!()
    }
}

#[test]
fn restart_estimate_matches_post_run_summary_tokens_and_full_article_fallback() {
    let state = update(
        fixture(),
        Msg::SignalCandidateCacheLoaded {
            cache: SignalCandidateCache::default(),
        },
    )
    .0;
    let (_, effects) = update(state.clone(), Msg::ArchiveClicked);
    let estimates = effects
        .iter()
        .find_map(|e| match e {
            Effect::OpenArchiveDialog {
                token_estimates, ..
            } => Some(*token_estimates),
            _ => None,
        })
        .unwrap();
    assert_eq!(
        estimates,
        crate::ArchiveTokenEstimates {
            full_tokens: 400,
            summary_tokens: 170,
            summary_coverage: 3
        }
    );
    assert_eq!(
        state.startup_readiness(),
        crate::StartupReadinessStatus::Ready
    );
    assert_eq!(state.view().archive_meter.selected_count, 0);
    assert_eq!(state.view().archive_meter.token_estimate, 0);
    assert_eq!(
        state.view().archive_meter.status,
        crate::ArchiveMeterStatus::NotScoredYet
    );
    let mut post_run = state;
    let mut triages = crate::TriageSession::new_loading(None);
    triages.set_articles(["high", "low", "old-summary", "raw"].map(article).to_vec());
    triages.transition_to_triaging();
    for (i, priority) in [5, 2, 4, 3].into_iter().enumerate() {
        triages.complete_article(i, triage(priority));
    }
    triages.complete();
    post_run.set_triage(triages);
    let (_, effects) = update(post_run, Msg::ArchiveClicked);
    assert!(effects.iter().any(|e| matches!(e, Effect::OpenArchiveDialog { token_estimates, .. } if *token_estimates == estimates)));
}

#[test]
fn restart_any_key_lookup_preserves_live_session_summary_precedence() {
    let mut state = fixture();
    let url = article("old-summary").url;
    assert_eq!(
        state.summary_result_for_url(&url).unwrap().title,
        "old-summary"
    );
    let mut session = crate::briefing::BriefingSession::new_loading();
    session.set_articles(vec![article("old-summary")]);
    session.transition_to_summarizing();
    session.complete_article(0, summary("live", 90));
    state.set_briefing(session);
    assert_eq!(state.summary_result_for_url(&url).unwrap().title, "live");
    assert!(state.current_summary_for_url(&url).is_none());
}

#[test]
fn restart_archive_snapshot_uses_current_results_and_stored_compatible_alias_model() {
    let mut state = fixture();
    let key = TriageCacheKey::try_new(
        "hash-high",
        PromptId::ArticleTriage,
        Some(1),
        Some("gpt-5.4-mini-2026-03-17"),
        &[],
    )
    .unwrap();
    let mut cache = TriageCache::new();
    cache.insert_entry(
        key,
        crate::TriageCacheEntry {
            result: triage(5),
            created_at_utc: "2026-10-03T10:00:00Z".into(),
        },
    );
    state = update(state, Msg::TriageCacheHydrated { cache }).0;
    if let Effect::ArchiveRequested {
        annotations,
        priority_snapshot,
        ..
    } = export(state, false)
    {
        assert_eq!(priority_snapshot.len(), 1);
        assert_eq!(
            annotations[&article("high").url].triage_model.as_deref(),
            Some("gpt-5.4-mini-2026-03-17")
        );
        assert!(!priority_snapshot.contains_key(&article("stale").url));
    } else {
        unreachable!()
    }
}

#[test]
fn restart_archive_priority_population_includes_manually_excluded_current_results() {
    let mut state = fixture();
    let key = state
        .pre_triage()
        .entry_for_url(&article("low").url)
        .unwrap()
        .key
        .clone();
    state
        .pre_triage_mut()
        .set_manual_decision(&key, crate::ManualDecision::Exclude)
        .unwrap();
    let articles = state
        .pre_triage()
        .window_articles()
        .map(|(_, article)| article.clone())
        .collect();
    let request_id = state.alloc_triage_request_id();
    state.set_triage_in_flight(request_id);
    state = update(
        state,
        Msg::TriageArticlesLoaded {
            request_id,
            delta: harvester_engine::TriageArticleDelta::full_window(articles, 100_000),
        },
    )
    .0;
    assert!(
        !state
            .saved_results_for_url(&article("low").url)
            .unwrap()
            .actionable
    );
    if let Effect::ArchiveRequested {
        ordered_urls,
        priority_snapshot,
        ..
    } = export(state, false)
    {
        assert!(!ordered_urls.contains(&article("low").url));
        assert_eq!(priority_snapshot[&article("low").url], 2);
    } else {
        unreachable!()
    }
}

#[test]
fn remembered_tab_and_article_restore_after_hydration_without_run() {
    let state = fixture();
    let high = state.view().desktop_job_list.rows[0].job_id;
    let state = update(
        state,
        Msg::JobListModeSet {
            mode: JobListMode::Results,
        },
    )
    .0;
    let state = update(state, Msg::JobSelected { job_id: high }).0;
    let saved = crate::PersistenceSnapshot::capture(&state);
    let state = update(
        fixture(),
        Msg::RestoreDesktopView {
            mode: saved.job_list_mode,
            selected_article_url: saved.selected_article_url,
            now: now(),
        },
    )
    .0;
    assert_eq!(state.job_list_mode(), JobListMode::Results);
    assert_eq!(state.selected_job_id(), Some(high));
    assert!(state.view().right_pane.summary_markdown.is_some());
    assert!(state.run_progress().is_none());
}

#[test]
fn restored_article_outside_tab_is_dropped_and_view_changes_persist_only_on_change() {
    let state = update(
        fixture(),
        Msg::RestoreDesktopView {
            mode: Some(JobListMode::Results),
            selected_article_url: Some(article("raw").url),
            now: now(),
        },
    )
    .0;
    assert!(state.selected_job_id().is_none());
    let (state, effects) = update(
        state,
        Msg::JobListModeSet {
            mode: JobListMode::Last24Hours,
        },
    );
    assert!(effects.iter().any(|e| matches!(e, Effect::PersistRuntimeState { snapshot } if snapshot.job_list_mode == Some(JobListMode::Last24Hours))));
    let id = state.view().desktop_job_list.rows[0].job_id;
    let (state, effects) = update(state, Msg::JobSelected { job_id: id });
    assert!(effects.iter().any(|e| matches!(e, Effect::PersistRuntimeState { snapshot } if snapshot.selected_article_url == Some(article("high").url))));
    assert!(!update(state, Msg::JobSelected { job_id: id })
        .1
        .iter()
        .any(|e| matches!(e, Effect::PersistRuntimeState { .. })));
}

#[test]
fn post_run_and_restart_emit_identical_saved_archive_requests() {
    let restarted = fixture();
    let mut post_run = restarted.clone();
    let mut live = crate::TriageSession::new_loading(None);
    live.set_articles(["high", "low", "old-summary", "raw"].map(article).to_vec());
    live.transition_to_triaging();
    for (i, priority) in [5, 2, 4, 3].into_iter().enumerate() {
        live.complete_article(i, triage(priority));
    }
    live.complete();
    post_run.set_triage(live);
    assert_eq!(export(post_run, false), export(restarted, false));
}

#[test]
fn undated_window_articles_keep_archive_selection_annotations_priority_and_coverage() {
    for fetched_utc in [None, Some("malformed timestamp")] {
        let mut a = article("high");
        a.fetched_utc = fetched_utc.map(str::to_owned);
        let state = update(
            fixture(),
            Msg::RestoreCompletedJobs(vec![CompletedJobSnapshot {
                url: a.url.clone(),
                fetched_utc: None,
                tokens: Some(100),
                bytes: Some(1000),
                links: vec![],
            }]),
        )
        .0;
        let mut state = loaded(state, std::slice::from_ref(&a));
        assert_eq!(state.view().archive_meter.selected_count, 0);
        assert_eq!(
            state.view().archive_meter.status,
            crate::ArchiveMeterStatus::NotScoredYet
        );
        // Changing the frontmatter invalidates the scoring input; score the undated
        // article under its actual current key before asserting selection membership.
        let key = state
            .saved_results_for_url(&a.url)
            .unwrap()
            .signal_key
            .clone()
            .unwrap();
        state.store_signal_candidate_result(key, score("high", 90), "2026-10-03T11:00:00Z".into());
        assert!(state.briefing_since_utc().is_some());
        assert_eq!(state.archive_corpus().ordered_urls(), &[a.url.clone()]);
        assert_eq!(
            state.startup_readiness(),
            crate::StartupReadinessStatus::Ready
        );
        assert_eq!(state.view().archive_meter.selected_count, 1);
        assert_eq!(
            state.view().archive_meter.status,
            crate::ArchiveMeterStatus::Scored
        );
        assert!(state.view().desktop_job_list.rows.is_empty());
        assert_eq!(state.view().desktop_job_list.hidden_without_fetch_time, 1);
        let restored = update(
            state.clone(),
            Msg::RestoreDesktopView {
                mode: Some(JobListMode::SinceCheckpoint),
                selected_article_url: Some(a.url.clone()),
                now: now(),
            },
        )
        .0;
        assert!(restored.selected_job_id().is_none());
        assert!(restored.remembered_article_url().is_none());
        let recent = update(
            state.clone(),
            Msg::JobListModeSet {
                mode: JobListMode::Last24Hours,
            },
        )
        .0;
        assert!(recent.view().desktop_job_list.rows.is_empty());
        if let Effect::ArchiveRequested {
            ordered_urls,
            priority_snapshot,
            annotations,
            ..
        } = export(state, false)
        {
            assert_eq!(ordered_urls, [a.url.clone()]);
            assert_eq!(priority_snapshot[&a.url], 5);
            assert!(annotations.contains_key(&a.url));
        } else {
            unreachable!()
        }
    }
}

#[test]
fn run_drops_pending_desktop_restore_before_late_startup_hydration() {
    let state = update(fixture(), Msg::StartupHydrationRequested).0;
    let state = update(
        state,
        Msg::RestoreDesktopView {
            mode: Some(JobListMode::SinceCheckpoint),
            selected_article_url: Some(article("high").url),
            now: now(),
        },
    )
    .0;
    assert!(state.selected_job_id().is_none());
    assert!(state.remembered_article_url().is_some());
    let state = update(
        state,
        Msg::PipelineRunRequested {
            scope: crate::PipelineRunScope::Resume,
        },
    )
    .0;
    assert!(state.run_progress_is_active());
    assert!(state.remembered_article_url().is_none());
    let (state, effects) = update(state, metadata(1));
    assert!(!effects
        .iter()
        .any(|effect| matches!(effect, Effect::LoadArticleLinks { .. })));
    let (state, effects) = update(
        state,
        Msg::PromptContextsLoaded {
            contexts: HashMap::new(),
        },
    );
    assert!(state.run_progress_is_active());
    assert!(state.selected_job_id().is_none());
    assert!(state.remembered_article_url().is_none());
    assert!(!effects
        .iter()
        .any(|effect| matches!(effect, Effect::LoadArticleLinks { .. })));
    assert!(crate::PersistenceSnapshot::capture(&state)
        .selected_article_url
        .is_none());
}

#[test]
fn saved_summary_writes_preserve_any_key_newest_ties_growth_and_replacement() {
    let mut state = fixture();
    // Repeated hashes exercise ties and older writes, while unrelated hashes
    // force store growth. Overwrites include replacing a newest key with older data.
    for i in 0..80 {
        let hash = if i % 2 == 0 {
            "hash-high".to_owned()
        } else {
            format!("other-{i}")
        };
        let key = SummaryCacheKey::try_new(
            &hash,
            PromptId::ArticleSummary,
            Some((i % 7) as u32),
            Some("old-model"),
            &[],
        )
        .unwrap();
        state = update(
            state,
            Msg::ValidatedResultReceived {
                record: Box::new(crate::SavedResult::Summary(
                    key,
                    SummaryCacheEntry {
                        result: summary(&format!("write-{i}"), 80),
                        created_at_utc: format!("2026-10-03T{:02}:00:00Z", 9 + i % 3),
                    },
                )),
            },
        )
        .0;
        if hash == "hash-high" {
            assert_eq!(
                state.newest_summary_for_url(&article("high").url),
                state
                    .summary_cache()
                    .lookup_any_by_content_hash("hash-high")
                    .map(|entry| &entry.result),
                "write {i}"
            );
        }
    }
}
