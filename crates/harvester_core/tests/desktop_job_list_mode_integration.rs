use harvester_core::{update, AppState, JobListMode, JobResultKind, Msg, WorkspaceView};
use std::collections::BTreeSet;

fn submit_urls(state: AppState, input: &str) -> AppState {
    let (state, _) = update(state, Msg::InputChanged(input.to_string()));
    update(state, Msg::UrlsSubmitted).0
}

fn mark_done(state: AppState, job_id: u64) -> AppState {
    update(
        state,
        Msg::JobDone {
            job_id,
            result: JobResultKind::Success,
            content_preview: None,
            extracted_links: Vec::new(),
            fetched_utc: Some("2026-03-03T10:00:00Z".to_string()),
        },
    )
    .0
}

#[test]
fn mode_toggling_across_desktop_workspaces_preserves_mode() {
    let (state, _) = update(
        AppState::new(),
        Msg::JobListModeSet {
            mode: JobListMode::Last24Hours,
        },
    );
    let (state, _) = update(
        state,
        Msg::WorkspaceViewSet {
            view: WorkspaceView::Trends,
        },
    );
    let (state, _) = update(
        state,
        Msg::WorkspaceViewSet {
            view: WorkspaceView::Blacklist,
        },
    );
    let (state, _) = update(state, Msg::JobsSearchRevealRequested);

    assert_eq!(state.job_list_mode(), JobListMode::Last24Hours);
    assert_eq!(state.workspace_view(), WorkspaceView::Review);
}

#[test]
fn burst_updates_with_mode_and_workspace_switches_keep_desktop_rows_unique() {
    let mut state = submit_urls(
        AppState::new(),
        "https://example.com/a\nhttps://example.com/b\nhttps://example.com/c\n",
    );
    state = update(
        state,
        Msg::WorkspaceViewSet {
            view: WorkspaceView::Trends,
        },
    )
    .0;
    state = update(
        state,
        Msg::JobListModeSet {
            mode: JobListMode::Last24Hours,
        },
    )
    .0;

    state = mark_done(state, 1);
    state = update(
        state,
        Msg::WorkspaceViewSet {
            view: WorkspaceView::Blacklist,
        },
    )
    .0;
    state = mark_done(state, 2);
    state = update(
        state,
        Msg::JobListModeSet {
            mode: JobListMode::SinceCheckpoint,
        },
    )
    .0;
    state = mark_done(state, 3);

    let rows = state.view().desktop_job_list.rows;
    let ids: Vec<u64> = rows.iter().map(|row| row.job_id).collect();
    let unique: BTreeSet<u64> = ids.iter().copied().collect();
    assert_eq!(ids.len(), 3, "expected exactly three visible jobs");
    assert_eq!(unique.len(), 3, "burst updates must not duplicate rows");
    assert_eq!(ids, vec![1, 2, 3], "rows must remain deterministic");
}
