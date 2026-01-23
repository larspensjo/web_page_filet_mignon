use harvester_core::{update, AppState, Effect, JobResultKind, Msg, Stage, TOKEN_LIMIT};

fn submit_urls(state: AppState, input: &str) -> (AppState, Vec<Effect>) {
    let (state, _) = update(state, Msg::InputChanged(input.to_string()));
    update(state, Msg::UrlsSubmitted)
}

#[test]
fn urls_pasted_trims_and_ignores_empty() {
    let state = AppState::new();
    let input = "https://a.example.com \n\n  https://b.example.com\n   \n";

    let (mut next, _effects) = submit_urls(state, input);
    let view = next.view();

    assert!(view.queued_urls.is_empty());
    assert_eq!(view.job_count, 2);
    assert!(next.consume_dirty());

    // Progress on job 1.
    let (mut next, _effects) = update(
        next,
        Msg::JobProgress {
            job_id: 1,
            stage: Stage::Downloading,
            tokens: Some(10),
            bytes: Some(1024),
            content_preview: None,
        },
    );
    let job1 = next
        .view()
        .jobs
        .iter()
        .find(|j| j.job_id == 1)
        .unwrap()
        .clone();
    assert_eq!(job1.stage, Stage::Downloading);
    assert_eq!(job1.tokens, Some(10));
    assert_eq!(job1.bytes, Some(1024));
    assert!(next.consume_dirty());

    // Completion for job 1.
    let (mut next, _effects) = update(
        next,
        Msg::JobDone {
            job_id: 1,
            result: JobResultKind::Success,
            content_preview: None,
        },
    );
    let job1_done = next
        .view()
        .jobs
        .iter()
        .find(|j| j.job_id == 1)
        .unwrap()
        .clone();
    assert_eq!(job1_done.stage, Stage::Done);
    assert_eq!(job1_done.outcome, Some(JobResultKind::Success));
    assert!(next.consume_dirty());
}

#[test]
fn jobs_are_ordered_by_btree_key() {
    let state = AppState::new();
    let (mut state, _effects) = submit_urls(state, "b.com\na.com\n");

    // BTreeMap iteration should yield deterministic ascending JobId order (1,2,...)
    let ids: Vec<_> = state.view().jobs.iter().map(|j| j.job_id).collect();
    assert_eq!(ids, vec![1, 2]);
    assert!(state.consume_dirty());
}

#[test]
fn token_totals_accumulate_and_replace_previous_values() {
    let state = AppState::new();
    let (state, _effects) = submit_urls(state, "a.com\nb.com\n");

    let (mut state, _effects) = update(
        state,
        Msg::JobProgress {
            job_id: 1,
            stage: Stage::Tokenizing,
            tokens: Some(120),
            bytes: None,
            content_preview: None,
        },
    );
    let view_after_first = state.view();
    assert_eq!(view_after_first.total_tokens, 120);
    assert_eq!(view_after_first.token_limit, TOKEN_LIMIT);
    assert!(state.consume_dirty());

    let (mut state, _effects) = update(
        state,
        Msg::JobProgress {
            job_id: 1,
            stage: Stage::Tokenizing,
            tokens: Some(150),
            bytes: None,
            content_preview: None,
        },
    );
    assert_eq!(state.view().total_tokens, 150);
    assert!(state.consume_dirty());

    let (mut state, _effects) = update(
        state,
        Msg::JobProgress {
            job_id: 2,
            stage: Stage::Tokenizing,
            tokens: Some(50),
            bytes: None,
            content_preview: None,
        },
    );
    assert_eq!(state.view().total_tokens, 200);
    assert!(state.consume_dirty());
}
