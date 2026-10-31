use super::*;

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
fn resume_run_consumes_reviewing_pre_triage_into_triage_session() {
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
            delta: harvester_engine::TriageArticleDelta::full_window(articles, 100_000),
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
        state.view().run_enabled,
        "Reviewing phase with tentative included articles must allow triage start"
    );

    let state = prime_llm_metadata(state);
    let (state, effects) = update(
        state,
        Msg::PipelineRunRequested {
            scope: crate::PipelineRunScope::Resume,
        },
    );

    assert!(!effects.is_empty(), "triage should dispatch from Reviewing");
    assert!(
        matches!(
            state.pre_triage().phase(),
            crate::pre_triage_filter::PreTriagePhase::Idle
        ),
        "pre-triage must reset to Idle after a Resume run"
    );
    assert_eq!(
        state.triage().articles().len(),
        2,
        "automatic pre-triage includes both review articles without retired overrides"
    );
    assert_eq!(
        state
            .triage()
            .articles()
            .iter()
            .map(|article| article.url.as_str())
            .collect::<Vec<_>>(),
        vec![url1, url2]
    );
}

#[test]
fn resume_run_consumes_ready_pre_triage_into_triage_session() {
    init_logging();
    let urls = &["https://handoff.com/1", "https://handoff.com/2"];
    let state = ready_pre_triage_state(urls);
    let state = prime_llm_metadata(state);

    assert!(matches!(
        state.pre_triage().phase(),
        crate::pre_triage_filter::PreTriagePhase::ReadyToTriage
    ));

    let (state, _effects) = update(
        state,
        Msg::PipelineRunRequested {
            scope: crate::PipelineRunScope::Resume,
        },
    );

    assert!(
        matches!(
            state.pre_triage().phase(),
            crate::pre_triage_filter::PreTriagePhase::Idle
        ),
        "pre-triage must reset to Idle after a Resume run"
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
fn resume_run_makes_archive_corpus_available_when_triage_completes() {
    init_logging();
    let urls = &["https://corpus-src.com/1"];
    let state = ready_pre_triage_state(urls);
    let state = prime_llm_metadata(state);

    let (state, effects) = update(
        state,
        Msg::PipelineRunRequested {
            scope: crate::PipelineRunScope::Resume,
        },
    );

    let state = complete_all_triage_llm_requests(state, effects);
    assert_eq!(state.triage().phase(), &crate::TriagePhase::Complete);
    assert_eq!(state.archive_corpus().count(), 1);
}

#[test]
fn archive_clicked_after_triage_start_is_gated_while_run_active() {
    init_logging();
    let urls = &["https://archive-handoff.com/1"];
    let state = ready_pre_triage_state(urls);
    let state = prime_llm_metadata(state);

    let (state, effects) = update(
        state,
        Msg::PipelineRunRequested {
            scope: crate::PipelineRunScope::Resume,
        },
    );
    let state = complete_all_triage_llm_requests(state, effects);

    assert_eq!(state.run_state(), crate::RunState::Active);
    let (state, archive_effects) = update(state, Msg::ArchiveClicked);
    assert!(archive_effects.is_empty());
    assert!(!state.view().archive_enabled);
    let (mut state, _) = update(state, Msg::StopFinishClicked);
    let in_flight = state
        .briefing()
        .articles()
        .iter()
        .filter_map(|article| match article.summary_state {
            crate::briefing::ArticleSummaryState::InProgress { request_id } => Some(request_id),
            _ => None,
        })
        .collect::<Vec<_>>();
    for request_id in in_flight {
        (state, _) = update(
            state,
            Msg::LlmCompleted {
                request_id,
                result: crate::LlmResultKind::Failed {
                    reason: "summary fixture drain".into(),
                },
                metadata: None,
            },
        );
    }
    assert_eq!(state.run_state(), crate::RunState::Idle);
    let (_, archive_effects) = update(state, Msg::ArchiveClicked);
    let pending_count = archive_effects
        .iter()
        .find_map(|effect| match effect {
            Effect::OpenArchiveDialog {
                pending_pre_triage_count,
                ..
            } => Some(*pending_pre_triage_count),
            _ => None,
        })
        .expect("expected OpenArchiveDialog after run settled");
    assert_eq!(pending_count, 0);
}

#[test]
fn pre_triage_refresh_after_triage_start_repopulates_pre_triage_without_mutating_active_triage() {
    init_logging();
    let urls = &["https://repopulate.com/1"];
    let state = ready_pre_triage_state(urls);
    let state = prime_llm_metadata(state);

    let (state, _effects) = update(
        state,
        Msg::PipelineRunRequested {
            scope: crate::PipelineRunScope::Resume,
        },
    );
    assert!(
        matches!(
            state.pre_triage().phase(),
            crate::pre_triage_filter::PreTriagePhase::Idle
        ),
        "pre-triage must be Idle after a Resume run"
    );

    let triage_article_count_before = state.triage().articles().len();
    let triage_phase_before = state.triage().phase().clone();

    let new_url = "https://repopulate.com/new-article";
    let state = add_completed_job_for_test(state, new_url);
    assert_eq!(state.triage().articles().len(), triage_article_count_before);
    let (state, request_id) = tick_until_dispatch(state);
    assert_eq!(state.triage().articles().len(), triage_article_count_before);
    let new_articles = loaded_pre_triage_articles(&[new_url]);
    let (state, _) = update(
        state,
        Msg::TriageArticlesLoaded {
            request_id,
            delta: harvester_engine::TriageArticleDelta::full_window(new_articles, 100_000),
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
