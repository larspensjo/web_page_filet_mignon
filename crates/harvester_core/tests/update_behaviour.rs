use harvester_core::{update, AppState, Effect, Msg, SessionState, StopPolicy};

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

#[test]
fn start_emits_start_and_enqueue_effects() {
    let state = AppState::new();
    let (state, _effects) = update(
        state,
        Msg::UrlsPasted("https://a.example.com\nhttps://b.example.com\n".into()),
    );

    let (_next, effects) = update(state, Msg::StartClicked);

    assert_eq!(
        effects,
        vec![
            Effect::StartSession,
            Effect::EnqueueUrl {
                job_id: 1,
                url: "https://a.example.com".to_string(),
            },
            Effect::EnqueueUrl {
                job_id: 2,
                url: "https://b.example.com".to_string(),
            },
        ]
    );
}

#[test]
fn stop_finish_emits_effect() {
    let state = AppState::new();
    let (state, _effects) = update(state, Msg::StartClicked);
    let (_state, effects) = update(state, Msg::StopFinishClicked);

    assert_eq!(
        effects,
        vec![Effect::StopFinish {
            policy: StopPolicy::Finish
        }]
    );
}
