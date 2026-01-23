use harvester_core::{update, AppState, CompletedJobSnapshot, JobResultKind, Msg, Stage};

fn init_logging() {
    engine_logging::initialize_for_tests();
}

#[test]
fn completed_jobs_can_be_restored_for_resume() {
    init_logging();
    let (state, effects) = update(
        AppState::new(),
        Msg::UrlsPasted("https://example.com\n".to_string()),
    );
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
            content_preview: None,
        },
    );

    let snapshot = state.completed_jobs_snapshot();
    assert_eq!(snapshot.len(), 1);
    assert_eq!(snapshot[0].url, "https://example.com");
    assert_eq!(snapshot[0].tokens, Some(42));
    assert_eq!(snapshot[0].bytes, Some(1234));

    let mut restored = AppState::new();
    restored.restore_completed_jobs(snapshot);
    let view = restored.view();
    assert_eq!(view.job_count, 1);
    assert_eq!(view.total_tokens, 42);
    assert_eq!(view.jobs[0].outcome, Some(JobResultKind::Success));
    assert_eq!(view.jobs[0].stage, Stage::Done);
}

#[test]
fn restored_jobs_are_deduped_on_paste() {
    init_logging();
    let mut state = AppState::new();
    state.restore_completed_jobs(vec![CompletedJobSnapshot {
        url: "https://example.com".to_string(),
        tokens: None,
        bytes: None,
    }]);

    let (next, effects) = update(state, Msg::UrlsPasted("https://example.com\n".to_string()));
    assert_eq!(next.view().job_count, 1);
    assert!(effects.is_empty());
}
