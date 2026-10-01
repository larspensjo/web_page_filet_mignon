use crate::effect::StopPolicy;
use crate::pre_triage_filter::FilterReason;
use crate::state::JobOrigin;
use crate::tabs::JobListMode;
use crate::{
    JobId, JobResultKind, RunCompletionNotice, RunProgressView, RunState, Stage, UnfinishedWork,
};
use chrono::{DateTime, Utc};
use harvester_engine::llm::dto::SourceTier;
use harvester_engine::LinkKind;
use serde::{Deserialize, Serialize};

// This token limit is the recommended limit to be used when creating an archive.
pub const TOKEN_LIMIT: u64 = 100_000;

/// Per-model LLM token usage snapshot for rendering.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LlmModelUsageView {
    pub model: String,
    pub input_tokens: u64,
    pub output_tokens: u64,
}

pub use crate::llm_quota_view::LlmQuotaView;

/// Maximum desktop job-list rows in one snapshot.
pub const DESKTOP_JOB_LIST_MAX_ROWS: usize = 400;

/// Rolling window size for the Last24Hours desktop job-list mode.
pub const DESKTOP_JOB_LIST_RECENT_WINDOW_HOURS: i64 = 24;

#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct LastPasteStats {
    pub enqueued: usize,
    pub skipped: usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ScoreBand {
    High,
    Mid,
    Low,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum SignalCandidateRowState {
    Scoring,
    Scored,
    Failed { reason: String },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum SignalCandidateOutcome {
    /// >= threshold AND the cluster representative -> goes to the archive.
    Selected,
    /// >= threshold but lost to another representative of the same signal_key.
    Deduplicated { kept_gist: String },
    /// score < threshold.
    BelowThreshold,
    /// signal_key manually excluded at the active prompt version.
    Excluded,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SignalCandidateRow {
    pub job_id: JobId,
    pub url: String,
    pub score: u8,
    pub score_band: ScoreBand,
    pub source_tier: SourceTier,
    pub themes: Vec<String>,
    pub gist_truncated: String,
    pub dupes_count: usize,
    pub state_label: SignalCandidateRowState,
    pub signal_key: String,
    /// Selection outcome for `Scored` rows; `None` for `Scoring`/`Failed`.
    pub outcome: Option<SignalCandidateOutcome>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum StopFinishButtonState {
    #[default]
    Disabled,
    Enabled {
        policy: StopPolicy,
    },
}

impl StopFinishButtonState {
    pub fn is_enabled(self) -> bool {
        matches!(self, Self::Enabled { .. })
    }

    pub fn policy(self) -> Option<StopPolicy> {
        match self {
            Self::Disabled => None,
            Self::Enabled { policy } => Some(policy),
        }
    }
}

/// Summary body rendered by the reading pane.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct RightPaneView {
    pub summary_markdown: Option<String>,
}

/// Default desktop window dimensions.
pub const DEFAULT_WINDOW_WIDTH: i32 = 960;
pub const DEFAULT_WINDOW_HEIGHT: i32 = 720;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ArchivePartialCoverageView {
    pub triaged: usize,
    pub actionable_total: usize,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReprocessNoticeView {
    pub articles: usize,
    pub estimated_calls: u64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AppViewModel {
    pub job_count: usize,
    pub desktop_job_list: DesktopJobListView,
    pub last_paste_stats: Option<LastPasteStats>,
    pub token_limit: u64,
    /// Summary-mode archive size over the filtered corpus: cached summary tokens
    /// where available, raw article tokens otherwise. Drives the token meter bar.
    pub archive_token_estimate: u64,
    /// Number of articles in the filtered archive corpus.
    pub archive_filtered_count: usize,
    /// Cache-derived triage coverage shown alongside the archive count meter.
    pub archive_partial_coverage: Option<ArchivePartialCoverageView>,
    /// Archive-eligible articles that lack a cached summary (`filtered` minus
    /// the summary-coverage count).
    pub raw_unprocessed_count: usize,
    pub stop_finish_button: StopFinishButtonState,
    pub signal_candidate_rows: Vec<SignalCandidateRow>,
    pub ai_unavailable_message: Option<String>,
    /// Reducer-owned cumulative timeline for the desktop pipeline experience.
    pub run_progress: RunProgressView,
    pub archive_enabled: bool,
    pub run_state: RunState,
    pub run_completion_notice: Option<RunCompletionNotice>,
    pub run_enabled: bool,
    pub resume_enabled: bool,
    /// Core-authored tooltip text when Process unfinished is disabled.
    pub resume_disabled_reason: Option<String>,
    /// Stored reducer summary; the view never classifies work.
    pub unfinished_work: UnfinishedWork,
    pub reprocess_notice: Option<ReprocessNoticeView>,
    pub checkpoint_status_message: Option<String>,
    /// Session LLM call quota meter.
    pub llm_quota: LlmQuotaView,
    /// Right-pane tab content area view.
    pub right_pane: RightPaneView,
}

impl Default for AppViewModel {
    fn default() -> Self {
        Self {
            job_count: 0,
            desktop_job_list: DesktopJobListView::default(),
            last_paste_stats: None,
            token_limit: TOKEN_LIMIT,
            archive_token_estimate: 0,
            archive_filtered_count: 0,
            archive_partial_coverage: None,
            raw_unprocessed_count: 0,
            stop_finish_button: StopFinishButtonState::Disabled,
            signal_candidate_rows: Vec::new(),
            ai_unavailable_message: None,
            run_progress: RunProgressView::default(),
            archive_enabled: true,
            run_state: RunState::Idle,
            run_completion_notice: None,
            run_enabled: true,
            resume_enabled: false,
            resume_disabled_reason: Some("Unfinished work is not known yet.".to_string()),
            unfinished_work: UnfinishedWork::Unknown,
            reprocess_notice: None,
            checkpoint_status_message: None,
            llm_quota: crate::build_llm_quota_view(&crate::LlmQuotaState::default()),
            right_pane: RightPaneView::default(),
        }
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn blacklist_records_active_cooldown_and_failure() {
        use crate::blacklist::BlacklistState;
        use harvester_engine::FetchOutcomeClass;
        let now = chrono::Utc::now();
        let mut bl = BlacklistState::default();
        for _ in 0..3 {
            bl.record_outcome(
                "bloomberg.com",
                FetchOutcomeClass::PermanentBlock,
                Some("http status 403"),
                now,
            );
        }
        assert!(bl.is_blocked("bloomberg.com", now));
        let rows = bl.rows();
        assert_eq!(rows.len(), 1);
        let (domain, record) = rows[0];
        assert_eq!(domain, "bloomberg.com");
        assert_eq!(record.strikes, 3);
        assert_eq!(record.last_failure_kind.as_deref(), Some("http status 403"));
        let until = record.cooldown_until.expect("cooldown armed");
        assert!(until > now);
        assert!(!bl.is_blocked("bloomberg.com", until));
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct JobRowView {
    pub job_id: JobId,
    pub url: String,
    pub stage: Stage,
    pub outcome: Option<JobResultKind>,
    pub tokens: Option<u32>,
    pub bytes: Option<u64>,
    pub link_count: usize,
    pub links: Vec<LinkRowView>,
    pub origin: JobOrigin,
    pub triage_annotation: Option<TriageAnnotationView>,
    pub has_summary: bool,
    pub summary_title: Option<String>,
    pub summary_tokens: Option<u32>,
    pub filter_status: Option<JobFilterStatus>,
    pub has_analysis: bool,
    pub is_since_checkpoint: bool,
}

/// The rows the desktop page can actually show, plus the selected job.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct DesktopJobListView {
    pub mode: JobListMode,
    pub query: String,
    pub rows: Vec<JobListRowView>,
    pub selected_job: Option<SelectedJobView>,
    /// Rows in scope after mode and search, before the cap.
    pub scoped_count: usize,
    /// `rows.len()`.
    pub visible_count: usize,
    /// `scoped_count > visible_count`.
    pub truncated: bool,
    /// Jobs excluded from a time-based scope only because they carry no
    /// `fetched_utc`. Zero when the relevant time reference is unavailable and
    /// in Results mode.
    pub hidden_without_fetch_time: usize,
}

/// A list row: `JobRowView` without `links`, plus the fetch time.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct JobListRowView {
    pub job_id: JobId,
    pub url: String,
    pub stage: Stage,
    pub outcome: Option<JobResultKind>,
    pub tokens: Option<u32>,
    pub bytes: Option<u64>,
    pub link_count: usize,
    pub origin: JobOrigin,
    pub triage_annotation: Option<TriageAnnotationView>,
    pub has_summary: bool,
    pub summary_title: Option<String>,
    pub summary_tokens: Option<u32>,
    pub filter_status: Option<JobFilterStatus>,
    pub has_analysis: bool,
    pub is_since_checkpoint: bool,
    pub fetched_utc: Option<DateTime<Utc>>,
}

impl JobListRowView {
    pub fn from_row(row: &JobRowView, fetched_utc: Option<DateTime<Utc>>) -> Self {
        Self {
            job_id: row.job_id,
            url: row.url.clone(),
            stage: row.stage,
            outcome: row.outcome.clone(),
            tokens: row.tokens,
            bytes: row.bytes,
            link_count: row.link_count,
            origin: row.origin.clone(),
            triage_annotation: row.triage_annotation.clone(),
            has_summary: row.has_summary,
            summary_title: row.summary_title.clone(),
            summary_tokens: row.summary_tokens,
            filter_status: row.filter_status.clone(),
            has_analysis: row.has_analysis,
            is_since_checkpoint: row.is_since_checkpoint,
            fetched_utc,
        }
    }
}

/// Why the selected job is, or is not, in the rendered list.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum SelectedJobVisibility {
    /// Rendered in the current list — a job row, or (in Results mode) a
    /// candidate row in `signal_candidate_rows`.
    Visible,
    /// Excluded by the mode's scope: before the checkpoint, or, in Results
    /// mode, not a signal candidate at all.
    OutsideScope,
    /// In scope, but excluded by the active search query.
    QueryMismatch,
    /// In scope and matching the query, but not among the newest rows the cap kept.
    Capped,
}

/// Everything the reading pane needs for one job, in or out of scope.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SelectedJobView {
    pub job_id: JobId,
    pub url: String,
    pub summary_title: Option<String>,
    pub stage: Stage,
    pub outcome: Option<JobResultKind>,
    pub tokens: Option<u32>,
    pub bytes: Option<u64>,
    pub origin: JobOrigin,
    pub triage_annotation: Option<TriageAnnotationView>,
    pub has_summary: bool,
    pub summary_tokens: Option<u32>,
    pub filter_status: Option<JobFilterStatus>,
    pub fetched_utc: Option<DateTime<Utc>>,
    /// Why the selection is, or is not, in the rendered list.
    pub list_visibility: SelectedJobVisibility,
    pub links: Vec<LinkRowView>,
}

impl SelectedJobView {
    pub fn from_row(
        row: &JobRowView,
        fetched_utc: Option<DateTime<Utc>>,
        list_visibility: SelectedJobVisibility,
    ) -> Self {
        Self {
            job_id: row.job_id,
            url: row.url.clone(),
            summary_title: row.summary_title.clone(),
            stage: row.stage,
            outcome: row.outcome.clone(),
            tokens: row.tokens,
            bytes: row.bytes,
            origin: row.origin.clone(),
            triage_annotation: row.triage_annotation.clone(),
            has_summary: row.has_summary,
            summary_tokens: row.summary_tokens,
            filter_status: row.filter_status.clone(),
            fetched_utc,
            list_visibility,
            links: row.links.clone(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum JobFilterStatus {
    HardExcluded { reasons: Vec<FilterReason> },
    ReviewNeeded { reasons: Vec<FilterReason> },
    AutoIncluded,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LinkRowView {
    pub index: u32,
    pub url: String,
    pub label: String,
    pub kind: LinkKind,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TriageAnnotationView {
    pub priority: u8,
    pub category: String,
    pub tags: Vec<String>,
}
