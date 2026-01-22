//! Harvester core: pure state machine and view-model helpers.
mod effect;
mod msg;
mod state;
mod update;
mod view_model;

pub use effect::{Effect, StopPolicy};
pub use msg::Msg;
pub use state::{
    normalize_url_for_dedupe, AppState, CompletedJobSnapshot, JobId, JobResultKind, SessionState,
    Stage,
};
pub use update::update;
pub use view_model::{AppViewModel, JobRowView, TOKEN_LIMIT};
