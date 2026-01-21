//! Harvester core: pure state machine and view-model helpers.
mod effect;
mod msg;
mod state;
mod update;
mod view_model;

pub use effect::Effect;
pub use msg::Msg;
pub use state::{AppState, JobId, JobResultKind, SessionState, Stage};
pub use update::update;
pub use view_model::AppViewModel;
