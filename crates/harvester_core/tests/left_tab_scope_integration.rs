use harvester_core::{update, AppState, JobListScope, JobResultKind, LeftTab, Msg};
use std::collections::BTreeSet;

fn submit_urls(state: AppState, input: &str) -> AppState {
    let (state, _) = update(state, Msg::InputChanged(input.to_string()));
    let (state, _) = update(state, Msg::UrlsSubmitted);
    state
}

fn mark_done(state: AppState, job_id: u64) -> AppState {
    let (state, _) = update(
        state,
        Msg::JobDone {
            job_id,
            result: JobResultKind::Success,
            content_preview: None,
            extracted_links: Vec::new(),
            fetched_utc: Some("2026-03-03T10:00:00Z".to_string()),
        },
    );
    state
}

#[test]
fn scope_toggling_across_job_tabs_preserves_scope() {
    let state = AppState::new();
    let (state, _) = update(
        state,
        Msg::JobListScopeSet {
            scope: JobListScope::SinceCheckpoint,
        },
    );
    let (state, _) = update(
        state,
        Msg::LeftTabSelected {
            tab: LeftTab::TriageReview,
        },
    );
    let (state, _) = update(
        state,
        Msg::LeftTabSelected {
            tab: LeftTab::TriageResults,
        },
    );
    let (state, _) = update(state, Msg::LeftTabSelected { tab: LeftTab::Jobs });
    assert_eq!(state.job_list_scope(), JobListScope::SinceCheckpoint);
}

#[test]
fn triage_clicked_does_not_force_left_tab_switch() {
    let (state, _) = update(
        AppState::new(),
        Msg::LeftTabSelected {
            tab: LeftTab::TriageResults,
        },
    );
    let (state, _) = update(state, Msg::TriageClicked);
    assert_eq!(state.left_tab(), LeftTab::TriageResults);
}

#[test]
fn burst_updates_with_tab_and_scope_switches_keep_job_rows_unique() {
    let mut state = AppState::new();
    state = submit_urls(
        state,
        "https://example.com/a\nhttps://example.com/b\nhttps://example.com/c\n",
    );
    let (next, _) = update(
        state,
        Msg::LeftTabSelected {
            tab: LeftTab::TriageReview,
        },
    );
    state = next;
    let (next, _) = update(
        state,
        Msg::JobListScopeSet {
            scope: JobListScope::SinceCheckpoint,
        },
    );
    state = next;

    // Simulate a burst where jobs complete while users switch tabs/scope quickly.
    state = mark_done(state, 1);
    let (next, _) = update(
        state,
        Msg::LeftTabSelected {
            tab: LeftTab::TriageResults,
        },
    );
    state = next;
    state = mark_done(state, 2);
    let (next, _) = update(
        state,
        Msg::JobListScopeSet {
            scope: JobListScope::All,
        },
    );
    state = next;
    state = mark_done(state, 3);
    let (state, _) = update(
        state,
        Msg::LeftTabSelected {
            tab: LeftTab::Jobs,
        },
    );

    let view = state.view();
    let ids: Vec<u64> = view.jobs.iter().map(|j| j.job_id).collect();
    let unique: BTreeSet<u64> = ids.iter().copied().collect();
    assert_eq!(ids.len(), 3, "expected exactly three visible jobs");
    assert_eq!(unique.len(), 3, "burst updates must not duplicate rows");
    assert_eq!(ids, vec![1, 2, 3], "jobs must remain deterministic");
}
