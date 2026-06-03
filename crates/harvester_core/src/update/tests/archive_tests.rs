use super::*;

#[test]
fn pre_triage_refresh_dispatch_includes_briefing_checkpoint_since_utc() {
    init_logging();
    let since = chrono::DateTime::parse_from_rfc3339("2026-03-21T18:17:00Z")
        .unwrap()
        .with_timezone(&chrono::Utc);
    let mut state = add_completed_job_for_test(AppState::new(), "https://example.com/1");
    state.set_briefing_since_utc(Some(since));

    for _ in 0..200 {
        let (next, effects) = update(state, Msg::Tick);
        state = next;
        if let Some(effect_since_utc) = effects.iter().find_map(|e| match e {
            Effect::LoadArticlesForTriage { since_utc, .. } => *since_utc,
            _ => None,
        }) {
            assert_eq!(effect_since_utc, since);
            return;
        }
    }

    panic!("no LoadArticlesForTriage dispatch within 200 ticks");
}

#[test]
fn archive_clicked_emits_open_dialog_with_request_id_and_article_count() {
    init_logging();
    let state = add_completed_job_for_test(AppState::new(), "https://example.com/1");
    let (state, effects) = update(state, Msg::ArchiveClicked);
    assert_eq!(state.archive_request_id(), 1);
    let effect = effects
        .into_iter()
        .find(|effect| matches!(effect, Effect::OpenArchiveDialog { .. }))
        .expect("OpenArchiveDialog effect expected");
    match effect {
        Effect::OpenArchiveDialog {
            request_id,
            article_count,
            since_utc,
            default_basename,
            ..
        } => {
            assert_eq!(request_id, 1);
            assert_eq!(article_count, 0);
            assert!(since_utc.is_none());
            assert_eq!(default_basename, "archive.md");
        }
        _ => unreachable!(),
    }
}

#[test]
fn archive_clicked_with_triage_complete_and_pre_triage_ready_sets_pending_count() {
    init_logging();
    let state = complete_triage_state_for_test(1);
    let url = "https://pending.com/1";
    let state = add_completed_job_for_test(state, url);
    let (state, request_id) = tick_until_dispatch(state);
    let (state, _) = update(
        state,
        Msg::TriageArticlesLoaded {
            request_id,
            articles: loaded_pre_triage_articles(&[url]),
        },
    );
    assert_eq!(
        state.current_working_corpus().source(),
        crate::working_corpus::CurrentWorkingCorpusSource::PreTriageReady,
        "working corpus should be PreTriageReady — archive corpus is different"
    );

    let (_, effects) = update(state, Msg::ArchiveClicked);
    let (article_count, pending_count) = effects
        .iter()
        .find_map(|e| {
            if let Effect::OpenArchiveDialog {
                article_count,
                pending_pre_triage_count,
                ..
            } = e
            {
                Some((*article_count, *pending_pre_triage_count))
            } else {
                None
            }
        })
        .expect("expected OpenArchiveDialog effect");
    assert_eq!(
        article_count, 1,
        "archive must use TriageComplete (1 article), not pre-triage"
    );
    assert!(
        pending_count > 0,
        "pending_pre_triage_count must be > 0, got {}",
        pending_count
    );
}

#[test]
fn archive_clicked_with_only_pre_triage_ready_has_zero_article_count() {
    init_logging();
    let state = ready_pre_triage_state(&["https://example.com/a", "https://example.com/b"]);
    let (_, effects) = update(state, Msg::ArchiveClicked);
    let (article_count, pending_count) = effects
        .iter()
        .find_map(|e| {
            if let Effect::OpenArchiveDialog {
                article_count,
                pending_pre_triage_count,
                ..
            } = e
            {
                Some((*article_count, *pending_pre_triage_count))
            } else {
                None
            }
        })
        .expect("expected OpenArchiveDialog effect");
    assert_eq!(
        article_count, 0,
        "no triage done → archive article_count must be 0"
    );
    assert_eq!(
        pending_count, 2,
        "both pre-triage articles must appear in pending count"
    );
}

#[test]
fn archive_clicked_with_triage_complete_and_no_pre_triage_has_zero_pending_count() {
    init_logging();
    let state = complete_triage_state_for_test(2);
    let (_, effects) = update(state, Msg::ArchiveClicked);
    let (article_count, pending_count) = effects
        .iter()
        .find_map(|e| {
            if let Effect::OpenArchiveDialog {
                article_count,
                pending_pre_triage_count,
                ..
            } = e
            {
                Some((*article_count, *pending_pre_triage_count))
            } else {
                None
            }
        })
        .expect("expected OpenArchiveDialog effect");
    assert_eq!(
        article_count, 2,
        "TriageComplete corpus must supply 2 articles"
    );
    assert_eq!(
        pending_count, 0,
        "no pre-triage ready → pending count must be 0"
    );
}

#[test]
fn archive_token_estimates_uses_summary_output_tokens_when_available() {
    use crate::briefing::ArticleSummaryResult;
    use crate::summary_cache::SummaryCacheKey;
    use harvester_engine::llm::dto::SummaryEntities;
    use harvester_engine::llm::prompt::PromptId;

    init_logging();
    let url = "https://triage-complete.com/0".to_string();
    let state = complete_triage_state_for_test(1);
    let mut state = add_completed_job_with_tokens_for_test(state, &url, 500);
    let key = SummaryCacheKey {
        content_hash: "hash-tc-0".to_string(),
        prompt_id: PromptId::ArticleSummary,
        prompt_version: 4,
        model_id: "claude-sonnet".to_string(),
        context_hash: "ctx".to_string(),
    };
    let result = ArticleSummaryResult {
        title: "Art".to_string(),
        summary: "summary text".to_string(),
        key_points: vec![],
        input_tokens: 100,
        output_tokens: 42,
        entities: SummaryEntities::default(),
    };
    state.store_summary_result(key, result, "2026-04-01T00:00:00Z".to_string());

    let estimates = state.archive_token_estimates(&[url]);

    assert_eq!(estimates.full_tokens, 500);
    assert_eq!(estimates.summary_tokens, 42);
    assert_eq!(estimates.summary_coverage, 1);
}

#[test]
fn view_job_rows_include_cached_summary_tokens() {
    use crate::briefing::ArticleSummaryResult;
    use crate::summary_cache::SummaryCacheKey;
    use harvester_engine::llm::dto::SummaryEntities;
    use harvester_engine::llm::prompt::PromptId;

    init_logging();
    let url = "https://triage-complete.com/0".to_string();
    let state = complete_triage_state_for_test(1);
    let mut state = add_completed_job_with_tokens_for_test(state, &url, 500);
    state.store_summary_result(
        SummaryCacheKey {
            content_hash: "hash-tc-0".to_string(),
            prompt_id: PromptId::ArticleSummary,
            prompt_version: 4,
            model_id: "model".to_string(),
            context_hash: "ctx".to_string(),
        },
        ArticleSummaryResult {
            title: "Art".to_string(),
            summary: "summary text".to_string(),
            key_points: vec![],
            input_tokens: 100,
            output_tokens: 42,
            entities: SummaryEntities::default(),
        },
        "2026-04-01T00:00:00Z".to_string(),
    );

    let view = state.view();
    let row = view
        .jobs
        .iter()
        .find(|job| job.url == url)
        .expect("expected matching job row");

    assert_eq!(row.tokens, Some(500));
    assert_eq!(row.summary_tokens, Some(42));
    assert!(row.has_summary);
}

#[test]
fn view_job_rows_use_pre_triage_content_hash_for_cached_summary_tokens() {
    use crate::briefing::{ArticleSummaryResult, LoadedArticle};
    use crate::pre_triage_filter::{PreTriagePolicy, PreTriageSession};
    use crate::summary_cache::SummaryCacheKey;
    use harvester_engine::llm::dto::SummaryEntities;
    use harvester_engine::llm::prompt::PromptId;

    init_logging();
    let url = "https://startup.example.com/a".to_string();
    let state = add_completed_job_with_tokens_for_test(AppState::new(), &url, 500);
    let mut state = state;
    let pre_triage = PreTriageSession::load_articles(
        vec![LoadedArticle {
            url: url.clone(),
            source_title: None,
            prepared_text: std::iter::repeat_n("startup", 220)
                .collect::<Vec<_>>()
                .join(" "),
            content_hash: "startup-hash".to_string(),
            fetched_utc: None,
        }],
        &PreTriagePolicy::default(),
    );
    state.set_pre_triage(pre_triage);
    state.store_summary_result(
        SummaryCacheKey {
            content_hash: "startup-hash".to_string(),
            prompt_id: PromptId::ArticleSummary,
            prompt_version: 4,
            model_id: "model".to_string(),
            context_hash: "ctx".to_string(),
        },
        ArticleSummaryResult {
            title: "Startup".to_string(),
            summary: "summary text".to_string(),
            key_points: vec![],
            input_tokens: 100,
            output_tokens: 42,
            entities: SummaryEntities::default(),
        },
        "2026-04-01T00:00:00Z".to_string(),
    );

    let view = state.view();
    let row = view
        .jobs
        .iter()
        .find(|job| job.url == url)
        .expect("expected matching job row");

    assert_eq!(row.tokens, Some(500));
    assert_eq!(row.summary_tokens, Some(42));
    assert!(row.has_summary);
}

#[test]
fn archive_token_estimates_falls_back_to_full_tokens_when_no_summary() {
    init_logging();
    let url = "https://triage-complete.com/0".to_string();
    let state = complete_triage_state_for_test(1);
    let state = add_completed_job_with_tokens_for_test(state, &url, 300);

    let estimates = state.archive_token_estimates(&[url]);

    assert_eq!(estimates.full_tokens, 300);
    assert_eq!(estimates.summary_tokens, 300);
    assert_eq!(estimates.summary_coverage, 0);
}

#[test]
fn archive_clicked_emits_real_token_estimates_from_state() {
    use crate::briefing::ArticleSummaryResult;
    use crate::summary_cache::SummaryCacheKey;
    use harvester_engine::llm::dto::SummaryEntities;
    use harvester_engine::llm::prompt::PromptId;

    init_logging();
    let url = "https://triage-complete.com/0".to_string();
    let state = complete_triage_state_for_test(1);
    let mut state = add_completed_job_with_tokens_for_test(state, &url, 400);
    state.store_summary_result(
        SummaryCacheKey {
            content_hash: "hash-tc-0".to_string(),
            prompt_id: PromptId::ArticleSummary,
            prompt_version: 4,
            model_id: "model".to_string(),
            context_hash: "ctx".to_string(),
        },
        ArticleSummaryResult {
            title: "Art".to_string(),
            summary: "s".to_string(),
            key_points: vec![],
            input_tokens: 10,
            output_tokens: 99,
            entities: SummaryEntities::default(),
        },
        "2026-04-01T00:00:00Z".to_string(),
    );

    let (_, effects) = update(state, Msg::ArchiveClicked);
    let estimates = effects
        .iter()
        .find_map(|effect| match effect {
            Effect::OpenArchiveDialog {
                token_estimates, ..
            } => Some(*token_estimates),
            _ => None,
        })
        .expect("OpenArchiveDialog expected");

    assert_eq!(estimates.full_tokens, 400);
    assert_eq!(estimates.summary_tokens, 99);
    assert_eq!(estimates.summary_coverage, 1);
}

fn complete_all_triage_llm_requests(mut state: AppState, effects: Vec<Effect>) -> AppState {
    let request_ids: Vec<u64> = effects
        .iter()
        .filter_map(|e| match e {
            Effect::RequestLlmCompletion { request_id, .. } => Some(*request_id),
            _ => None,
        })
        .collect();
    for rid in request_ids {
        let (next, _) = update(state, triage_success(rid));
        state = next;
    }
    state
}

#[test]
fn consume_interactive_pre_triage_articles_for_triage_rejects_non_interactive_phase() {
    init_logging();
    let mut state = AppState::new();
    let result = state.consume_interactive_pre_triage_articles_for_triage();
    assert!(result.is_none(), "Idle phase must return None");
    assert!(
        matches!(
            state.pre_triage().phase(),
            crate::pre_triage_filter::PreTriagePhase::Idle
        ),
        "phase must remain Idle after failed consume"
    );
}

#[test]
fn consume_interactive_pre_triage_articles_for_triage_returns_articles_and_resets_to_idle() {
    init_logging();
    let urls = &["https://example.com/a", "https://example.com/b"];
    let mut state = ready_pre_triage_state(urls);
    assert!(matches!(
        state.pre_triage().phase(),
        crate::pre_triage_filter::PreTriagePhase::ReadyToTriage
    ));

    let articles = state.consume_interactive_pre_triage_articles_for_triage();

    let articles = articles.expect("should return Some in ReadyToTriage with articles");
    assert_eq!(
        articles.len(),
        urls.len(),
        "should return all resolved articles"
    );

    assert!(
        matches!(
            state.pre_triage().phase(),
            crate::pre_triage_filter::PreTriagePhase::Idle
        ),
        "phase must be Idle after consuming"
    );

    assert!(
        state.pre_triage().resolved_included_urls().is_empty(),
        "no URLs should remain in pre-triage after consuming"
    );
}

#[test]
fn triage_clicked_consumes_reviewing_pre_triage_into_triage_session() {
    init_logging();
    let review_content: String = std::iter::repeat_n("longword", 100)
        .collect::<Vec<_>>()
        .join(" ");
    let url1 = "https://review-handoff.com/1";
    let url2 = "https://review-handoff.com/2";
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
    let (state, _) = update(
        state,
        Msg::TriageArticlesLoaded {
            request_id,
            articles,
        },
    );
    let key = state.pre_triage().entries()[0].key.clone();
    let (state, _) = update(
        state,
        Msg::PreTriageDecisionSet {
            key,
            decision: crate::pre_triage_filter::ManualDecision::Exclude,
        },
    );
    assert!(
        matches!(
            state.pre_triage().phase(),
            crate::pre_triage_filter::PreTriagePhase::Reviewing
        ),
        "one unresolved review item should keep pre-triage in Reviewing"
    );
    assert!(
        state.view().triage_can_start,
        "Reviewing phase with tentative included articles must allow triage start"
    );

    let state = prime_llm_metadata(state);
    let (state, effects) = update(state, Msg::TriageClicked);

    assert!(!effects.is_empty(), "triage should dispatch from Reviewing");
    assert!(
        matches!(
            state.pre_triage().phase(),
            crate::pre_triage_filter::PreTriagePhase::Idle
        ),
        "pre-triage must reset to Idle after TriageClicked"
    );
    assert_eq!(
        state.triage().articles().len(),
        1,
        "only the tentatively included article should be handed to triage"
    );
    assert_eq!(state.triage().articles()[0].url, url2);
}

#[test]
fn triage_clicked_consumes_ready_pre_triage_into_triage_session() {
    init_logging();
    let urls = &["https://handoff.com/1", "https://handoff.com/2"];
    let state = ready_pre_triage_state(urls);
    let state = prime_llm_metadata(state);

    assert!(matches!(
        state.pre_triage().phase(),
        crate::pre_triage_filter::PreTriagePhase::ReadyToTriage
    ));

    let (state, _effects) = update(state, Msg::TriageClicked);

    assert!(
        matches!(
            state.pre_triage().phase(),
            crate::pre_triage_filter::PreTriagePhase::Idle
        ),
        "pre-triage must reset to Idle after TriageClicked"
    );
    assert!(
        state.pre_triage().resolved_included_urls().is_empty(),
        "pre-triage must have no resolved URLs after consume"
    );

    assert_eq!(
        state.triage().articles().len(),
        urls.len(),
        "triage session must hold all pre-triage articles"
    );
    let triage_urls: Vec<&str> = state
        .triage()
        .articles()
        .iter()
        .map(|a| a.url.as_str())
        .collect();
    for url in urls {
        assert!(
            triage_urls.contains(url),
            "triage session must contain pre-triage URL {url}"
        );
    }
}

#[test]
fn triage_clicked_sets_current_working_corpus_to_unavailable_until_triage_completes() {
    init_logging();
    let urls = &["https://corpus-src.com/1"];
    let state = ready_pre_triage_state(urls);
    let state = prime_llm_metadata(state);

    assert_eq!(
        state.current_working_corpus().source(),
        crate::working_corpus::CurrentWorkingCorpusSource::PreTriageReady,
        "source must be PreTriageReady before TriageClicked"
    );

    let (state, effects) = update(state, Msg::TriageClicked);
    assert_eq!(
        state.current_working_corpus().source(),
        crate::working_corpus::CurrentWorkingCorpusSource::Unavailable,
        "source must be Unavailable while triage is in-flight"
    );

    let state = complete_all_triage_llm_requests(state, effects);

    assert_eq!(
        state.current_working_corpus().source(),
        crate::working_corpus::CurrentWorkingCorpusSource::TriageComplete,
        "source must be TriageComplete after triage finishes"
    );
}

#[test]
fn archive_clicked_after_triage_start_has_zero_pending_pre_triage_count() {
    init_logging();
    let urls = &["https://archive-handoff.com/1"];
    let state = ready_pre_triage_state(urls);
    let state = prime_llm_metadata(state);

    let (state, effects) = update(state, Msg::TriageClicked);
    let state = complete_all_triage_llm_requests(state, effects);

    let (_, archive_effects) = update(state, Msg::ArchiveClicked);
    let pending_count = archive_effects
        .iter()
        .find_map(|e| {
            if let Effect::OpenArchiveDialog {
                pending_pre_triage_count,
                ..
            } = e
            {
                Some(*pending_pre_triage_count)
            } else {
                None
            }
        })
        .expect("expected OpenArchiveDialog effect");

    assert_eq!(
        pending_count, 0,
        "pending_pre_triage_count must be 0 after reducer handoff path"
    );
}

#[test]
fn pre_triage_refresh_after_triage_start_repopulates_pre_triage_without_mutating_active_triage() {
    init_logging();
    let urls = &["https://repopulate.com/1"];
    let state = ready_pre_triage_state(urls);
    let state = prime_llm_metadata(state);

    let (state, _effects) = update(state, Msg::TriageClicked);
    assert!(
        matches!(
            state.pre_triage().phase(),
            crate::pre_triage_filter::PreTriagePhase::Idle
        ),
        "pre-triage must be Idle after TriageClicked"
    );

    let triage_article_count_before = state.triage().articles().len();
    let triage_phase_before = state.triage().phase().clone();

    let new_url = "https://repopulate.com/new-article";
    let state = add_completed_job_for_test(state, new_url);
    let (state, request_id) = tick_until_dispatch(state);
    let new_articles = loaded_pre_triage_articles(&[new_url]);
    let (state, _) = update(
        state,
        Msg::TriageArticlesLoaded {
            request_id,
            articles: new_articles,
        },
    );

    assert_eq!(
        state.triage().articles().len(),
        triage_article_count_before,
        "triage session article count must not change after pre-triage refresh"
    );
    assert_eq!(
        state.triage().phase(),
        &triage_phase_before,
        "triage session phase must not change after pre-triage refresh"
    );

    assert!(
        matches!(
            state.pre_triage().phase(),
            crate::pre_triage_filter::PreTriagePhase::ReadyToTriage
        ),
        "pre-triage must be ReadyToTriage after new refresh"
    );
    assert!(
        state
            .pre_triage()
            .resolved_included_urls()
            .iter()
            .any(|u| u == new_url),
        "pre-triage must contain the newly refreshed article"
    );
}

#[test]
fn archive_clicked_with_pre_triage_reviewing_has_zero_pending_count() {
    init_logging();
    let state = complete_triage_state_for_test(1);
    let review_content: String = std::iter::repeat_n("longword", 100)
        .collect::<Vec<_>>()
        .join(" ");
    let url1 = "https://review.com/1";
    let url2 = "https://review.com/2";
    let state = add_completed_job_for_test(state, url1);
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
    let (state, _) = update(
        state,
        Msg::TriageArticlesLoaded {
            request_id,
            articles,
        },
    );
    assert!(
        matches!(
            state.pre_triage().phase(),
            crate::pre_triage_filter::PreTriagePhase::Reviewing
        ),
        "unresolved review articles should derive Reviewing immediately after load"
    );
    let key = state.pre_triage().entries()[0].key.clone();
    let (state, _) = update(
        state,
        Msg::PreTriageDecisionSet {
            key,
            decision: crate::pre_triage_filter::ManualDecision::Include,
        },
    );
    assert!(
        matches!(
            state.pre_triage().phase(),
            crate::pre_triage_filter::PreTriagePhase::Reviewing
        ),
        "must be Reviewing after first decision with second still unresolved"
    );

    let (_, effects) = update(state, Msg::ArchiveClicked);
    let pending_count = effects
        .iter()
        .find_map(|e| {
            if let Effect::OpenArchiveDialog {
                pending_pre_triage_count,
                ..
            } = e
            {
                Some(*pending_pre_triage_count)
            } else {
                None
            }
        })
        .expect("expected OpenArchiveDialog effect");
    assert_eq!(
        pending_count, 0,
        "Reviewing phase → resolved_included_urls() is empty → pending count must be 0"
    );
}

#[test]
fn archive_dialog_ready_ignores_stale_request() {
    init_logging();
    let state = AppState::new();
    let (state, effects) = update(
        state,
        Msg::ArchiveDialogReady {
            request_id: 99,
            article_count: 0,
            since_utc: None,
            default_basename: "archive.md".to_string(),
            default_file_exists: false,
            export_dir: std::path::PathBuf::from("/tmp"),
            pending_pre_triage_count: 0,
            token_estimates: crate::ArchiveTokenEstimates::default(),
            signal_candidate_default:
                crate::signal_candidate::SignalCandidateDialogDefault::OffDisabled,
            signal_candidate_count: 0,
            signal_candidate_scoring_done: 0,
            signal_candidate_scoring_total: 0,
            signal_candidate_token_estimates: crate::ArchiveTokenEstimates::default(),
        },
    );
    assert!(effects.is_empty());
    assert_eq!(state.archive_request_id(), 0);
}

#[test]
fn archive_dialog_ready_emits_show_dialog_for_current_request() {
    init_logging();
    let state = add_completed_job_for_test(AppState::new(), "https://example.com/1");
    let (state, _) = update(state, Msg::ArchiveClicked);
    let request_id = state.archive_request_id();
    let (state, effects) = update(
        state,
        Msg::ArchiveDialogReady {
            request_id,
            article_count: 1,
            since_utc: None,
            default_basename: "archive.md".to_string(),
            default_file_exists: false,
            export_dir: std::path::PathBuf::from("/tmp"),
            pending_pre_triage_count: 0,
            token_estimates: crate::ArchiveTokenEstimates::default(),
            signal_candidate_default:
                crate::signal_candidate::SignalCandidateDialogDefault::OffDisabled,
            signal_candidate_count: 0,
            signal_candidate_scoring_done: 0,
            signal_candidate_scoring_total: 0,
            signal_candidate_token_estimates: crate::ArchiveTokenEstimates::default(),
        },
    );
    assert_eq!(state.archive_request_id(), 1);
    let effect = effects
        .into_iter()
        .find(|effect| matches!(effect, Effect::ShowArchiveDialog { .. }))
        .expect("ShowArchiveDialog effect expected");
    match effect {
        Effect::ShowArchiveDialog {
            request_id,
            article_count,
            since_utc,
            default_basename,
            default_file_exists,
            export_dir,
            ..
        } => {
            assert_eq!(request_id, 1);
            assert_eq!(article_count, 1);
            assert!(since_utc.is_none());
            assert_eq!(default_basename, "archive.md");
            assert!(!default_file_exists);
            assert_eq!(export_dir, std::path::PathBuf::from("/tmp"));
        }
        _ => unreachable!(),
    }
}

#[test]
fn archive_dialog_submitted_validates_basename_and_checkpoint_flag() {
    init_logging();
    let since = chrono::DateTime::parse_from_rfc3339("2026-03-21T18:17:00Z")
        .unwrap()
        .with_timezone(&chrono::Utc);
    let state = add_completed_job_for_test(AppState::new(), "https://example.com/1");
    let (state, _) = update(state, Msg::ArchiveClicked);
    let request_id = state.archive_request_id();

    let (_state, effects) = update(
        state,
        Msg::ArchiveDialogSubmitted {
            request_id,
            basename: "custom-archive.md".to_string(),
            set_checkpoint: true,
            submitted_at: since,
            use_summaries: false,
            use_signal_candidates: false,
        },
    );
    let effect = effects
        .into_iter()
        .find(|effect| matches!(effect, Effect::ArchiveRequested { .. }))
        .expect("ArchiveRequested effect expected");
    match effect {
        Effect::ArchiveRequested {
            request_id,
            basename,
            ordered_urls,
            since_utc,
            requested_checkpoint,
            use_summaries,
            summaries,
        } => {
            assert_eq!(request_id, 1);
            assert_eq!(basename, "custom-archive.md");
            assert_eq!(ordered_urls.len(), 0);
            assert!(since_utc.is_none());
            assert_eq!(requested_checkpoint, Some(since));
            assert!(!use_summaries);
            assert!(summaries.is_empty());
        }
        _ => unreachable!(),
    }
}

#[test]
fn archive_dialog_submitted_with_only_pre_triage_ready_exports_zero_urls() {
    init_logging();
    let since = chrono::DateTime::parse_from_rfc3339("2026-03-21T18:17:00Z")
        .unwrap()
        .with_timezone(&chrono::Utc);
    let state = ready_pre_triage_state(&["https://example.com/1"]);
    let (state, _) = update(state, Msg::ArchiveClicked);
    let request_id = state.archive_request_id();

    let (_state, effects) = update(
        state,
        Msg::ArchiveDialogSubmitted {
            request_id,
            basename: "archive.md".to_string(),
            set_checkpoint: false,
            submitted_at: since,
            use_summaries: false,
            use_signal_candidates: false,
        },
    );
    let effect = effects
        .into_iter()
        .find(|effect| matches!(effect, Effect::ArchiveRequested { .. }))
        .expect("ArchiveRequested effect expected");
    match effect {
        Effect::ArchiveRequested {
            ordered_urls,
            since_utc,
            requested_checkpoint,
            ..
        } => {
            assert_eq!(ordered_urls, Vec::<String>::new());
            assert!(since_utc.is_none());
            assert_eq!(requested_checkpoint, None);
        }
        _ => unreachable!(),
    }
}

#[test]
fn archive_submitted_with_use_summaries_true_and_cached_summary_populates_map() {
    use crate::briefing::ArticleSummaryResult;
    use crate::summary_cache::SummaryCacheKey;
    use harvester_engine::archive_url_key;
    use harvester_engine::llm::dto::SummaryEntities;
    use harvester_engine::llm::prompt::PromptId;

    init_logging();
    let since = chrono::DateTime::parse_from_rfc3339("2026-03-21T18:17:00Z")
        .unwrap()
        .with_timezone(&chrono::Utc);
    let url = "https://triage-complete.com/0".to_string();
    let mut state = complete_triage_state_for_test(1);
    state.store_summary_result(
        SummaryCacheKey {
            content_hash: "hash-tc-0".to_string(),
            prompt_id: PromptId::ArticleSummary,
            prompt_version: 4,
            model_id: "model".to_string(),
            context_hash: "ctx".to_string(),
        },
        ArticleSummaryResult {
            title: "Art".to_string(),
            summary: "compact".to_string(),
            key_points: vec!["point one".to_string()],
            input_tokens: 10,
            output_tokens: 5,
            entities: SummaryEntities::default(),
        },
        "2026-04-01T00:00:00Z".to_string(),
    );

    let (state, _) = update(state, Msg::ArchiveClicked);
    let request_id = state.archive_request_id();
    let (_state, effects) = update(
        state,
        Msg::ArchiveDialogSubmitted {
            request_id,
            basename: "archive.md".to_string(),
            set_checkpoint: false,
            submitted_at: since,
            use_summaries: true,
            use_signal_candidates: false,
        },
    );
    let effect = effects
        .into_iter()
        .find(|effect| matches!(effect, Effect::ArchiveRequested { .. }))
        .expect("ArchiveRequested effect expected");
    match effect {
        Effect::ArchiveRequested {
            use_summaries,
            summaries,
            ..
        } => {
            assert!(use_summaries);
            let body = summaries
                .get(&archive_url_key(&url))
                .expect("summary body for url");
            assert!(body.contains("## Summary\ncompact"));
            assert!(body.contains("## Key Points\n- point one"));
        }
        _ => unreachable!(),
    }
}

#[test]
fn archive_submitted_with_use_summaries_false_emits_empty_summary_map() {
    init_logging();
    let since = chrono::DateTime::parse_from_rfc3339("2026-03-21T18:17:00Z")
        .unwrap()
        .with_timezone(&chrono::Utc);
    let state = complete_triage_state_for_test(1);
    let (state, _) = update(state, Msg::ArchiveClicked);
    let request_id = state.archive_request_id();

    let (_state, effects) = update(
        state,
        Msg::ArchiveDialogSubmitted {
            request_id,
            basename: "archive.md".to_string(),
            set_checkpoint: false,
            submitted_at: since,
            use_summaries: false,
            use_signal_candidates: false,
        },
    );
    let effect = effects
        .into_iter()
        .find(|effect| matches!(effect, Effect::ArchiveRequested { .. }))
        .expect("ArchiveRequested effect expected");
    match effect {
        Effect::ArchiveRequested {
            use_summaries,
            summaries,
            ..
        } => {
            assert!(!use_summaries);
            assert!(summaries.is_empty());
        }
        _ => unreachable!(),
    }
}

#[test]
fn archive_dialog_submitted_rejects_invalid_basename() {
    init_logging();
    let state = add_completed_job_for_test(AppState::new(), "https://example.com/1");
    let (state, _) = update(state, Msg::ArchiveClicked);
    let (_, effects) = update(
        state,
        Msg::ArchiveDialogSubmitted {
            request_id: 1,
            basename: "../bad.md".to_string(),
            set_checkpoint: true,
            submitted_at: chrono::Utc::now(),
            use_summaries: false,
            use_signal_candidates: false,
        },
    );
    assert!(effects.is_empty());
}

#[test]
fn archive_export_completed_only_saves_checkpoint_for_current_request() {
    init_logging();
    let state = add_completed_job_for_test(AppState::new(), "https://example.com/1");
    let (state, _) = update(state, Msg::ArchiveClicked);
    let checkpoint = chrono::DateTime::parse_from_rfc3339("2026-03-22T00:00:00Z")
        .unwrap()
        .with_timezone(&chrono::Utc);
    let (state, effects) = update(
        state,
        Msg::ArchiveExportCompleted {
            request_id: 1,
            path: std::path::PathBuf::from("archive.md"),
            doc_count: 1,
            requested_checkpoint: Some(checkpoint),
        },
    );
    assert_eq!(
        effects,
        vec![Effect::SaveBriefingCheckpoint {
            save_id: 1,
            since_utc: Some(checkpoint)
        }]
    );
    assert_eq!(state.briefing_since_utc(), Some(checkpoint));
    assert_eq!(
        state.briefing_checkpoint_status_message(),
        Some("Checkpoint saving...")
    );
}

#[test]
fn briefing_checkpoint_save_success_clears_pending_status() {
    init_logging();
    let checkpoint = chrono::DateTime::parse_from_rfc3339("2026-03-22T00:00:00Z")
        .unwrap()
        .with_timezone(&chrono::Utc);
    let (state, effects) = update(
        AppState::new(),
        Msg::BriefingCheckpointSet(Some("2026-03-22T00:00:00Z".to_string())),
    );
    assert_eq!(
        effects,
        vec![Effect::SaveBriefingCheckpoint {
            save_id: 1,
            since_utc: Some(checkpoint)
        }]
    );
    assert_eq!(
        state.pending_briefing_checkpoint_save(),
        Some(crate::state::PendingBriefingCheckpointSaveSnapshot {
            save_id: 1,
            previous_since_utc: None,
            pending_since_utc: Some(checkpoint),
        })
    );

    let (state, follow_up) = update(state, Msg::BriefingCheckpointSaveSucceeded { save_id: 1 });
    assert!(follow_up.is_empty());
    assert_eq!(state.briefing_since_utc(), Some(checkpoint));
    assert_eq!(state.pending_briefing_checkpoint_save(), None);
    assert_eq!(state.briefing_checkpoint_status_message(), None);
}

#[test]
fn briefing_checkpoint_save_failure_reverts_in_memory_state() {
    init_logging();
    let initial = chrono::DateTime::parse_from_rfc3339("2026-03-20T00:00:00Z")
        .unwrap()
        .with_timezone(&chrono::Utc);
    let mut state = AppState::new();
    state.set_briefing_since_utc(Some(initial));
    let (state, effects) = update(
        state,
        Msg::BriefingCheckpointSet(Some("2026-03-22T00:00:00Z".to_string())),
    );
    assert!(matches!(
        effects.as_slice(),
        [Effect::SaveBriefingCheckpoint { save_id: 1, .. }]
    ));
    let (state, follow_up) = update(
        state,
        Msg::BriefingCheckpointSaveFailed {
            save_id: 1,
            reason: "disk full".to_string(),
        },
    );
    assert!(follow_up.is_empty());
    assert_eq!(state.briefing_since_utc(), Some(initial));
    assert_eq!(state.pending_briefing_checkpoint_save(), None);
    assert_eq!(
        state.briefing_checkpoint_status_message(),
        Some("Checkpoint save failed: disk full")
    );
}

#[test]
fn briefing_checkpoint_loaded_clears_pending_save_tracking() {
    init_logging();
    let checkpoint = chrono::DateTime::parse_from_rfc3339("2026-03-22T00:00:00Z")
        .unwrap()
        .with_timezone(&chrono::Utc);
    let (state, _) = update(
        AppState::new(),
        Msg::BriefingCheckpointSet(Some("2026-03-22T00:00:00Z".to_string())),
    );
    assert!(state.pending_briefing_checkpoint_save().is_some());

    let (state, follow_up) = update(
        state,
        Msg::BriefingCheckpointLoaded {
            since_utc: Some("2026-03-25T00:00:00Z".to_string()),
        },
    );
    assert!(follow_up.is_empty());
    assert_eq!(
        state.briefing_since_utc(),
        Some(
            chrono::DateTime::parse_from_rfc3339("2026-03-25T00:00:00Z")
                .unwrap()
                .with_timezone(&chrono::Utc)
        )
    );
    assert_eq!(state.pending_briefing_checkpoint_save(), None);
    assert_eq!(state.briefing_checkpoint_status_message(), None);
    assert_ne!(state.briefing_since_utc(), Some(checkpoint));
}

#[test]
fn stale_checkpoint_save_failure_is_ignored_when_newer_save_is_pending() {
    init_logging();
    let first_checkpoint = chrono::DateTime::parse_from_rfc3339("2026-03-20T00:00:00Z")
        .unwrap()
        .with_timezone(&chrono::Utc);
    let latest_checkpoint = chrono::DateTime::parse_from_rfc3339("2026-03-22T00:00:00Z")
        .unwrap()
        .with_timezone(&chrono::Utc);
    let (state, _) = update(
        AppState::new(),
        Msg::BriefingCheckpointSet(Some("2026-03-20T00:00:00Z".to_string())),
    );
    let (state, effects) = update(
        state,
        Msg::BriefingCheckpointSet(Some("2026-03-22T00:00:00Z".to_string())),
    );
    assert!(matches!(
        effects.as_slice(),
        [Effect::SaveBriefingCheckpoint { save_id: 2, .. }]
    ));

    let (state, follow_up) = update(
        state,
        Msg::BriefingCheckpointSaveFailed {
            save_id: 1,
            reason: "stale failure".to_string(),
        },
    );
    assert!(follow_up.is_empty());
    assert_eq!(state.briefing_since_utc(), Some(latest_checkpoint));
    assert_eq!(
        state.pending_briefing_checkpoint_save(),
        Some(crate::state::PendingBriefingCheckpointSaveSnapshot {
            save_id: 2,
            previous_since_utc: Some(first_checkpoint),
            pending_since_utc: Some(latest_checkpoint),
        })
    );
}

#[test]
fn archive_open_and_submit_use_identical_pinned_corpus() {
    init_logging();
    let since = chrono::DateTime::parse_from_rfc3339("2026-03-22T00:00:00Z")
        .unwrap()
        .with_timezone(&chrono::Utc);
    let state = complete_triage_state_for_test(2);

    let (state, open_effects) = update(state, Msg::ArchiveClicked);
    let request_id = state.archive_request_id();

    let open_effect = open_effects
        .into_iter()
        .find(|e| matches!(e, Effect::OpenArchiveDialog { .. }))
        .expect("OpenArchiveDialog effect expected");
    let open_count = match open_effect {
        Effect::OpenArchiveDialog { article_count, .. } => article_count,
        _ => unreachable!(),
    };
    assert_eq!(
        open_count, 2,
        "open dialog should report 2 pinned triage articles"
    );

    let (_state, submit_effects) = update(
        state,
        Msg::ArchiveDialogSubmitted {
            request_id,
            basename: "archive.md".to_string(),
            set_checkpoint: false,
            submitted_at: since,
            use_summaries: false,
            use_signal_candidates: false,
        },
    );
    let submit_effect = submit_effects
        .into_iter()
        .find(|e| matches!(e, Effect::ArchiveRequested { .. }))
        .expect("ArchiveRequested effect expected");
    match submit_effect {
        Effect::ArchiveRequested { ordered_urls, .. } => {
            assert_eq!(
                ordered_urls,
                vec![
                    "https://triage-complete.com/0".to_string(),
                    "https://triage-complete.com/1".to_string()
                ],
                "submitted export must use the pinned triage corpus"
            );
        }
        _ => unreachable!(),
    }
}

#[test]
fn refresh_between_open_and_submit_uses_pinned_snapshot() {
    init_logging();
    let since = chrono::DateTime::parse_from_rfc3339("2026-03-22T00:00:00Z")
        .unwrap()
        .with_timezone(&chrono::Utc);

    let triage_url = "https://triage-complete.com/0";
    let state = complete_triage_state_for_test(1);

    let (state, open_effects) = update(state, Msg::ArchiveClicked);
    let request_id = state.archive_request_id();
    let open_count = open_effects
        .into_iter()
        .find_map(|e| match e {
            Effect::OpenArchiveDialog { article_count, .. } => Some(article_count),
            _ => None,
        })
        .expect("OpenArchiveDialog effect expected");
    assert_eq!(open_count, 1, "open dialog must see 1 triage article");

    let pre_triage_url = "https://pretriage.com/1";
    let state = add_completed_job_for_test(state, pre_triage_url);
    let (state, request_id2) = tick_until_dispatch(state);
    let (state, _) = update(
        state,
        Msg::TriageArticlesLoaded {
            request_id: request_id2,
            articles: loaded_pre_triage_articles(&[pre_triage_url]),
        },
    );
    assert_eq!(
        state.archive_corpus().count(),
        1,
        "archive corpus must still see 1 triage article after pre-triage refresh"
    );

    let (_state, submit_effects) = update(
        state,
        Msg::ArchiveDialogSubmitted {
            request_id,
            basename: "archive.md".to_string(),
            set_checkpoint: false,
            submitted_at: since,
            use_summaries: false,
            use_signal_candidates: false,
        },
    );
    let submit_effect = submit_effects
        .into_iter()
        .find(|e| matches!(e, Effect::ArchiveRequested { .. }))
        .expect("ArchiveRequested effect expected");
    match submit_effect {
        Effect::ArchiveRequested { ordered_urls, .. } => {
            assert_eq!(
                ordered_urls,
                vec![triage_url.to_string()],
                "submit must use pinned triage corpus from open time, not pre-triage"
            );
        }
        _ => unreachable!(),
    }
}

#[test]
fn parity_a_pre_triage_ready_archive_count_is_zero_pending_count_is_nonzero() {
    init_logging();
    let urls = &["https://parity-a.com/1", "https://parity-a.com/2"];
    let state = ready_pre_triage_state(urls);
    let since = chrono::DateTime::parse_from_rfc3339("2026-03-22T00:00:00Z")
        .unwrap()
        .with_timezone(&chrono::Utc);

    let corpus = state.current_working_corpus();
    assert_eq!(
        corpus.source(),
        crate::working_corpus::CurrentWorkingCorpusSource::PreTriageReady,
        "source must be PreTriageReady"
    );

    let (state, open_effects) = update(state, Msg::ArchiveClicked);
    let request_id = state.archive_request_id();
    let (archive_count, pending_count) = open_effects
        .into_iter()
        .find_map(|e| match e {
            Effect::OpenArchiveDialog {
                article_count,
                pending_pre_triage_count,
                ..
            } => Some((article_count, pending_pre_triage_count)),
            _ => None,
        })
        .expect("OpenArchiveDialog effect expected");
    assert_eq!(
        archive_count, 0,
        "archive must be empty when only pre-triage is ready"
    );
    assert_eq!(
        pending_count,
        urls.len(),
        "pending count must equal the number of pre-triage ready articles"
    );

    let (_state, submit_effects) = update(
        state,
        Msg::ArchiveDialogSubmitted {
            request_id,
            basename: "archive.md".to_string(),
            set_checkpoint: false,
            submitted_at: since,
            use_summaries: false,
            use_signal_candidates: false,
        },
    );
    let submit_urls = submit_effects
        .into_iter()
        .find_map(|e| match e {
            Effect::ArchiveRequested { ordered_urls, .. } => Some(ordered_urls),
            _ => None,
        })
        .expect("ArchiveRequested effect expected");
    assert_eq!(
        submit_urls,
        Vec::<String>::new(),
        "submit must export zero URLs when only pre-triage is ready"
    );
}

#[test]
fn parity_b_triage_complete_corpus_count_dialog_count_urls_match() {
    init_logging();
    let state = complete_triage_state_for_test(2);
    let since = chrono::DateTime::parse_from_rfc3339("2026-03-22T00:00:00Z")
        .unwrap()
        .with_timezone(&chrono::Utc);

    let corpus = state.current_working_corpus();
    assert_eq!(
        corpus.source(),
        crate::working_corpus::CurrentWorkingCorpusSource::TriageComplete,
        "source must be TriageComplete when pre-triage is idle"
    );
    let expected_count = corpus.count();
    let expected_urls: Vec<String> = corpus.ordered_urls().to_vec();
    assert!(
        expected_count > 0,
        "corpus must be non-empty for a meaningful parity test"
    );

    let (state, open_effects) = update(state, Msg::ArchiveClicked);
    let request_id = state.archive_request_id();
    let dialog_count = open_effects
        .into_iter()
        .find_map(|e| match e {
            Effect::OpenArchiveDialog { article_count, .. } => Some(article_count),
            _ => None,
        })
        .expect("OpenArchiveDialog effect expected");
    assert_eq!(
        dialog_count, expected_count,
        "OpenArchiveDialog article_count must equal corpus.count()"
    );

    let (_state, submit_effects) = update(
        state,
        Msg::ArchiveDialogSubmitted {
            request_id,
            basename: "archive.md".to_string(),
            set_checkpoint: false,
            submitted_at: since,
            use_summaries: false,
            use_signal_candidates: false,
        },
    );
    let submit_urls = submit_effects
        .into_iter()
        .find_map(|e| match e {
            Effect::ArchiveRequested { ordered_urls, .. } => Some(ordered_urls),
            _ => None,
        })
        .expect("ArchiveRequested effect expected");
    assert_eq!(
        submit_urls.len(),
        expected_count,
        "ArchiveRequested url count must equal corpus.count()"
    );
    assert_eq!(
        submit_urls, expected_urls,
        "ArchiveRequested URLs must match corpus.ordered_urls()"
    );
}

#[test]
fn checkpoint_set_does_not_reduce_corpus_count_to_zero() {
    init_logging();
    let mut state = complete_triage_state_for_test(2);

    let checkpoint = chrono::DateTime::parse_from_rfc3339("2026-01-01T00:00:00Z")
        .unwrap()
        .with_timezone(&chrono::Utc);
    state.set_briefing_since_utc(Some(checkpoint));
    assert_eq!(
        state.briefing_since_utc(),
        Some(checkpoint),
        "checkpoint must be set before the test"
    );

    let corpus = state.current_working_corpus();
    assert_eq!(
        corpus.source(),
        crate::working_corpus::CurrentWorkingCorpusSource::TriageComplete,
        "corpus source must be TriageComplete"
    );
    assert!(
        corpus.count() > 0,
        "corpus count must be non-zero even when briefing checkpoint is set"
    );
    let expected_count = corpus.count();
    let expected_urls: Vec<String> = corpus.ordered_urls().to_vec();

    let (state, open_effects) = update(state, Msg::ArchiveClicked);
    let dialog_since_utc = open_effects
        .iter()
        .find_map(|e| match e {
            Effect::OpenArchiveDialog { since_utc, .. } => Some(*since_utc),
            _ => None,
        })
        .expect("OpenArchiveDialog effect expected");
    let dialog_count = open_effects
        .into_iter()
        .find_map(|e| match e {
            Effect::OpenArchiveDialog { article_count, .. } => Some(article_count),
            _ => None,
        })
        .expect("OpenArchiveDialog effect (article_count) expected");

    assert_eq!(
        dialog_since_utc,
        Some(checkpoint),
        "OpenArchiveDialog must propagate the briefing checkpoint as since_utc"
    );
    assert_eq!(
        dialog_count, expected_count,
        "OpenArchiveDialog article_count must equal corpus.count() — checkpoint must not reduce it"
    );
    assert!(
        dialog_count > 0,
        "article_count must be non-zero when checkpoint is set and triage has completed articles"
    );

    let request_id = state.archive_request_id();
    let submit_since = chrono::DateTime::parse_from_rfc3339("2026-03-22T00:00:00Z")
        .unwrap()
        .with_timezone(&chrono::Utc);
    let (_state, submit_effects) = update(
        state,
        Msg::ArchiveDialogSubmitted {
            request_id,
            basename: "archive.md".to_string(),
            set_checkpoint: false,
            submitted_at: submit_since,
            use_summaries: false,
            use_signal_candidates: false,
        },
    );
    let submit_urls = submit_effects
        .into_iter()
        .find_map(|e| match e {
            Effect::ArchiveRequested { ordered_urls, .. } => Some(ordered_urls),
            _ => None,
        })
        .expect("ArchiveRequested effect expected");
    assert_eq!(
        submit_urls.len(),
        expected_count,
        "ArchiveRequested url count must equal corpus.count() even with checkpoint set"
    );
    assert_eq!(
        submit_urls, expected_urls,
        "submit URLs must match corpus URLs even with checkpoint set"
    );
}

#[test]
fn archive_clicked_reports_signal_candidate_snapshot() {
    use harvester_engine::llm::dto::{Confidence, SignalCandidateResult, SourceTier};

    init_logging();
    let mut state = AppState::new();
    state
        .signal_candidate_mut()
        .enqueue("https://signal.example/a".to_string());
    state
        .signal_candidate_mut()
        .mark_scoring("https://signal.example/a", 7);
    state.signal_candidate_mut().complete(
        "https://signal.example/a",
        SignalCandidateResult {
            signal_score: 80,
            signal_key: "cluster-a".to_string(),
            themes: vec!["theme".to_string()],
            draft_gist: "gist".to_string(),
            source_tier: SourceTier::Tier1,
            confidence: Confidence::High,
            reasoning: "reason".to_string(),
            input_tokens: 10,
            output_tokens: 2,
        },
    );

    let (state, effects) = update(state, Msg::ArchiveClicked);
    let effect = effects
        .into_iter()
        .find(|e| matches!(e, Effect::OpenArchiveDialog { .. }))
        .expect("OpenArchiveDialog effect expected");
    match effect {
        Effect::OpenArchiveDialog {
            signal_candidate_default,
            signal_candidate_count,
            signal_candidate_scoring_done,
            signal_candidate_scoring_total,
            signal_candidate_token_estimates,
            ..
        } => {
            assert!(matches!(
                signal_candidate_default,
                crate::signal_candidate::SignalCandidateDialogDefault::OnAllSettled
            ));
            assert_eq!(signal_candidate_count, 1);
            assert_eq!(signal_candidate_scoring_done, 1);
            assert_eq!(signal_candidate_scoring_total, 1);
            assert_eq!(
                signal_candidate_token_estimates,
                crate::ArchiveTokenEstimates::default()
            );
        }
        _ => unreachable!(),
    }

    let pinned = state
        .pinned_signal_candidate_selection()
        .expect("signal candidate snapshot should be pinned at archive open");
    assert_eq!(
        pinned.selected_urls,
        vec!["https://signal.example/a".to_string()]
    );
}

#[test]
fn archive_dialog_submit_uses_pinned_signal_candidate_snapshot_and_clears_overrides() {
    use harvester_engine::llm::dto::{Confidence, SignalCandidateResult, SourceTier};

    init_logging();
    let mut state = AppState::new();
    state
        .signal_candidate_mut()
        .enqueue("https://signal.example/a".to_string());
    state
        .signal_candidate_mut()
        .mark_scoring("https://signal.example/a", 7);
    state.signal_candidate_mut().complete(
        "https://signal.example/a",
        SignalCandidateResult {
            signal_score: 80,
            signal_key: "cluster-a".to_string(),
            themes: vec!["theme".to_string()],
            draft_gist: "gist".to_string(),
            source_tier: SourceTier::Tier1,
            confidence: Confidence::High,
            reasoning: "reason".to_string(),
            input_tokens: 10,
            output_tokens: 2,
        },
    );

    let (state, _) = update(
        state,
        Msg::ToggleSignalCandidateExclusion {
            signal_key: "cluster-x".to_string(),
        },
    );
    let (mut state, _) = update(state, Msg::ArchiveClicked);

    state
        .signal_candidate_mut()
        .enqueue("https://signal.example/b".to_string());
    state
        .signal_candidate_mut()
        .mark_scoring("https://signal.example/b", 8);
    state.signal_candidate_mut().complete(
        "https://signal.example/b",
        SignalCandidateResult {
            signal_score: 90,
            signal_key: "cluster-b".to_string(),
            themes: vec!["theme".to_string()],
            draft_gist: "gist".to_string(),
            source_tier: SourceTier::Tier1,
            confidence: Confidence::High,
            reasoning: "reason".to_string(),
            input_tokens: 10,
            output_tokens: 2,
        },
    );

    let request_id = state.archive_request_id();
    let (state, effects) = update(
        state,
        Msg::ArchiveDialogSubmitted {
            request_id,
            basename: "archive.md".to_string(),
            set_checkpoint: true,
            submitted_at: chrono::Utc::now(),
            use_summaries: false,
            use_signal_candidates: true,
        },
    );

    let effect = effects
        .iter()
        .find(|e| matches!(e, Effect::ArchiveRequested { .. }))
        .expect("ArchiveRequested effect expected");
    match effect {
        Effect::ArchiveRequested { ordered_urls, .. } => {
            assert_eq!(
                *ordered_urls,
                vec!["https://signal.example/a".to_string()],
                "submit must use the pinned signal-candidate snapshot from archive-open time"
            );
        }
        _ => unreachable!(),
    }

    assert!(effects.iter().any(|e| matches!(
        e,
        Effect::PersistSignalCandidateOverrides { overrides } if overrides.is_empty()
    )));
    assert!(
        state.signal_candidate().excluded().is_empty(),
        "checkpoint submit must clear signal-candidate overrides"
    );
}

#[test]
fn archive_dialog_submit_with_empty_candidate_snapshot_exports_empty_selection_when_explicitly_enabled(
) {
    use harvester_engine::llm::dto::{Confidence, SignalCandidateResult, SourceTier};

    init_logging();
    let mut state = complete_triage_state_for_test(1);
    state
        .signal_candidate_mut()
        .enqueue("https://triage-complete.com/0".to_string());
    state
        .signal_candidate_mut()
        .mark_scoring("https://triage-complete.com/0", 7);
    state.signal_candidate_mut().complete(
        "https://triage-complete.com/0",
        SignalCandidateResult {
            signal_score: 20,
            signal_key: "below-threshold".to_string(),
            themes: vec!["theme".to_string()],
            draft_gist: "gist".to_string(),
            source_tier: SourceTier::Tier1,
            confidence: Confidence::High,
            reasoning: "reason".to_string(),
            input_tokens: 10,
            output_tokens: 2,
        },
    );

    let (state, _) = update(state, Msg::ArchiveClicked);
    let request_id = state.archive_request_id();
    let (_state, effects) = update(
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

    let archived = effects
        .into_iter()
        .find_map(|effect| match effect {
            Effect::ArchiveRequested { ordered_urls, .. } => Some(ordered_urls),
            _ => None,
        })
        .expect("ArchiveRequested effect expected");
    assert!(
        archived.is_empty(),
        "explicit candidate export should honor the empty pinned candidate selection"
    );
}

#[test]
fn archive_dialog_checkpoint_submit_skips_override_persist_when_overrides_already_empty() {
    init_logging();
    let state = complete_triage_state_for_test(1);
    let (state, _) = update(state, Msg::ArchiveClicked);
    let request_id = state.archive_request_id();

    let (_state, effects) = update(
        state,
        Msg::ArchiveDialogSubmitted {
            request_id,
            basename: "archive.md".to_string(),
            set_checkpoint: true,
            submitted_at: chrono::Utc::now(),
            use_summaries: false,
            use_signal_candidates: false,
        },
    );

    assert!(
        effects
            .iter()
            .all(|effect| !matches!(effect, Effect::PersistSignalCandidateOverrides { .. })),
        "checkpoint submit should not write an already-empty override set"
    );
}

#[test]
fn signal_candidate_selection_applies_threshold_and_order() {
    use harvester_engine::llm::dto::{Confidence, SignalCandidateResult, SourceTier};

    init_logging();
    let mut state = complete_triage_state_for_test(2);
    state = with_signal_candidate_metadata(state);

    for (i, score, key) in [(0usize, 80u8, "cluster-a"), (1usize, 30u8, "cluster-b")] {
        let url = format!("https://triage-complete.com/{i}");
        state.signal_candidate_mut().enqueue(url.clone());
        state
            .signal_candidate_mut()
            .mark_scoring(&url, i as u64 + 1);
        state.signal_candidate_mut().complete(
            &url,
            SignalCandidateResult {
                signal_score: score,
                signal_key: key.to_string(),
                themes: vec!["t".to_string()],
                draft_gist: "g".to_string(),
                source_tier: SourceTier::Tier1,
                confidence: Confidence::High,
                reasoning: "r".to_string(),
                input_tokens: 1,
                output_tokens: 1,
            },
        );
    }

    let selection = state.signal_candidate_selection();
    assert_eq!(
        selection.selected_urls,
        vec!["https://triage-complete.com/0".to_string()],
        "only the above-threshold article is selected"
    );
}

#[test]
fn archive_final_selection_signal_filtered_matches_shared_selection() {
    use crate::signal_candidate::ArchiveSelectionSource;
    use harvester_engine::llm::dto::{Confidence, SignalCandidateResult, SourceTier};

    init_logging();
    let mut state = complete_triage_state_for_test(2);
    state = with_signal_candidate_metadata(state);
    for (i, score, key) in [(0usize, 80u8, "cluster-a"), (1usize, 30u8, "cluster-b")] {
        let url = format!("https://triage-complete.com/{i}");
        state.signal_candidate_mut().enqueue(url.clone());
        state
            .signal_candidate_mut()
            .mark_scoring(&url, i as u64 + 1);
        state.signal_candidate_mut().complete(
            &url,
            SignalCandidateResult {
                signal_score: score,
                signal_key: key.to_string(),
                themes: vec!["t".to_string()],
                draft_gist: "g".to_string(),
                source_tier: SourceTier::Tier1,
                confidence: Confidence::High,
                reasoning: "r".to_string(),
                input_tokens: 1,
                output_tokens: 1,
            },
        );
    }

    let final_selection = state.archive_final_selection();
    assert_eq!(
        final_selection.source,
        ArchiveSelectionSource::SignalFiltered
    );
    assert_eq!(
        final_selection.ordered_urls,
        state.signal_candidate_selection().selected_urls,
        "archive_final_selection must use the shared selection compute"
    );
    assert_eq!(
        final_selection.ordered_urls,
        vec!["https://triage-complete.com/0".to_string()]
    );
}

#[test]
fn archive_final_selection_settled_empty_falls_back_to_full_corpus() {
    use crate::signal_candidate::ArchiveSelectionSource;
    use harvester_engine::llm::dto::{Confidence, SignalCandidateResult, SourceTier};

    init_logging();
    let mut state = complete_triage_state_for_test(2);
    state = with_signal_candidate_metadata(state);
    for i in 0..2usize {
        let url = format!("https://triage-complete.com/{i}");
        state.signal_candidate_mut().enqueue(url.clone());
        state
            .signal_candidate_mut()
            .mark_scoring(&url, i as u64 + 1);
        state.signal_candidate_mut().complete(
            &url,
            SignalCandidateResult {
                signal_score: 10,
                signal_key: format!("k{i}"),
                themes: vec!["t".to_string()],
                draft_gist: "g".to_string(),
                source_tier: SourceTier::Tier1,
                confidence: Confidence::High,
                reasoning: "r".to_string(),
                input_tokens: 1,
                output_tokens: 1,
            },
        );
    }

    let final_selection = state.archive_final_selection();
    assert_eq!(
        final_selection.source,
        ArchiveSelectionSource::FullCorpusNoCandidates
    );
    assert_eq!(
        final_selection.ordered_urls,
        state.archive_corpus().ordered_urls().to_vec()
    );
}

#[test]
fn archive_final_selection_no_candidates_falls_back_to_full_corpus() {
    use crate::signal_candidate::ArchiveSelectionSource;

    init_logging();
    let state = complete_triage_state_for_test(2);
    let final_selection = state.archive_final_selection();
    assert_eq!(
        final_selection.source,
        ArchiveSelectionSource::FullCorpusSignalUnavailable
    );
    assert_eq!(
        final_selection.ordered_urls,
        state.archive_corpus().ordered_urls().to_vec()
    );
}

#[test]
fn summary_failed_for_url_returns_true_for_failed_summary() {
    use crate::briefing::{BriefingSession, LoadedArticle};

    init_logging();
    let url = "https://triage-complete.com/0";
    let mut briefing = BriefingSession::new_loading(None);
    briefing.set_articles(
        vec![LoadedArticle {
            url: url.to_string(),
            source_title: None,
            prepared_text: "text".to_string(),
            content_hash: "hash-tc-0".to_string(),
            fetched_utc: None,
        }],
        "collection".to_string(),
    );
    briefing.transition_to_summarizing();
    briefing.start_article(0, 1);
    briefing.fail_article(0, "network".to_string());

    assert!(briefing.summary_failed_for_url(url));
    assert!(!briefing.summary_failed_for_url("https://triage-complete.com/1"));
}

#[test]
fn summaries_can_start_true_when_triage_complete_and_briefing_idle() {
    init_logging();
    let state = complete_triage_state_for_test(2);
    assert!(state.summaries_can_start());
}

#[test]
fn summaries_can_start_false_when_triage_not_complete() {
    init_logging();
    let state = AppState::new();
    assert!(!state.summaries_can_start());
}

#[test]
fn summaries_can_start_false_when_briefing_active() {
    use crate::briefing::BriefingSession;

    init_logging();
    let mut state = complete_triage_state_for_test(2);
    state.set_briefing(BriefingSession::new_loading(None));
    assert!(!state.briefing().can_start());
    assert!(!state.summaries_can_start());
}

#[test]
fn briefing_generate_readiness_triage_or_corpus_not_ready_when_empty() {
    use crate::state::BriefingGenerateReadiness;

    init_logging();
    let state = AppState::new();
    assert!(matches!(
        state.briefing_generate_readiness(),
        BriefingGenerateReadiness::TriageOrCorpusNotReady
    ));
}

#[test]
fn briefing_generate_readiness_summaries_not_settled() {
    use crate::state::BriefingGenerateReadiness;

    init_logging();
    let state = complete_triage_state_for_test(2);
    assert!(matches!(
        state.briefing_generate_readiness(),
        BriefingGenerateReadiness::SummariesNotSettled
    ));
}

#[test]
fn briefing_generate_readiness_ready_when_failed_summary_does_not_block() {
    use crate::briefing::{ArticleSummaryResult, BriefingSession, LoadedArticle};
    use crate::state::BriefingGenerateReadiness;
    use crate::summary_cache::SummaryCacheKey;
    use harvester_engine::llm::dto::SummaryEntities;
    use harvester_engine::llm::prompt::PromptId;

    init_logging();
    let mut state = complete_triage_state_for_test(2);

    state.store_summary_result(
        SummaryCacheKey {
            content_hash: "hash-tc-0".to_string(),
            prompt_id: PromptId::ArticleSummary,
            prompt_version: 1,
            model_id: "test-summary-model".to_string(),
            context_hash: "ctx".to_string(),
        },
        ArticleSummaryResult {
            title: "A".to_string(),
            summary: "s".to_string(),
            key_points: vec![],
            input_tokens: 1,
            output_tokens: 1,
            entities: SummaryEntities::default(),
        },
        "2026-05-01T00:00:00Z".to_string(),
    );

    let mut briefing = BriefingSession::new_loading(None);
    briefing.set_articles(
        vec![LoadedArticle {
            url: "https://triage-complete.com/1".to_string(),
            source_title: None,
            prepared_text: "t".to_string(),
            content_hash: "hash-tc-1".to_string(),
            fetched_utc: None,
        }],
        "c".to_string(),
    );
    briefing.transition_to_summarizing();
    briefing.start_article(0, 1);
    briefing.fail_article(0, "network".to_string());
    briefing.complete_without_briefing();
    state.set_briefing(briefing);

    assert!(matches!(
        state.briefing_generate_readiness(),
        BriefingGenerateReadiness::Ready { .. }
    ));
}

#[test]
fn briefing_generate_readiness_signal_scoring_in_progress() {
    use crate::briefing::ArticleSummaryResult;
    use crate::state::BriefingGenerateReadiness;
    use crate::summary_cache::SummaryCacheKey;
    use harvester_engine::llm::dto::SummaryEntities;
    use harvester_engine::llm::prompt::PromptId;

    init_logging();
    let mut state = complete_triage_state_for_test(2);

    for i in 0..2usize {
        state.store_summary_result(
            SummaryCacheKey {
                content_hash: format!("hash-tc-{i}"),
                prompt_id: PromptId::ArticleSummary,
                prompt_version: 1,
                model_id: "test-summary-model".to_string(),
                context_hash: "ctx".to_string(),
            },
            ArticleSummaryResult {
                title: "A".to_string(),
                summary: "s".to_string(),
                key_points: vec![],
                input_tokens: 1,
                output_tokens: 1,
                entities: SummaryEntities::default(),
            },
            "2026-05-01T00:00:00Z".to_string(),
        );
    }

    let url = "https://triage-complete.com/0".to_string();
    state.signal_candidate_mut().enqueue(url);
    assert!(state.signal_candidate().in_flight_count() > 0);

    assert!(matches!(
        state.briefing_generate_readiness(),
        BriefingGenerateReadiness::SignalScoringInProgress
    ));
}

#[test]
fn view_exposes_archive_token_estimate_and_article_counts() {
    use crate::briefing::ArticleSummaryResult;
    use crate::summary_cache::SummaryCacheKey;
    use crate::{JobResultKind, Stage};
    use harvester_engine::llm::dto::SummaryEntities;
    use harvester_engine::llm::prompt::PromptId;

    init_logging();

    // Local helper: enqueue a URL and return its job id (so we can drive it into
    // queued / in-flight / failed states the completed-job helper can't produce).
    fn enqueue(state: AppState, url: &str) -> (AppState, crate::JobId) {
        let (state, e1) = update(state, Msg::InputChanged(format!("{url}\n")));
        let (state, e2) = update(state, Msg::UrlsSubmitted);
        let job_id = e1
            .into_iter()
            .chain(e2)
            .find_map(|e| match e {
                Effect::EnqueueUrl { job_id, .. } => Some(job_id),
                _ => None,
            })
            .expect("EnqueueUrl effect");
        (state, job_id)
    }

    // (1) One article in the completed triage corpus, downloaded (500 raw tokens)
    //     and summarized (42 output tokens). In the archive corpus; NOT raw.
    let triaged_url = "https://triage-complete.com/0".to_string();
    let state = complete_triage_state_for_test(1);
    let mut state = add_completed_job_with_tokens_for_test(state, &triaged_url, 500);
    state.store_summary_result(
        SummaryCacheKey {
            content_hash: "hash-tc-0".to_string(),
            prompt_id: PromptId::ArticleSummary,
            prompt_version: 4,
            model_id: "model".to_string(),
            context_hash: "ctx".to_string(),
        },
        ArticleSummaryResult {
            title: "Art".to_string(),
            summary: "summary text".to_string(),
            key_points: vec![],
            input_tokens: 100,
            output_tokens: 42,
            entities: SummaryEntities::default(),
        },
        "2026-04-01T00:00:00Z".to_string(),
    );

    // (2) A successful downloaded job that is NOT in the archive corpus.
    //     With the correct implementation this must NOT count as raw.
    let state = add_completed_job_with_tokens_for_test(state, "https://fresh.example.com/new", 300);

    // (3) A FAILED job that still carries tokens (apply_done does not clear them).
    let (state, failed_id) = enqueue(state, "https://fail.example.com/x");
    let (state, _) = update(
        state,
        Msg::JobProgress {
            job_id: failed_id,
            stage: Stage::Tokenizing,
            tokens: Some(700),
            bytes: None,
            content_preview: None,
        },
    );
    let (state, _) = update(
        state,
        Msg::JobDone {
            job_id: failed_id,
            result: JobResultKind::Failed {
                reason: "boom".to_string(),
            },
            content_preview: None,
            extracted_links: Vec::new(),
            fetched_utc: None,
        },
    );

    // (4) An IN-FLIGHT job: tokens set via progress, never completed.
    let (state, inflight_id) = enqueue(state, "https://inflight.example.com/x");
    let (state, _) = update(
        state,
        Msg::JobProgress {
            job_id: inflight_id,
            stage: Stage::Tokenizing,
            tokens: Some(800),
            bytes: None,
            content_preview: None,
        },
    );

    // (5) A QUEUED job: enqueued only, no tokens, not done.
    let (state, _queued_id) = enqueue(state, "https://queued.example.com/x");

    let view = state.view();

    // Estimate = summary-mode archive size over the filtered corpus only.
    assert_eq!(view.archive_token_estimate, 42);
    // Filtered corpus has exactly the one triaged article.
    assert_eq!(view.archive_filtered_count, 1);
    // The single archive article (1) is fully summarized. Job (2) has no summary
    // but is outside the archive corpus — it must not be counted as raw.
    assert_eq!(view.raw_unprocessed_count, 0);
}

#[test]
fn raw_unprocessed_count_is_archive_corpus_articles_without_summary() {
    use crate::briefing::ArticleSummaryResult;
    use crate::summary_cache::SummaryCacheKey;
    use harvester_engine::llm::dto::SummaryEntities;
    use harvester_engine::llm::prompt::PromptId;

    init_logging();

    // Two archive-corpus articles: one summarized, one not.
    let url_summarized = "https://triage-complete.com/0".to_string();
    let url_raw = "https://triage-complete.com/1".to_string();
    let state = complete_triage_state_for_test(2);
    let state = add_completed_job_with_tokens_for_test(state, &url_summarized, 400);
    let mut state = add_completed_job_with_tokens_for_test(state, &url_raw, 600);

    state.store_summary_result(
        SummaryCacheKey {
            content_hash: "hash-tc-0".to_string(),
            prompt_id: PromptId::ArticleSummary,
            prompt_version: 4,
            model_id: "model".to_string(),
            context_hash: "ctx".to_string(),
        },
        ArticleSummaryResult {
            title: "Summarized".to_string(),
            summary: "s".to_string(),
            key_points: vec![],
            input_tokens: 100,
            output_tokens: 30,
            entities: SummaryEntities::default(),
        },
        "2026-04-01T00:00:00Z".to_string(),
    );

    let view = state.view();

    assert_eq!(view.archive_filtered_count, 2);
    // url_raw is in the archive corpus but has no summary → counts as 1 raw.
    assert_eq!(view.raw_unprocessed_count, 1);
}

#[test]
fn signal_candidate_mode_keeps_raw_count_over_full_archive_corpus() {
    use crate::briefing::ArticleSummaryResult;
    use crate::summary_cache::SummaryCacheKey;
    use harvester_engine::llm::dto::{
        Confidence, SignalCandidateResult, SourceTier, SummaryEntities,
    };
    use harvester_engine::llm::prompt::PromptId;

    init_logging();

    // Three triaged articles in the archive corpus. Only article /0 is summarized
    // and promoted to a settled signal candidate; /1 and /2 remain unsummarized.
    let url_candidate = "https://triage-complete.com/0".to_string();
    let state = complete_triage_state_for_test(3);
    let state = add_completed_job_with_tokens_for_test(state, &url_candidate, 400);
    let state = add_completed_job_with_tokens_for_test(state, "https://triage-complete.com/1", 500);
    let mut state =
        add_completed_job_with_tokens_for_test(state, "https://triage-complete.com/2", 600);

    state.store_summary_result(
        SummaryCacheKey {
            content_hash: "hash-tc-0".to_string(),
            prompt_id: PromptId::ArticleSummary,
            prompt_version: 4,
            model_id: "model".to_string(),
            context_hash: "ctx".to_string(),
        },
        ArticleSummaryResult {
            title: "Candidate".to_string(),
            summary: "s".to_string(),
            key_points: vec![],
            input_tokens: 100,
            output_tokens: 30,
            entities: SummaryEntities::default(),
        },
        "2026-04-01T00:00:00Z".to_string(),
    );

    // Settle a single signal candidate (article /0). All scoring done, none in flight,
    // so the meter switches to the signal-candidate export subset.
    state.signal_candidate_mut().enqueue(url_candidate.clone());
    state.signal_candidate_mut().mark_scoring(&url_candidate, 1);
    state.signal_candidate_mut().complete(
        &url_candidate,
        SignalCandidateResult {
            signal_score: 90,
            signal_key: "cluster-a".to_string(),
            themes: vec!["theme".to_string()],
            draft_gist: "gist".to_string(),
            source_tier: SourceTier::Tier1,
            confidence: Confidence::High,
            reasoning: "reason".to_string(),
            input_tokens: 10,
            output_tokens: 2,
        },
    );

    let view = state.view();

    // Bar/filtered reflect the export subset: the single selected candidate.
    assert_eq!(view.archive_filtered_count, 1);
    assert_eq!(view.archive_token_estimate, 30);
    // Raw stays the full-corpus backlog: /1 and /2 have no summary → 2 raw.
    assert_eq!(view.raw_unprocessed_count, 2);
}
