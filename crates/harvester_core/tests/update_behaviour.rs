use std::sync::Once;

use harvester_core::{update, AppState, Effect, Msg, SessionState, StopPolicy};

fn init_logging() {
    static INIT: Once = Once::new();
    INIT.call_once(engine_logging::initialize_for_tests);
}

fn submit_urls(state: AppState, input: &str) -> (AppState, Vec<Effect>) {
    let (state, _) = update(state, Msg::InputChanged(input.to_string()));
    update(state, Msg::UrlsSubmitted)
}

#[test]
fn urls_pasted_trims_and_ignores_empty() {
    init_logging();
    let state = AppState::new();
    let input = "https://a.example.com \n\n  https://b.example.com\n   \n";

    let (next, effects) = submit_urls(state, input);
    let view = next.view();

    assert_eq!(view.session, SessionState::Running);
    assert_eq!(view.queued_urls, Vec::<String>::new());
    assert_eq!(view.job_count, 2);
    assert!(view.dirty);
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

    let (next, effects) = submit_urls(next, "   \n\n");
    assert_eq!(next.view().job_count, 2);
    assert!(effects.is_empty());
}

#[test]
fn stop_finish_moves_running_to_finishing() {
    init_logging();
    let state = AppState::new();
    let (state, _effects) = submit_urls(state, "https://example.com\n");
    let (state, _effects) = update(state, Msg::StopFinishClicked);

    assert_eq!(state.view().session, SessionState::Finishing);
    assert!(state.view().dirty);
}

#[test]
fn stop_finish_emits_effect() {
    init_logging();
    let state = AppState::new();
    let (state, _effects) = submit_urls(state, "https://example.com\n");
    let (_state, effects) = update(state, Msg::StopFinishClicked);

    assert_eq!(
        effects,
        vec![Effect::StopFinish {
            policy: StopPolicy::Finish
        }]
    );
}

#[test]
fn urls_pasted_ignored_while_finishing() {
    init_logging();
    let state = AppState::new();
    let (state, _effects) = submit_urls(state, "https://example.com\n");
    let (mut state, _effects) = update(state, Msg::StopFinishClicked);
    assert!(state.consume_dirty());

    let (mut next, effects) = submit_urls(state, "https://a.example.com\n");

    assert_eq!(next.view().session, SessionState::Finishing);
    assert_eq!(next.view().job_count, 1);
    assert!(effects.is_empty());
    assert!(!next.consume_dirty());
}

#[test]
fn urls_pasted_while_running_stays_running() {
    init_logging();
    let state = AppState::new();
    // First paste: Idle -> Running
    let (state, effects) = submit_urls(state, "https://first.example.com\n");
    assert_eq!(state.view().session, SessionState::Running);
    assert_eq!(effects.len(), 2); // StartSession + EnqueueUrl

    // Second paste while Running: should stay Running, no StartSession
    let (state, effects) = submit_urls(state, "https://second.example.com\n");
    assert_eq!(state.view().session, SessionState::Running);
    assert_eq!(state.view().job_count, 2);
    assert_eq!(
        effects,
        vec![Effect::EnqueueUrl {
            job_id: 2,
            url: "https://second.example.com".to_string(),
        }]
    );
}

#[test]
fn duplicate_paste_skipped() {
    init_logging();
    let state = AppState::new();
    // First paste
    let (state, effects) = submit_urls(state, "https://example.com\n");
    assert_eq!(state.view().job_count, 1);
    assert_eq!(effects.len(), 2); // StartSession + EnqueueUrl
    let view = state.view();
    assert_eq!(view.last_paste_stats.as_ref().unwrap().enqueued, 1);
    assert_eq!(view.last_paste_stats.as_ref().unwrap().skipped, 0);

    // Second paste with same URL - should be skipped
    let (state, effects) = submit_urls(state, "https://example.com\n");
    assert_eq!(state.view().job_count, 1); // No new job
    assert_eq!(effects.len(), 0); // No effects
    let view = state.view();
    assert_eq!(view.last_paste_stats.as_ref().unwrap().enqueued, 0);
    assert_eq!(view.last_paste_stats.as_ref().unwrap().skipped, 1);
}

#[test]
fn url_normalization_catches_variants() {
    init_logging();
    let state = AppState::new();
    // First paste with trailing slash
    let (state, effects) = submit_urls(state, "https://example.com/\n");
    assert_eq!(state.view().job_count, 1);
    assert_eq!(effects.len(), 2);

    // Second paste without trailing slash - should be recognized as duplicate
    let (state, effects) = submit_urls(state, "https://example.com\n");
    assert_eq!(state.view().job_count, 1);
    assert_eq!(effects.len(), 0);
    assert_eq!(state.view().last_paste_stats.as_ref().unwrap().skipped, 1);

    // Third paste with different case - should be recognized as duplicate
    let (state, effects) = submit_urls(state, "HTTPS://EXAMPLE.COM\n");
    assert_eq!(state.view().job_count, 1);
    assert_eq!(effects.len(), 0);
    assert_eq!(state.view().last_paste_stats.as_ref().unwrap().skipped, 1);

    // Fourth paste with extra whitespace - should be recognized as duplicate
    let (state, effects) = submit_urls(state, "  https://example.com/  \n");
    assert_eq!(state.view().job_count, 1);
    assert_eq!(effects.len(), 0);
    assert_eq!(state.view().last_paste_stats.as_ref().unwrap().skipped, 1);
}

#[test]
fn paste_with_mixed_new_and_duplicate_urls() {
    init_logging();
    let state = AppState::new();
    // First paste with two URLs
    let (state, effects) = submit_urls(state, "https://a.example.com\nhttps://b.example.com\n");
    assert_eq!(state.view().job_count, 2);
    assert_eq!(effects.len(), 3); // StartSession + 2x EnqueueUrl
    let view = state.view();
    assert_eq!(view.last_paste_stats.as_ref().unwrap().enqueued, 2);
    assert_eq!(view.last_paste_stats.as_ref().unwrap().skipped, 0);

    // Second paste with one duplicate and one new URL
    let (state, effects) = submit_urls(state, "https://a.example.com\nhttps://c.example.com\n");
    assert_eq!(state.view().job_count, 3);
    assert_eq!(effects.len(), 1); // Only 1 EnqueueUrl (c.example.com)
    let view = state.view();
    assert_eq!(view.last_paste_stats.as_ref().unwrap().enqueued, 1);
    assert_eq!(view.last_paste_stats.as_ref().unwrap().skipped, 1);
}

#[test]
fn archive_click_emits_effect_without_state_change() {
    init_logging();
    let state = AppState::new();
    let before = state.view();

    let (next, effects) = update(state, Msg::ArchiveClicked);

    assert_eq!(next.view(), before);
    assert_eq!(effects, vec![Effect::ArchiveRequested]);
}
