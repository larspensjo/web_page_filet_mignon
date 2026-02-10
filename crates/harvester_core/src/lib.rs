//! Harvester core: pure state machine and view-model helpers.
mod briefing;
mod effect;
mod msg;
mod state;
mod triage;
mod ui_geometry;
mod update;
mod url_age;
mod view_model;

pub use briefing::{
    ArticleSummaryResult, BriefingArticle, BriefingArticleId, BriefingPhase, BriefingResult,
    BriefingSession, BriefingThemeResult, LoadedArticle,
};
pub use effect::{Effect, StopPolicy};
pub use msg::{LlmResultKind, Msg};
pub use state::{
    normalize_url_for_dedupe, AppState, CompletedJobSnapshot, JobId, JobResultKind,
    LinkDownloadState, LinkSnapshotRecord, LlmRequestState, LlmResultIndex, SessionState, Stage,
};
pub use triage::{
    ArticleTriageResult, ArticleTriageState, TriageArticle, TriageArticleId, TriagePhase,
    TriageSession,
};
pub use ui_geometry::calc_left_width;
pub use update::update;
pub use view_model::{
    AppViewModel, JobRowView, LinkRowView, PreviewHeaderView, TriageAnnotationView,
    DEFAULT_JOBS_PANEL_WIDTH, DEFAULT_LEFT_PANEL_WIDTH, DEFAULT_WINDOW_WIDTH,
    INPUT_PANEL_FIXED_WIDTH, MIN_JOBS_PANEL_WIDTH, TOKEN_LIMIT,
};
