use harvester_core::{
    update, AppState, CompletedJobSnapshot, Effect, JobResultKind, LinkDownloadState,
    LinkSnapshotRecord, Msg, Stage,
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
            content_preview: None,
        },
    );
    let (state, _) = update(
        state,
        Msg::JobDone {
            job_id,
            result: JobResultKind::Success,
            content_preview: None,
            extracted_links: Vec::new(),
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
    assert_eq!(view.total_tokens, 42);
    assert_eq!(view.jobs[0].outcome, Some(JobResultKind::Success));
    assert_eq!(view.jobs[0].stage, Stage::Done);
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
        }]),
    );

    let (next, effects) = submit_urls(state, "https://example.com\n");
    assert_eq!(next.view().job_count, 1);
    assert!(effects.is_empty());
}

#[test]
fn restore_completed_job_records_downloaded_link_paths() {
    init_logging();
    let snapshot = vec![CompletedJobSnapshot {
        url: "https://example.com".to_string(),
        tokens: None,
        bytes: None,
        links: vec![LinkSnapshotRecord {
            url: "https://downloaded.example".to_string(),
            downloaded_path: Some("linked/123.md".to_string()),
        }],
    }];

    let (state, _) = update(AppState::new(), Msg::RestoreCompletedJobs(snapshot));
    let links = state.job_links(1).expect("job links available");
    assert_eq!(links.len(), 1);
    assert!(matches!(
        links[0].download_state,
        LinkDownloadState::Downloaded { ref path } if path.ends_with("linked/123.md")
    ));
}
