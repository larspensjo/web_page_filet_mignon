use super::*;

/// Seed the persisted triage cache with a completed result for each URL, keyed
/// under the current metadata snapshot. Mirrors what a prior triage run would
/// have persisted. Requires triage metadata to be primed first.
fn seed_triage_cache_for_urls(state: &mut AppState, urls: &[&str], priority: u8) {
    for url in urls {
        state.store_triage_result(
            &format!("hash-{url}"),
            crate::triage::ArticleTriageResult {
                category: "tech".to_string(),
                priority,
                tags: vec![],
                rationale: "cached".to_string(),
                input_tokens: 0,
                output_tokens: 0,
            },
        );
    }
}

#[test]
fn startup_triage_coverage_does_not_fall_back_into_meter_selection() {
    init_logging();
    // Reproduces the startup state: completed jobs restored, pre-triage rebuilt to
    // ReadyToTriage, triage metadata + a fully-covering triage cache hydrated — but
    // the user has NOT clicked Run Triage, so the live TriageSession is still Idle.
    let urls = &["https://startup.com/1", "https://startup.com/2"];
    let mut state = ready_pre_triage_state(urls);
    state = prime_llm_metadata(state);
    seed_triage_cache_for_urls(&mut state, urls, 3);

    let state = crate::fixture_support::complete_archive_meter_startup(state);
    let view = state.view();
    assert_eq!(view.archive_meter.selected_count, 0);
    assert_eq!(
        view.archive_meter.status,
        crate::ArchiveMeterStatus::NotScoredYet
    );
    assert_eq!(view.archive_meter.unsettled_count, 2);
    assert_eq!(view.archive_partial_coverage, None);
    let (_, effects) = update(state, Msg::ArchiveClicked);
    assert!(effects.iter().any(|e| matches!(
        e,
        Effect::OpenArchiveDialog {
            article_count: 2,
            ..
        }
    )));
}

#[test]
fn cache_derived_archive_estimates_use_pre_triage_content_hash_for_summaries() {
    use crate::briefing::ArticleSummaryResult;
    use crate::summary_cache::SummaryCacheKey;
    use harvester_engine::llm::dto::SummaryEntities;
    use harvester_engine::llm::prompt::PromptId;

    init_logging();
    let url = "https://startup-summary.example/1";
    let mut state = ready_pre_triage_state(&[url]);
    state = prime_llm_metadata(state);
    seed_triage_cache_for_urls(&mut state, &[url], 3);
    state.store_summary_result(
        SummaryCacheKey {
            content_hash: format!("hash-{url}"),
            prompt_id: PromptId::ArticleSummary,
            prompt_version: 1,
            model_id: "summary-model".to_string(),
            context_hash: "ctx".to_string(),
        },
        ArticleSummaryResult {
            title: "Cached".to_string(),
            summary: "summary".to_string(),
            key_points: vec![],
            input_tokens: 1,
            output_tokens: 42,
            entities: SummaryEntities::default(),
        },
        "2026-07-12T00:00:00Z".to_string(),
    );

    let estimates = state.archive_token_estimates(&[url.to_string()]);
    assert_eq!(estimates.summary_coverage, 1);
    assert_eq!(estimates.summary_tokens, 42);
    let state = crate::fixture_support::complete_archive_meter_startup(state);
    assert_eq!(state.view().archive_meter.selected_count, 0);
    assert_eq!(state.view().archive_meter.token_estimate, 0);
    assert_eq!(
        state.view().archive_meter.status,
        crate::ArchiveMeterStatus::NotScoredYet
    );
    let (_, effects) = update(state, Msg::ArchiveClicked);
    assert!(effects.iter().any(|e| matches!(e, Effect::OpenArchiveDialog { token_estimates, .. } if *token_estimates == estimates)));
}

#[test]
fn saved_startup_counts_reach_full_archive_export() {
    init_logging();
    let urls = &[
        "https://startup-export.example/1",
        "https://startup-export.example/2",
    ];
    let mut state = ready_pre_triage_state(urls);
    state = prime_llm_metadata(state);
    seed_triage_cache_for_urls(&mut state, urls, 3);

    let state = crate::fixture_support::complete_archive_meter_startup(state);
    assert_eq!(state.view().archive_meter.selected_count, 0);
    assert_eq!(
        state.view().archive_meter.status,
        crate::ArchiveMeterStatus::NotScoredYet
    );
    let (state, effects) = update(state, Msg::ArchiveClicked);
    let open = effects
        .iter()
        .find_map(|effect| match effect {
            Effect::OpenArchiveDialog { article_count, .. } => Some(*article_count),
            _ => None,
        })
        .expect("OpenArchiveDialog effect expected");
    assert_eq!(open, 2);
    assert!(matches!(
        state.pinned_archive_corpus().map(|corpus| corpus.source()),
        Some(crate::CurrentWorkingCorpusSource::TriageComplete)
    ));

    let request_id = state.archive_request_id();
    let (_, effects) = update(
        state,
        Msg::ArchiveDialogSubmitted {
            request_id,
            basename: "archive.md".to_string(),
            set_checkpoint: false,
            submitted_at: chrono::Utc::now(),
            use_summaries: false,
            use_signal_candidates: false,
        },
    );
    let ordered_urls = effects
        .into_iter()
        .find_map(|effect| match effect {
            Effect::ArchiveRequested { ordered_urls, .. } => Some(ordered_urls),
            _ => None,
        })
        .expect("ArchiveRequested effect expected");
    assert_eq!(
        ordered_urls,
        urls.iter().map(|url| url.to_string()).collect::<Vec<_>>()
    );
}

#[test]
fn saved_startup_counts_hide_signal_results_without_current_upstream_keys() {
    use harvester_engine::llm::dto::{Confidence, SignalCandidateResult, SourceTier};

    init_logging();
    let urls = &["https://startup-signal.example/1"];
    let mut state = ready_pre_triage_state(urls);
    state = prime_llm_metadata(state);
    seed_triage_cache_for_urls(&mut state, urls, 3);
    state
        .signal_candidate_mut()
        .enqueue(urls[0].to_string(), "fixture-input".to_string());
    state.signal_candidate_mut().mark_scoring(urls[0], 1);
    state.signal_candidate_mut().complete(
        urls[0],
        SignalCandidateResult {
            signal_score: 90,
            signal_key: "cached-cluster".to_string(),
            themes: vec![],
            draft_gist: "cached".to_string(),
            source_tier: SourceTier::Tier1,
            confidence: Confidence::High,
            reasoning: "cached".to_string(),
            input_tokens: 0,
            output_tokens: 0,
        },
    );

    let state = crate::fixture_support::complete_archive_meter_startup(state);
    assert_eq!(state.view().archive_meter.selected_count, 0);
    assert_eq!(
        state.view().archive_meter.status,
        crate::ArchiveMeterStatus::NotScoredYet
    );
    let (state, effects) = update(state, Msg::ArchiveClicked);
    let (signal_candidate_count, signal_candidate_default) = effects
        .iter()
        .find_map(|effect| match effect {
            Effect::OpenArchiveDialog {
                signal_candidate_count,
                signal_candidate_default,
                ..
            } => Some((*signal_candidate_count, *signal_candidate_default)),
            _ => None,
        })
        .expect("OpenArchiveDialog effect expected");
    assert_eq!(signal_candidate_count, 0);
    assert!(matches!(
        signal_candidate_default,
        crate::SignalCandidateDialogDefault::OffDisabled
    ));

    let request_id = state.archive_request_id();
    let (_, effects) = update(
        state,
        Msg::ArchiveDialogSubmitted {
            request_id,
            basename: "archive.md".to_string(),
            set_checkpoint: false,
            submitted_at: chrono::Utc::now(),
            use_summaries: false,
            use_signal_candidates: true,
        },
    );
    let ordered_urls = effects
        .into_iter()
        .find_map(|effect| match effect {
            Effect::ArchiveRequested { ordered_urls, .. } => Some(ordered_urls),
            _ => None,
        })
        .expect("ArchiveRequested effect expected");
    assert!(ordered_urls.is_empty());
}

#[test]
fn cache_derived_archive_counts_do_not_mutate_the_live_triage_session() {
    init_logging();
    // The cache-derived archive corpus is a display-only projection. It must not
    // touch the load-bearing TriageSession, so the explicit Resume run still has
    // unfinished triage to admit after the display-only projection.
    let urls = &["https://startup.com/1", "https://startup.com/2"];
    let mut state = ready_pre_triage_state(urls);
    state = prime_llm_metadata(state);
    seed_triage_cache_for_urls(&mut state, urls, 3);

    // Force the archive corpus to be computed (this is what the view does).
    let _ = state.view();

    assert!(
        matches!(state.triage().phase(), crate::triage::TriagePhase::Idle),
        "live triage session must remain Idle; the derived corpus is display-only"
    );
    let (state, effects) = update(
        state,
        Msg::PipelineRunRequested {
            scope: crate::PipelineRunScope::Resume,
        },
    );
    assert_eq!(state.triage().completed_count(), 2);
    assert!(
        effects.iter().any(|effect| matches!(
            effect,
            crate::Effect::RequestLlmCompletion {
                prompt_id: harvester_engine::llm::prompt::PromptId::ArticleSummary,
                ..
            }
        )),
        "the Resume run releases cached triage to summaries"
    );
}

#[test]
fn cache_derived_archive_corpus_counts_the_covered_subset_under_partial_coverage() {
    init_logging();
    // Only one of two ready articles has a cached triage result — the common real
    // case, where some articles were triaged under a superseded prompt/model or are
    // newly polled. Dialog coverage keeps the triaged subset; the meter stays unscored.
    let urls = &["https://partial.com/1", "https://partial.com/2"];
    let mut state = ready_pre_triage_state(urls);
    state = prime_llm_metadata(state);
    seed_triage_cache_for_urls(&mut state, &urls[..1], 3);

    let state = crate::fixture_support::complete_archive_meter_startup(state);
    let view = state.view();
    assert_eq!(
        view.archive_partial_coverage,
        Some(crate::ArchivePartialCoverageView {
            triaged: 1,
            actionable_total: 2
        })
    );
    assert_eq!(view.archive_meter.selected_count, 0);
    assert_eq!(
        view.archive_meter.status,
        crate::ArchiveMeterStatus::NotScoredYet
    );
    assert_eq!(view.archive_meter.unsettled_count, 2);
    let (_, effects) = update(state, Msg::ArchiveClicked);
    assert!(effects.iter().any(|e| matches!(
        e,
        Effect::OpenArchiveDialog {
            article_count: 1,
            ..
        }
    )));
}

#[test]
fn cache_derived_meter_applies_exclusions_to_saved_selection() {
    use harvester_engine::llm::dto::{Confidence, SignalCandidateResult, SourceTier};

    init_logging();
    let urls = &[
        "https://cache-view.example/1",
        "https://cache-view.example/2",
    ];
    let mut state = ready_pre_triage_state(urls);
    state = prime_llm_metadata(state);
    seed_triage_cache_for_urls(&mut state, urls, 3);
    state
        .signal_candidate_mut()
        .enqueue(urls[0].to_string(), "fixture-input".to_string());
    state.signal_candidate_mut().mark_scoring(urls[0], 1);
    state.signal_candidate_mut().complete(
        urls[0],
        SignalCandidateResult {
            signal_score: 90,
            signal_key: "cache-view-cluster".to_string(),
            themes: vec![],
            draft_gist: "cached".to_string(),
            source_tier: SourceTier::Tier1,
            confidence: Confidence::High,
            reasoning: "cached".to_string(),
            input_tokens: 0,
            output_tokens: 0,
        },
    );

    crate::fixture_support::save_session_results(&mut state);
    let state = crate::fixture_support::complete_archive_meter_startup(state);
    assert_eq!(state.view().archive_meter.selected_count, 1);
    assert_eq!(
        state.view().archive_meter.status,
        crate::ArchiveMeterStatus::Scored
    );
    let (state, _) = update(
        state,
        Msg::ToggleSignalCandidateExclusion {
            signal_key: "cache-view-cluster".into(),
        },
    );
    assert_eq!(state.view().archive_meter.selected_count, 0);
    assert_eq!(state.view().archive_meter.token_estimate, 0);
    assert_eq!(
        state.view().archive_meter.status,
        crate::ArchiveMeterStatus::Scored
    );
    assert_eq!(state.view().archive_partial_coverage, None);
    let (_, effects) = update(state, Msg::ArchiveClicked);
    assert!(effects.iter().any(|e| matches!(
        e,
        Effect::OpenArchiveDialog {
            article_count: 2,
            signal_candidate_count: 0,
            ..
        }
    )));
}

#[test]
fn cache_derived_archive_counts_populate_while_pre_triage_is_reviewing() {
    init_logging();
    // Real corpora with borderline-length articles land pre-triage in `Reviewing`,
    // not `ReadyToTriage`. The archive counts must still derive from the cache for
    // the tentatively-included corpus, mirroring that triage can start from review.
    let review_content: String = std::iter::repeat_n("longword", 100)
        .collect::<Vec<_>>()
        .join(" ");
    let url1 = "https://review-derive.com/1";
    let url2 = "https://review-derive.com/2";
    let state = add_completed_job_for_test(AppState::new(), url1);
    let state = add_completed_job_for_test(state, url2);
    let (state, request_id) = tick_until_dispatch(state);
    let articles = vec![
        LoadedArticle {
            url: url1.to_string(),
            source_title: None,
            prepared_text: review_content.clone(),
            content_hash: format!("hash-{url1}"),
            fetched_utc: None,
        },
        LoadedArticle {
            url: url2.to_string(),
            source_title: None,
            prepared_text: review_content,
            content_hash: format!("hash-{url2}"),
            fetched_utc: None,
        },
    ];
    let (mut state, _) = update(
        state,
        Msg::TriageArticlesLoaded {
            request_id,
            delta: harvester_engine::TriageArticleDelta::full_window(articles, 100_000),
        },
    );
    // Manual resolution no longer exists, so both review articles remain
    // tentatively included and the phase stays Reviewing.
    assert!(
        matches!(
            state.pre_triage().phase(),
            crate::pre_triage_filter::PreTriagePhase::Reviewing
        ),
        "test setup must leave pre-triage in Reviewing"
    );

    state = prime_llm_metadata(state);
    seed_triage_cache_for_urls(&mut state, &[url1, url2], 3);

    let state = crate::fixture_support::complete_archive_meter_startup(state);
    assert_eq!(state.view().archive_partial_coverage, None);
    assert_eq!(state.view().archive_meter.selected_count, 0);
    assert_eq!(
        state.view().archive_meter.status,
        crate::ArchiveMeterStatus::NotScoredYet
    );
    let (state, effects) = update(state, Msg::ArchiveClicked);
    assert!(effects.iter().any(|e| matches!(
        e,
        Effect::OpenArchiveDialog {
            article_count: 2,
            ..
        }
    )));
    assert!(
        matches!(state.triage().phase(), crate::triage::TriagePhase::Idle),
        "live triage session must stay Idle — the derived corpus is display-only"
    );
}
