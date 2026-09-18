use crate::effect::StopPolicy;
use crate::pre_triage_filter::FilterReason;
use crate::preview::PreviewContentKind;
use crate::state::{JobOrigin, LinkDownloadState};
use crate::tabs::{JobListMode, TrendCategory};
use crate::trends::{CategoryTrend, EntityTrendData};
use crate::{JobId, JobResultKind, RunCompletionNotice, RunProgressView, SessionState, Stage};
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

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SignalCandidatePreviewView {
    pub signal_key: String,
    pub duplicate_urls: Vec<String>,
    pub exclude_checked: bool,
    pub state_label: String,
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

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PreviewHeaderView {
    pub domain: String,
    pub tokens: Option<u32>,
    pub bytes: Option<u64>,
    pub stage: Stage,
    pub outcome: Option<JobResultKind>,
    pub heading_count: usize,
    pub link_density: f64,
    pub nav_heavy: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LeftPaneHeaderView {
    pub title: String,
    pub scope_label: Option<String>,
    pub count_label: Option<String>,
    pub state_label: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum IndirectLinkPhase {
    Collecting,
    Ready,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct IndirectLinkSummary {
    pub count: usize,
    pub phase: IndirectLinkPhase,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PreviewContextView {
    pub source_label: String,
    pub status_label: String,
    pub attention_label: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct InlineWarningView {
    pub title: String,
    pub body: String,
}

/// View data for one entity in the trends tab.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EntityLineView {
    pub label: String,
    pub weekly_counts: Vec<u32>,
    pub total_count: u32,
}

/// View data for one category in the trends tab.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CategoryTrendView {
    /// Week display labels (oldest first).
    pub weeks: Vec<String>,
    /// Top-N entities.
    pub lines: Vec<EntityLineView>,
    /// Total number of entities (before top-N truncation).
    pub total_entity_count: usize,
}

/// View state for the Trends tab.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TrendsTabView {
    /// True when entity index has not yet loaded or been rebuilt.
    pub is_loading: bool,
    /// The currently selected trend category.
    pub active_category: TrendCategory,
    /// Data for the selected category; `None` when `is_loading` is true.
    pub category_data: Option<CategoryTrendView>,
}

impl Default for TrendsTabView {
    fn default() -> Self {
        Self {
            is_loading: true,
            active_category: TrendCategory::default(),
            category_data: None,
        }
    }
}

/// View state for the right-pane tab content area.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct RightPaneView {
    /// Markdown content for the Triage tab (formatted triage result).
    pub triage_markdown: Option<String>,
    /// Markdown content for the Summary tab.
    pub summary_markdown: Option<String>,
    /// Trends tab view data.
    pub trends: TrendsTabView,
    /// Formatted text for the Poll Stats tab. None until the first poll completes.
    pub poll_stats_markdown: Option<String>,
}

fn category_trend_to_view(trend: &CategoryTrend) -> CategoryTrendView {
    CategoryTrendView {
        weeks: trend.weeks.iter().map(|w| w.label.clone()).collect(),
        lines: trend
            .top_entities
            .iter()
            .map(|e| EntityLineView {
                label: e.display_label.clone(),
                weekly_counts: e.weekly_counts.clone(),
                total_count: e.total_count,
            })
            .collect(),
        total_entity_count: trend.total_entity_count,
    }
}

pub(crate) fn build_trends_tab_view(
    entity_trend_data: Option<&EntityTrendData>,
    active_category: TrendCategory,
) -> TrendsTabView {
    match entity_trend_data {
        None => TrendsTabView {
            is_loading: true,
            active_category,
            category_data: None,
        },
        Some(data) => {
            let trend = match active_category {
                TrendCategory::Companies => &data.companies,
                TrendCategory::Technologies => &data.technologies,
                TrendCategory::Products => &data.products,
                TrendCategory::Themes => &data.themes,
            };
            TrendsTabView {
                is_loading: false,
                active_category,
                category_data: Some(category_trend_to_view(trend)),
            }
        }
    }
}

/// Default desktop window dimensions.
pub const DEFAULT_WINDOW_WIDTH: i32 = 960;
pub const DEFAULT_WINDOW_HEIGHT: i32 = 720;

/// View state for the desktop job-list controls.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct LeftPaneView {
    /// Current Jobs-tab search query.
    pub jobs_search_query: String,
    /// First job visible in the desktop job list.
    pub first_visible_job_id: Option<JobId>,
    /// Whether the selected job remains visible under the Jobs-tab filter.
    pub selected_jobs_visible_in_filter: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ArchivePartialCoverageView {
    pub triaged: usize,
    pub actionable_total: usize,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AppViewModel {
    pub workspace_view: crate::WorkspaceView,
    pub job_list_mode: crate::JobListMode,
    pub session: SessionState,
    pub queued_urls: Vec<String>,
    pub job_count: usize,
    pub desktop_job_list: DesktopJobListView,
    pub last_paste_stats: Option<LastPasteStats>,
    pub dirty: bool,
    pub total_tokens: u64,
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
    pub preview_text: Option<String>,
    pub selected_job_id: Option<crate::JobId>,
    pub left_pane_header: LeftPaneHeaderView,
    pub preview_header: Option<PreviewHeaderView>,
    pub preview_context: Option<PreviewContextView>,
    pub ai_warning_banner: Option<InlineWarningView>,
    pub preview_header_text: Option<String>,
    pub preview_source: Option<PreviewContentKind>,
    pub briefing_generate_enabled: bool,
    pub next_item_enabled: bool,
    pub summaries_can_start: bool,
    pub stop_finish_button: StopFinishButtonState,
    pub triage_can_start: bool,
    pub triage_results_reorder_suppressed: bool,
    pub signal_candidate_rows: Vec<SignalCandidateRow>,
    pub signal_candidate_preview: Option<SignalCandidatePreviewView>,
    pub ai_unavailable_message: Option<String>,
    pub triage_blocked_reason: Option<String>,
    pub briefing_blocked_reason: Option<String>,
    /// Reducer-owned cumulative timeline for the desktop pipeline experience.
    pub run_progress: RunProgressView,
    pub run_completion_notice: Option<RunCompletionNotice>,
    pub poll_sources_enabled: bool,
    pub poll_indirect_links_enabled: bool,
    pub checkpoint_status_message: Option<String>,
    /// URL of the currently selected job, only when it has a completed summary.
    pub selected_url: Option<String>,
    pub left_pane: LeftPaneView,
    pub is_pre_triage_reviewing: bool,
    pub indirect_link_summary: Option<IndirectLinkSummary>,
    /// Per-model LLM token usage, sorted alphabetically by model name. Only Miss runs counted.
    pub llm_usage_by_model: Vec<LlmModelUsageView>,
    /// Session LLM call quota meter.
    pub llm_quota: LlmQuotaView,
    /// Right-pane tab content area view.
    pub right_pane: RightPaneView,
    /// Blacklist tab content.
    pub blacklist: BlacklistTabView,
}

impl Default for AppViewModel {
    fn default() -> Self {
        Self {
            workspace_view: crate::WorkspaceView::default(),
            job_list_mode: crate::JobListMode::default(),
            session: SessionState::Idle,
            queued_urls: Vec::new(),
            job_count: 0,
            desktop_job_list: DesktopJobListView::default(),
            last_paste_stats: None,
            dirty: false,
            total_tokens: 0,
            token_limit: TOKEN_LIMIT,
            archive_token_estimate: 0,
            archive_filtered_count: 0,
            archive_partial_coverage: None,
            raw_unprocessed_count: 0,
            preview_text: None,
            selected_job_id: None,
            left_pane_header: LeftPaneHeaderView {
                title: "Jobs".to_string(),
                scope_label: None,
                count_label: None,
                state_label: None,
            },
            preview_header: None,
            preview_context: None,
            ai_warning_banner: None,
            preview_header_text: None,
            preview_source: None,
            briefing_generate_enabled: false,
            next_item_enabled: false,
            summaries_can_start: false,
            stop_finish_button: StopFinishButtonState::Disabled,
            triage_can_start: false,
            triage_results_reorder_suppressed: false,
            signal_candidate_rows: Vec::new(),
            signal_candidate_preview: None,
            ai_unavailable_message: None,
            triage_blocked_reason: None,
            briefing_blocked_reason: None,
            run_progress: RunProgressView::default(),
            run_completion_notice: None,
            poll_sources_enabled: false,
            poll_indirect_links_enabled: false,
            checkpoint_status_message: None,
            selected_url: None,
            left_pane: LeftPaneView::default(),
            is_pre_triage_reviewing: false,
            indirect_link_summary: None,
            llm_usage_by_model: Vec::new(),
            llm_quota: crate::build_llm_quota_view(&crate::LlmQuotaState::default()),
            right_pane: RightPaneView::default(),
            blacklist: BlacklistTabView::default(),
        }
    }
}

// ---------------------------------------------------------------------------
// Blacklist tab view types
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BlacklistRowView {
    pub domain: String,
    pub strikes: u32,
    pub status: String,
    pub last_failure: String,
    pub next_retry: String,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct BlacklistTabView {
    pub rows: Vec<BlacklistRowView>,
    pub blacklisted_count: usize,
}

impl BlacklistTabView {
    pub fn from_state(
        state: &crate::blacklist::BlacklistState,
        now: chrono::DateTime<chrono::Utc>,
    ) -> Self {
        let mut blacklisted_count = 0;
        let rows = state
            .rows()
            .into_iter()
            .map(|(domain, rec)| {
                let blocked = state.is_blocked(domain, now);
                if blocked {
                    blacklisted_count += 1;
                }
                let status = match rec.cooldown_until {
                    Some(until) if now < until => format!("Cooling down ({} strikes)", rec.strikes),
                    Some(_) => "Probe pending".to_string(),
                    None => "Tracking".to_string(),
                };
                let next_retry = match rec.cooldown_until {
                    Some(until) if now < until => until.format("%Y-%m-%d %H:%M UTC").to_string(),
                    _ => "—".to_string(),
                };
                BlacklistRowView {
                    domain: domain.clone(),
                    strikes: rec.strikes,
                    status,
                    last_failure: rec
                        .last_failure_kind
                        .clone()
                        .unwrap_or_else(|| "—".to_string()),
                    next_retry,
                }
            })
            .collect();
        BlacklistTabView {
            rows,
            blacklisted_count,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn blacklist_view_marks_active_and_cooldown() {
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
        let view = BlacklistTabView::from_state(&bl, now);
        assert_eq!(view.blacklisted_count, 1);
        let row = &view.rows[0];
        assert_eq!(row.domain, "bloomberg.com");
        assert_eq!(row.strikes, 3);
        assert!(
            row.status.to_lowercase().contains("cool")
                || row.status.to_lowercase().contains("blacklist")
        );
        assert_eq!(row.last_failure, "http status 403");
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
    pub downloaded_link_count: usize,
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
    pub downloaded_link_count: usize,
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
            downloaded_link_count: row.downloaded_link_count,
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
    ManuallyExcluded,
    ManuallyIncluded,
    AutoIncluded,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LinkRowView {
    pub index: u32,
    pub url: String,
    pub label: String,
    pub kind: LinkKind,
    pub download_state: LinkDownloadState,
    pub age_suspect: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TriageAnnotationView {
    pub priority: u8,
    pub category: String,
    pub tags: Vec<String>,
}
