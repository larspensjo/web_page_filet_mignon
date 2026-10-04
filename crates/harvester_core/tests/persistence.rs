use harvester_core::{
    update, AppState, CompletedJobSnapshot, Effect, JobResultKind, LinkSnapshotRecord, Msg, Stage,
};

fn submit_urls(state: AppState, input: &str) -> (AppState, Vec<Effect>) {
    let (state, _) = update(state, Msg::InputChanged(input.to_string()));
    update(state, Msg::UrlsSubmitted)
}

fn init_logging() {
    engine_logging::initialize_for_tests();
}

#[test]
fn completed_jobs_can_be_restored_for_resume() {
    init_logging();
    let (state, effects) = submit_urls(AppState::new(), "https://example.com\n");
    let job_id = effects
        .iter()
        .find_map(|effect| match effect {
            harvester_core::Effect::EnqueueUrl { job_id, .. } => Some(*job_id),
            _ => None,
        })
        .expect("enqueue effect");

    let (state, _) = update(
        state,
        Msg::JobProgress {
            job_id,
            stage: Stage::Tokenizing,
            tokens: Some(42),
            bytes: Some(1234),
        },
    );
    let (state, _) = update(
        state,
        Msg::JobDone {
            job_id,
            result: JobResultKind::Success,
            extracted_links: Vec::new(),
            fetched_utc: None,
        },
    );

    let snapshot = state.completed_jobs_snapshot();
    assert_eq!(snapshot.len(), 1);
    assert_eq!(snapshot[0].url, "https://example.com");
    assert_eq!(snapshot[0].tokens, Some(42));
    assert_eq!(snapshot[0].bytes, Some(1234));

    let (restored, _) = update(AppState::new(), Msg::RestoreCompletedJobs(snapshot));
    let view = restored.view();
    assert_eq!(view.job_count, 1);
    assert_eq!(restored.completed_jobs_snapshot()[0].tokens, Some(42));
    assert_eq!(
        view.desktop_job_list.rows[0].outcome,
        Some(JobResultKind::Success)
    );
    assert_eq!(view.desktop_job_list.rows[0].stage, Stage::Done);
}

#[test]
fn restored_jobs_are_deduped_on_paste() {
    init_logging();
    let (state, _) = update(
        AppState::new(),
        Msg::RestoreCompletedJobs(vec![CompletedJobSnapshot {
            url: "https://example.com".to_string(),
            tokens: None,
            bytes: None,
            links: Vec::new(),
            fetched_utc: None,
        }]),
    );

    let (next, effects) = submit_urls(state, "https://example.com\n");
    assert_eq!(next.view().job_count, 1);
    assert!(effects.is_empty());
}

#[test]
fn restore_completed_job_ignores_downloaded_paths_and_keeps_links() {
    init_logging();
    let snapshot = vec![CompletedJobSnapshot {
        url: "https://example.com".to_string(),
        tokens: None,
        bytes: None,
        links: vec![LinkSnapshotRecord {
            url: "https://downloaded.example".to_string(),
            downloaded_path: Some("linked/123.md".to_string()),
        }],
        fetched_utc: None,
    }];

    let (state, _) = update(AppState::new(), Msg::RestoreCompletedJobs(snapshot));
    let links = state.job_links(1).expect("job links available");
    assert_eq!(links.len(), 1);
    assert_eq!(links[0].url, "https://downloaded.example/");
    assert!(state.completed_jobs_snapshot()[0].links[0]
        .downloaded_path
        .is_none());
    let (state, _) = update(state, Msg::JobSelected { job_id: 1 });
    assert_eq!(
        state.view().desktop_job_list.selected_job.unwrap().links[0].url,
        "https://downloaded.example/"
    );
}

#[test]
fn selected_job_keeps_extracted_links_after_completion() {
    let (state, _) = submit_urls(AppState::new(), "https://example.com/article");
    let (state, _) = update(
        state,
        Msg::JobDone {
            job_id: 1,
            result: JobResultKind::Success,
            extracted_links: vec![harvester_engine::ExtractedLink {
                url: "https://example.com/link".into(),
                text: Some("Read more".into()),
                kind: harvester_engine::LinkKind::Hyperlink,
            }],
            fetched_utc: None,
        },
    );
    let (state, _) = update(state, Msg::JobSelected { job_id: 1 });
    let selected = state.view().desktop_job_list.selected_job.unwrap();
    assert_eq!(selected.links.len(), 1);
    assert_eq!(selected.links[0].index, 0);
    assert_eq!(selected.links[0].url, "https://example.com/link");
    assert_eq!(selected.links[0].label, "Read more");
    let (_, effects) = update(
        state,
        Msg::ExtractedLinkOpenRequested {
            job_id: 1,
            link_index: 0,
        },
    );
    assert_eq!(
        effects,
        vec![Effect::OpenUrlInBrowser {
            url: "https://example.com/link".into()
        }]
    );
}

#[test]
fn selection_loads_links_only_on_change_and_ignores_stale_replies() {
    let snapshots =
        ["https://example.com/a", "https://example.com/b"].map(|url| CompletedJobSnapshot {
            url: url.into(),
            tokens: None,
            bytes: None,
            links: vec![],
            fetched_utc: None,
        });
    let (state, _) = update(
        AppState::new(),
        Msg::RestoreCompletedJobs(snapshots.to_vec()),
    );
    let (state, effects) = update(state, Msg::JobSelected { job_id: 1 });
    assert_eq!(
        effects,
        [
            Effect::LoadArticleLinks {
                job_id: 1,
                url: snapshots[0].url.clone()
            },
            Effect::PersistRuntimeState {
                snapshot: harvester_core::PersistenceSnapshot::capture(&state)
            }
        ]
    );
    let (state, effects) = update(state, Msg::JobSelected { job_id: 1 });
    assert!(effects.is_empty());
    let (state, _) = update(state, Msg::JobSelected { job_id: 2 });
    let links = vec![harvester_engine::ExtractedLink {
        url: "https://example.com/link".into(),
        text: None,
        kind: harvester_engine::LinkKind::Hyperlink,
    }];
    let (state, _) = update(
        state,
        Msg::ArticleLinksLoaded {
            job_id: 1,
            url: snapshots[0].url.clone(),
            links: Ok(links.clone()),
        },
    );
    assert!(state.job_links(1).unwrap().is_empty());
    let (state, _) = update(
        state,
        Msg::ArticleLinksLoaded {
            job_id: 2,
            url: snapshots[0].url.clone(),
            links: Ok(links.clone()),
        },
    );
    assert!(state.job_links(2).unwrap().is_empty());
    let (state, _) = update(
        state,
        Msg::ArticleLinksLoaded {
            job_id: 2,
            url: snapshots[1].url.clone(),
            links: Ok(links),
        },
    );
    assert_eq!(state.job_links(2).unwrap().len(), 1);
    let snapshot = harvester_core::PersistenceSnapshot::capture(&state);
    assert_eq!(snapshot.completed.len(), 2);
    assert_eq!(snapshot.completed[1].url, snapshots[1].url);
}

#[test]
fn late_empty_or_failed_link_reply_preserves_completed_session_links() {
    let url = "https://example.com/article";
    let (state, _) = update(AppState::new(), Msg::InputChanged(url.into()));
    let (state, _) = update(state, Msg::UrlsSubmitted);
    let (state, effects) = update(state, Msg::JobSelected { job_id: 1 });
    assert!(
        effects
            .iter()
            .all(|effect| matches!(effect, Effect::PersistRuntimeState { .. })),
        "in-flight jobs only persist selection and do not load links"
    );
    let (state, _) = update(
        state,
        Msg::JobDone {
            job_id: 1,
            result: harvester_core::JobResultKind::Success,
            extracted_links: vec![harvester_engine::ExtractedLink {
                url: "https://example.com/link".into(),
                text: Some("Kept".into()),
                kind: harvester_engine::LinkKind::Hyperlink,
            }],
            fetched_utc: None,
        },
    );
    for reply in [Ok(vec![]), Err("store unavailable".into())] {
        let (next, effects) = update(
            state.clone(),
            Msg::ArticleLinksLoaded {
                job_id: 1,
                url: url.into(),
                links: reply,
            },
        );
        assert!(effects.is_empty());
        assert_eq!(
            next.job_links(1).unwrap()[0].url,
            "https://example.com/link"
        );
        assert_eq!(
            next.view().desktop_job_list.selected_job.unwrap().links[0].label,
            "Kept"
        );
    }
    // Re-selecting a completed job with resident links also avoids disk I/O.
    let (state, _) = update(state, Msg::InputChanged("https://example.com/other".into()));
    let (state, _) = update(state, Msg::UrlsSubmitted);
    let (state, effects) = update(state, Msg::JobSelected { job_id: 2 });
    assert_eq!(
        effects,
        [Effect::PersistRuntimeState {
            snapshot: harvester_core::PersistenceSnapshot::capture(&state)
        }]
    );
    let (_, effects) = update(state, Msg::JobSelected { job_id: 1 });
    assert!(matches!(
        effects.as_slice(),
        [Effect::PersistRuntimeState { .. }]
    ));
}
