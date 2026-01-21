use harvester_core::{update, AppState, Msg};

fn main() {
    let state = AppState::new();
    let (_state, _effects) = update(state, Msg::NoOp);

    println!("harvester_app bootstrap placeholder");
}
