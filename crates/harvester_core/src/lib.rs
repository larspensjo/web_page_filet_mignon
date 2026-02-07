//! Harvester core: pure state machine and view-model helpers.
mod effect;
mod msg;
mod state;
mod ui_geometry;
mod update;
mod url_age;
mod view_model;

pub use effect::{Effect, StopPolicy};
pub use msg::Msg;
pub use state::{
    normalize_url_for_dedupe, AppState, CompletedJobSnapshot, JobId, JobResultKind,
    LinkDownloadState, LinkSnapshotRecord, SessionState, Stage,
};
pub use ui_geometry::calc_left_width;
pub use update::update;
pub use view_model::{
    AppViewModel, JobRowView, LinkRowView, PreviewHeaderView, DEFAULT_LEFT_PANEL_WIDTH,
    DEFAULT_WINDOW_WIDTH, TOKEN_LIMIT,
};
