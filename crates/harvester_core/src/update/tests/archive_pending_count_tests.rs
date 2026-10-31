use super::*;

#[test]
fn archive_clicked_with_triage_complete_and_pre_triage_ready_sets_pending_count() {
    init_logging();
    let state = complete_triage_state_for_test(1);
    let url = "https://pending.com/1";
    let state = add_completed_job_for_test(state, url);
    let (state, request_id) = tick_until_dispatch(state);
    let retained = state
        .triage()
        .articles()
        .iter()
        .map(|a| LoadedArticle {
            url: a.url.clone(),
            source_title: a.source_title.clone(),
            prepared_text: a.prepared_text.clone(),
            content_hash: a.content_hash.clone(),
            fetched_utc: a.fetched_utc.clone(),
        })
        .collect::<Vec<_>>();
    let (state, _) = update(
        state,
        Msg::TriageArticlesLoaded {
            request_id,
            delta: harvester_engine::TriageArticleDelta::full_window(
                retained
                    .into_iter()
                    .chain(loaded_pre_triage_articles(&[url]))
                    .collect(),
                100_000,
            ),
        },
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
            delta: harvester_engine::TriageArticleDelta::full_window(articles, 100_000),
        },
    );
    assert!(
        matches!(
            state.pre_triage().phase(),
            crate::pre_triage_filter::PreTriagePhase::Reviewing
        ),
        "unresolved review articles should derive Reviewing immediately after load"
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
