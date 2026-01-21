use harvester_core::{update, AppState, Msg};

#[test]
fn update_is_noop() {
    let state = AppState::new();
    let (next, effects) = update(state.clone(), Msg::NoOp);

    assert_eq!(state, next);
    assert!(effects.is_empty());
}
