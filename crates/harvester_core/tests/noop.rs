use harvester_core::{update, AppState, Msg};

#[test]
fn advance_without_a_run_is_noop() {
    let state = AppState::new();
    let (next, effects) = update(state.clone(), Msg::PipelineRunAdvance);

    assert_eq!(state, next);
    assert!(effects.is_empty());
}
