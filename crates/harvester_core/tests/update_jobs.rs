use harvester_core::{update, AppState, JobResultKind, Msg, Stage, TOKEN_LIMIT};

#[test]
fn urls_pasted_trims_and_ignores_empty() {
    let state = AppState::new();
    let input = "https://a.example.com \n\n  https://b.example.com\n   \n";

    let (mut next, _effects) = update(state, Msg::UrlsPasted(input.to_string()));
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
    let (mut state, _effects) = update(state, Msg::UrlsPasted("b.com\na.com\n".into()));

    // BTreeMap iteration should yield deterministic ascending JobId order (1,2,...)
    let ids: Vec<_> = state.view().jobs.iter().map(|j| j.job_id).collect();
    assert_eq!(ids, vec![1, 2]);
    assert!(state.consume_dirty());
}

#[test]
fn token_totals_accumulate_and_replace_previous_values() {
    let state = AppState::new();
    let (state, _effects) = update(state, Msg::UrlsPasted("a.com\nb.com\n".into()));

    let (mut state, _effects) = update(
        state,
        Msg::JobProgress {
            job_id: 1,
            stage: Stage::Tokenizing,
            tokens: Some(120),
            bytes: None,
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
        },
    );
    assert_eq!(state.view().total_tokens, 200);
    assert!(state.consume_dirty());
}
