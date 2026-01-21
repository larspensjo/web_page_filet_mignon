use crate::{AppState, Effect, Msg};

/// Pure update function: applies a message to state and returns any effects.
pub fn update(state: AppState, _msg: Msg) -> (AppState, Vec<Effect>) {
    // Phase 0 keeps the reducer a no-op; later phases will add real transitions.
    (state, Vec::new())
}
