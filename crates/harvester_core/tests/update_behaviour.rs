use harvester_core::{update, AppState, Msg, SessionState};

#[test]
fn urls_pasted_trims_and_ignores_empty() {
    let state = AppState::new();
    let input = "https://a.example.com \n\n  https://b.example.com\n   \n";

    let (next, _effects) = update(state, Msg::UrlsPasted(input.to_string()));
    let view = next.view();

    assert_eq!(
        view.queued_urls,
        vec![
            "https://a.example.com".to_string(),
            "https://b.example.com".to_string(),
        ]
    );
    assert!(view.dirty);
}

#[test]
fn start_moves_idle_to_running() {
    let state = AppState::new();
    let (next, _effects) = update(state, Msg::StartClicked);

    assert_eq!(next.view().session, SessionState::Running);
    assert!(next.view().dirty);
}

#[test]
fn stop_finish_moves_running_to_finishing() {
    let state = AppState::new();
    let (state, _effects) = update(state, Msg::StartClicked);
    let (state, _effects) = update(state, Msg::StopFinishClicked);

    assert_eq!(state.view().session, SessionState::Finishing);
    assert!(state.view().dirty);
}
