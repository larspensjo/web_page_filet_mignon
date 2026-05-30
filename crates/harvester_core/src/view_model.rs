use crate::effect::StopPolicy;
use crate::pre_triage_filter::FilterReason;
use crate::preview::PreviewContentKind;
use crate::prompt_lab::{
    prompt_id_for_stage, ModelCatalogSource, PromptLabCompareBatchRecord,
    PromptLabCompareBatchStatus, PromptLabInputSource, PromptLabRunId, PromptLabRunStatus,
    PromptLabStage, PromptLabState, PromptLabTemplateSnapshot,
};
use crate::state::{JobOrigin, LinkDownloadState};
use crate::tabs::{AppTab, JobListScope, LeftTab, TrendCategory};
use crate::trends::{CategoryTrend, EntityTrendData};
use crate::{serialize_pairs, JobId, JobResultKind, SessionState, Stage};
use harvester_engine::llm::dto::SourceTier;
use harvester_engine::llm::prompt::{PromptId, PromptVersion, TemplateSource};
use harvester_engine::llm::types::ModelId;
use harvester_engine::LinkKind;
use std::collections::HashMap;

// This token limit is the recommended limit to be used when creating an archive.
pub const TOKEN_LIMIT: u64 = 100_000;

/// Per-model LLM token usage snapshot for rendering.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LlmModelUsageView {
    pub model: String,
    pub input_tokens: u64,
    pub output_tokens: u64,
}

pub use crate::llm_quota_view::LlmQuotaView;

pub const INPUT_PANEL_FIXED_WIDTH: i32 = 500;
pub const MIN_JOBS_PANEL_WIDTH: i32 = 200;

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct LastPasteStats {
    pub enqueued: usize,
    pub skipped: usize,
}

/// Progress for the single active operation shown in the footer bar.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OperationProgress {
    pub label: String,
    pub completed: u32,
    pub total: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ScoreBand {
    High,
    Mid,
    Low,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SignalCandidateRowState {
    Scoring,
    Scored,
    Failed { reason: String },
}

#[derive(Debug, Clone, PartialEq, Eq)]
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

#[derive(Debug, Clone, PartialEq, Eq)]
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

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SignalCandidatePreviewView {
    pub signal_key: String,
    pub duplicate_urls: Vec<String>,
    pub exclude_checked: bool,
    pub state_label: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
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

#[derive(Debug, Clone, PartialEq)]
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

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LeftPaneHeaderView {
    pub title: String,
    pub scope_label: Option<String>,
    pub count_label: Option<String>,
    pub state_label: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum IndirectLinkPhase {
    Collecting,
    Ready,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IndirectLinkSummary {
    pub count: usize,
    pub phase: IndirectLinkPhase,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PreviewContextView {
    pub source_label: String,
    pub status_label: String,
    pub attention_label: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InlineWarningView {
    pub title: String,
    pub body: String,
}

/// View data for one entity in the trends tab.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EntityLineView {
    pub label: String,
    pub weekly_counts: Vec<u32>,
    pub total_count: u32,
}

/// View data for one category in the trends tab.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CategoryTrendView {
    /// Week display labels (oldest first).
    pub weeks: Vec<String>,
    /// Top-N entities.
    pub lines: Vec<EntityLineView>,
    /// Total number of entities (before top-N truncation).
    pub total_entity_count: usize,
}

/// View state for the Trends tab.
#[derive(Debug, Clone, PartialEq)]
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
#[derive(Debug, Clone, PartialEq, Default)]
pub struct RightPaneView {
    /// Which tab is currently active.
    pub active_tab: AppTab,
    /// Markdown content for the Triage tab (formatted triage result).
    pub triage_markdown: Option<String>,
    /// Markdown content for the Summary tab.
    pub summary_markdown: Option<String>,
    /// Markdown content for the Briefing tab.
    pub briefing_markdown: Option<String>,
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

// Default left panel width when the input panel is shown (PANEL_INPUT + PANEL_JOBS = 240 + 440)
pub const DEFAULT_LEFT_PANEL_WIDTH: i32 = 680;
pub const DEFAULT_JOBS_PANEL_WIDTH: i32 = DEFAULT_LEFT_PANEL_WIDTH - INPUT_PANEL_FIXED_WIDTH;
// Default window width
pub const DEFAULT_WINDOW_WIDTH: i32 = 960;

/// View state for the left-pane tab bar and its content.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct LeftPaneView {
    /// Which left-pane tab is currently active.
    pub left_tab: LeftTab,
    /// Scope filter applied to job-oriented tabs.
    pub job_list_scope: JobListScope,
    /// Current Jobs-tab search query.
    pub jobs_search_query: String,
    /// Job IDs visible on the Jobs tab after scope and search filtering.
    pub visible_jobs_after_filter: Vec<JobId>,
    /// First entry in `visible_jobs_after_filter`.
    pub first_visible_job_id: Option<JobId>,
    /// Whether the selected job remains visible under the Jobs-tab filter.
    pub selected_jobs_visible_in_filter: bool,
    /// Prompt Lab controls (shown when left_tab == PromptLab).
    pub prompt_lab: PromptLabView,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LayoutViewModel {
    pub left_panel_width: i32,
    pub input_panel_visible: bool,
    pub operation_progress_visible: bool,
    pub active_tab: AppTab,
    pub left_tab: LeftTab,
    pub left_header_meta_visible: bool,
    pub ai_warning_banner_visible: bool,
    pub preview_header_override_visible: bool,
    pub preview_context_visible: bool,
    pub preview_attention_visible: bool,
    pub signal_candidate_preview_visible: bool,
    pub prompt_lab_advanced_mode: bool,
    pub prompt_lab_compare_section_open: bool,
    pub prompt_lab_context_section_open: bool,
    pub prompt_lab_template_section_open: bool,
    pub prompt_lab_run_details_section_open: bool,
    pub prompt_lab_template_editor_open: bool,
}

#[derive(Debug, Clone, PartialEq)]
pub struct AppViewModel {
    pub session: SessionState,
    pub queued_urls: Vec<String>,
    pub job_count: usize,
    pub jobs: Vec<JobRowView>,
    pub last_paste_stats: Option<LastPasteStats>,
    pub dirty: bool,
    pub total_tokens: u64,
    pub token_limit: u64,
    /// Summary-mode archive size over the filtered corpus: cached summary tokens
    /// where available, raw article tokens otherwise. Drives the token meter bar.
    pub archive_token_estimate: u64,
    /// Number of articles in the filtered archive corpus.
    pub archive_filtered_count: usize,
    /// Successfully downloaded jobs (`Stage::Done` + `Success` + `tokens.is_some()`)
    /// that have no cached summary.
    pub raw_unprocessed_count: usize,
    pub preview_text: Option<String>,
    pub selected_job_id: Option<crate::JobId>,
    pub left_pane_header: LeftPaneHeaderView,
    pub preview_header: Option<PreviewHeaderView>,
    pub preview_context: Option<PreviewContextView>,
    pub ai_warning_banner: Option<InlineWarningView>,
    pub preview_header_text: Option<String>,
    pub preview_source: Option<PreviewContentKind>,
    pub briefing_can_start: bool,
    pub briefing_preview: Option<String>,
    pub stop_finish_button: StopFinishButtonState,
    pub triage_can_start: bool,
    pub triage_results_reorder_suppressed: bool,
    pub signal_candidate_rows: Vec<SignalCandidateRow>,
    pub signal_candidate_preview: Option<SignalCandidatePreviewView>,
    pub ai_unavailable_message: Option<String>,
    pub triage_blocked_reason: Option<String>,
    pub briefing_blocked_reason: Option<String>,
    pub operation_progress: Option<OperationProgress>,
    pub poll_sources_enabled: bool,
    pub poll_indirect_links_enabled: bool,
    pub operation_progress_visible: bool,
    pub checkpoint_status_message: Option<String>,
    /// Width of the left panels region (PANEL_INPUT + PANEL_JOBS).
    pub left_panel_width: i32,
    /// Whether the dropbox/input panel is currently visible.
    pub input_panel_visible: bool,
    /// Current window width.
    pub window_width: i32,
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
}

impl Default for AppViewModel {
    fn default() -> Self {
        Self {
            session: SessionState::Idle,
            queued_urls: Vec::new(),
            job_count: 0,
            jobs: Vec::new(),
            last_paste_stats: None,
            dirty: false,
            total_tokens: 0,
            token_limit: TOKEN_LIMIT,
            archive_token_estimate: 0,
            archive_filtered_count: 0,
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
            briefing_can_start: false,
            briefing_preview: None,
            stop_finish_button: StopFinishButtonState::Disabled,
            triage_can_start: false,
            triage_results_reorder_suppressed: false,
            signal_candidate_rows: Vec::new(),
            signal_candidate_preview: None,
            ai_unavailable_message: None,
            triage_blocked_reason: None,
            briefing_blocked_reason: None,
            operation_progress: None,
            poll_sources_enabled: false,
            poll_indirect_links_enabled: false,
            operation_progress_visible: false,
            checkpoint_status_message: None,
            left_panel_width: DEFAULT_LEFT_PANEL_WIDTH,
            input_panel_visible: false,
            window_width: DEFAULT_WINDOW_WIDTH,
            selected_url: None,
            left_pane: LeftPaneView::default(),
            is_pre_triage_reviewing: false,
            indirect_link_summary: None,
            llm_usage_by_model: Vec::new(),
            llm_quota: crate::build_llm_quota_view(&crate::LlmQuotaState::default()),
            right_pane: RightPaneView::default(),
        }
    }
}

// ---------------------------------------------------------------------------
// Prompt Lab view types
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PromptLabRunSummaryView {
    pub run_id: PromptLabRunId,
    pub stage: PromptLabStage,
    pub status_label: &'static str,
    pub output_json: Option<String>,
    pub failure_reason: Option<String>,
    pub input_tokens: Option<u32>,
    pub output_tokens: Option<u32>,
    /// Cost in microdollars (None if metadata not available).
    pub cost_microdollars: Option<u64>,
    /// Wall time in milliseconds (None if metadata not available).
    pub wall_ms: Option<u64>,
    /// Resolved model name (None if metadata not available).
    pub resolved_model: Option<String>,
    /// Whether the response parsed and validated successfully.
    pub parse_ok: Option<bool>,
    /// Cache hit/miss status as a display string (None if metadata not available).
    pub cache_status: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PromptLabView {
    pub visible: bool,
    pub advanced_mode: bool,
    pub compare_section_open: bool,
    pub context_section_open: bool,
    pub template_section_open: bool,
    pub run_details_section_open: bool,
    pub selected_stage: PromptLabStage,
    pub input_is_set: bool,
    pub is_in_flight: bool,
    pub run_count: usize,
    pub latest_run: Option<PromptLabRunSummaryView>,
    pub selected_input_source: PromptLabInputSource,
    pub url_input: String,
    pub can_run: bool,
    pub can_rerun: bool,
    pub run_disabled_reason: Option<&'static str>,
    pub resolve_pending: bool,
    pub url_resolve_failed: bool,
    pub latest_validation_error: Option<String>,
    pub context_draft_text: String,
    pub context_validation_errors: Vec<String>,
    pub context_dirty: bool,
    pub context_applied: bool,
    pub can_apply_context: bool,
    pub can_apply_and_rerun: bool,
    pub can_revert_context: bool,
    pub can_save_context: bool,
    pub context_status_message: Option<String>,
    pub template_editor_open: bool,
    pub template_snapshot_description: Option<String>,
    pub template_snapshot_expected_format: Option<String>,
    pub template_snapshot_source: Option<TemplateSource>,
    pub template_snapshot_version: Option<PromptVersion>,
    pub template_system_draft: String,
    pub template_user_draft: String,
    pub template_validation_errors: Vec<String>,
    pub template_dirty: bool,
    pub template_applied: bool,
    pub template_saved_version: Option<PromptVersion>,
    pub template_saved_path: Option<String>,
    pub draft_candidates: Vec<PromptLabCompareCandidateView>,
    pub active_batch: Option<PromptLabCompareBatchView>,
    pub can_add_candidate: bool,
    pub can_reset_draft: bool,
    pub selected_model_override: Option<ModelId>,
    pub model_catalog: Vec<ModelId>,
    pub model_catalog_source: ModelCatalogSource,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PromptLabCompareCandidateView {
    pub candidate_id: u64,
    pub label: String,
    pub stage_label: String,
    pub model_label: String,
    pub prompt_version_label: String,
    pub has_context_override: bool,
    pub has_template_override: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PromptLabCompareRowView {
    pub candidate_id: u64,
    pub label: String,
    pub run_id: Option<PromptLabRunId>,
    pub status_label: String,
    pub model_label: String,
    pub cost_label: String,
    pub wall_label: String,
    pub tokens_label: String,
    pub parse_ok: Option<bool>,
    pub rating: Option<u8>,
    pub is_manual_winner: bool,
    pub is_auto_winner: bool,
    pub rank: Option<usize>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PromptLabComparePolicyView {
    pub require_parse_ok: bool,
    pub max_cost_label: String,
    pub max_wall_label: String,
    pub rating_beats_cost: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PromptLabCompareBatchView {
    pub batch_id_label: String,
    pub status_label: String,
    pub warning: Option<String>,
    pub auto_select_warning: Option<String>,
    pub rows: Vec<PromptLabCompareRowView>,
    pub policy: PromptLabComparePolicyView,
    pub can_start: bool,
    pub can_cancel: bool,
    pub can_auto_select: bool,
    pub pending_confirmation: bool,
}

impl Default for PromptLabView {
    fn default() -> Self {
        Self {
            visible: false,
            advanced_mode: false,
            compare_section_open: false,
            context_section_open: false,
            template_section_open: false,
            run_details_section_open: false,
            selected_stage: PromptLabStage::Triage,
            input_is_set: false,
            is_in_flight: false,
            run_count: 0,
            latest_run: None,
            selected_input_source: PromptLabInputSource::default(),
            url_input: String::new(),
            can_run: false,
            can_rerun: false,
            run_disabled_reason: None,
            resolve_pending: false,
            url_resolve_failed: false,
            latest_validation_error: None,
            context_draft_text: String::new(),
            context_validation_errors: Vec::new(),
            context_dirty: false,
            context_applied: false,
            can_apply_context: false,
            can_apply_and_rerun: false,
            can_revert_context: false,
            can_save_context: false,
            context_status_message: None,
            template_editor_open: false,
            template_snapshot_description: None,
            template_snapshot_expected_format: None,
            template_snapshot_source: None,
            template_snapshot_version: None,
            template_system_draft: String::new(),
            template_user_draft: String::new(),
            template_validation_errors: Vec::new(),
            template_dirty: false,
            template_applied: false,
            template_saved_version: None,
            template_saved_path: None,
            draft_candidates: Vec::new(),
            active_batch: None,
            can_add_candidate: true,
            can_reset_draft: false,
            selected_model_override: None,
            model_catalog: Vec::new(),
            model_catalog_source: ModelCatalogSource::default(),
        }
    }
}

impl PromptLabView {
    pub(crate) fn from_state(
        state: &PromptLabState,
        contexts: &HashMap<PromptId, Vec<(String, String)>>,
        templates: &HashMap<PromptId, PromptLabTemplateSnapshot>,
        _selected_triage_article_available: bool,
    ) -> Self {
        let latest_run_record = state.latest_run();
        let latest_run = latest_run_record.map(|r| {
            let metadata = match &r.status {
                PromptLabRunStatus::Pending { .. } => None,
                PromptLabRunStatus::Completed { metadata, .. } => Some(metadata),
                PromptLabRunStatus::Failed { metadata, .. } => metadata.as_ref(),
            };
            let (
                status_label,
                output_json,
                failure_reason,
                input_tokens,
                output_tokens,
                cost_microdollars,
                wall_ms,
                resolved_model,
                parse_ok,
                cache_status,
            ) = match &r.status {
                PromptLabRunStatus::Pending { .. } => (
                    "pending", None, None, None, None, None, None, None, None, None,
                ),
                PromptLabRunStatus::Completed {
                    output_json,
                    metadata,
                } => (
                    "completed",
                    Some(output_json.clone()),
                    None,
                    Some(metadata.input_tokens),
                    Some(metadata.output_tokens),
                    Some(metadata.cost_microdollars),
                    Some(metadata.wall_ms),
                    Some(metadata.resolved_model.clone()),
                    Some(metadata.parse_ok),
                    Some(format!("{:?}", metadata.cache_status).to_lowercase()),
                ),
                PromptLabRunStatus::Failed { reason, .. } => (
                    "failed",
                    None,
                    Some(reason.clone()),
                    metadata.map(|m| m.input_tokens),
                    metadata.map(|m| m.output_tokens),
                    metadata.map(|m| m.cost_microdollars),
                    metadata.map(|m| m.wall_ms),
                    metadata.map(|m| m.resolved_model.clone()),
                    metadata.map(|m| m.parse_ok),
                    metadata.map(|m| format!("{:?}", m.cache_status).to_lowercase()),
                ),
            };
            PromptLabRunSummaryView {
                run_id: r.run_id,
                stage: r.stage,
                status_label,
                output_json,
                failure_reason,
                input_tokens,
                output_tokens,
                cost_microdollars,
                wall_ms,
                resolved_model,
                parse_ok,
                cache_status,
            }
        });
        let selected_input_source = state.selected_input_source();
        let source_reason = if state.url_input().trim().is_empty() {
            Some("Enter URL and resolve input")
        } else if state.resolved_url_snapshot().is_some() {
            None
        } else {
            Some("Resolve URL input")
        };
        let is_in_flight = state.has_in_flight_run();
        let can_run = !is_in_flight && source_reason.is_none();
        let run_disabled_reason = if is_in_flight {
            Some("Running…")
        } else {
            source_reason
        };
        let can_rerun = !is_in_flight
            && latest_run_record
                .map(|run| !matches!(run.status, PromptLabRunStatus::Pending { .. }))
                .unwrap_or(false);
        let resolve_pending = state.pending_resolve_id().is_some();
        let url_resolve_failed = state.last_resolve_failed();
        let latest_validation_error = latest_run_record.and_then(|run| {
            if let PromptLabRunStatus::Failed { reason, .. } = &run.status {
                if reason.to_lowercase().starts_with("validation failed") {
                    Some(reason.clone())
                } else {
                    None
                }
            } else {
                None
            }
        });
        let prompt_id = prompt_id_for_stage(state.selected_stage());
        let context_pairs = contexts
            .get(&prompt_id)
            .map(|pairs| pairs.as_slice())
            .unwrap_or(&[]);
        let context_draft = state.context_draft(prompt_id);
        let context_draft_text = context_draft
            .map(|draft| draft.text().to_string())
            .unwrap_or_else(|| serialize_pairs(context_pairs));
        let context_validation_errors = context_draft
            .map(|draft| {
                draft
                    .validation_errors()
                    .iter()
                    .map(|err| err.to_string())
                    .collect()
            })
            .unwrap_or_default();
        let context_dirty = context_draft.map(|draft| draft.dirty()).unwrap_or(false);
        let context_applied = context_draft.map(|draft| draft.applied()).unwrap_or(false);
        let context_is_valid = context_draft.map(|draft| draft.is_valid()).unwrap_or(true);
        let context_status_message = context_draft
            .and_then(|draft| draft.status_message())
            .map(|msg| msg.to_string());
        let can_save_context = state.can_save_context(prompt_id);
        let can_apply_context = context_is_valid && context_dirty;
        let can_apply_and_rerun = can_run && can_apply_context;
        let can_revert_context = context_dirty || context_applied;
        let template_snapshot = templates.get(&prompt_id);
        let template_snapshot_description =
            template_snapshot.map(|snapshot| snapshot.template.description.clone());
        let template_snapshot_expected_format =
            template_snapshot.map(|snapshot| snapshot.template.expected_format.clone());
        let template_snapshot_source = template_snapshot.map(|snapshot| snapshot.source);
        let template_snapshot_version = template_snapshot.map(|snapshot| snapshot.template.version);
        let template_system_base = template_snapshot
            .map(|snapshot| snapshot.template.system_template.clone())
            .unwrap_or_default();
        let template_user_base = template_snapshot
            .map(|snapshot| snapshot.template.user_template.clone())
            .unwrap_or_default();
        let template_draft = state.template_draft(prompt_id);
        let (
            template_system_draft,
            template_user_draft,
            template_validation_errors,
            template_dirty,
            template_applied,
            template_saved_version,
            template_saved_path,
        ) = if let Some(draft) = template_draft {
            (
                draft.system_draft().to_string(),
                draft.user_draft().to_string(),
                draft
                    .validation_errors()
                    .iter()
                    .map(|err| err.message.clone())
                    .collect(),
                draft.is_dirty(),
                draft.is_applied(),
                draft.saved_version(),
                draft.saved_path().map(|path| path.display().to_string()),
            )
        } else {
            (
                template_system_base.clone(),
                template_user_base.clone(),
                Vec::new(),
                false,
                false,
                None,
                None,
            )
        };
        let draft_candidates = state
            .draft_candidates()
            .iter()
            .map(|candidate| PromptLabCompareCandidateView {
                candidate_id: candidate.candidate_id,
                label: candidate.label.clone(),
                stage_label: format!("{:?}", candidate.stage),
                model_label: candidate
                    .model_override
                    .as_ref()
                    .map(|model| model.model_name().to_string())
                    .unwrap_or_else(|| "default".to_string()),
                prompt_version_label: candidate
                    .prompt_version
                    .map_or_else(|| "active".to_string(), |version| version.to_string()),
                has_context_override: !candidate.context_snapshot.is_empty(),
                has_template_override: candidate.template_snapshot.is_some(),
            })
            .collect::<Vec<_>>();
        let active_batch = state
            .active_batch()
            .map(|batch| compare_batch_view(state, batch));
        let batch_running = state.has_active_batch();
        Self {
            visible: state.is_visible(),
            advanced_mode: state.advanced_mode(),
            compare_section_open: state.compare_section_open(),
            context_section_open: state.context_section_open(),
            template_section_open: state.template_section_open(),
            run_details_section_open: state.run_details_section_open(),
            selected_stage: state.selected_stage(),
            input_is_set: !state.input().is_empty(),
            is_in_flight,
            run_count: state.run_count(),
            latest_run,
            selected_input_source,
            url_input: state.url_input().to_string(),
            can_run,
            can_rerun,
            run_disabled_reason,
            resolve_pending,
            url_resolve_failed,
            latest_validation_error,
            context_draft_text,
            context_validation_errors,
            context_dirty,
            context_applied,
            can_apply_context,
            can_apply_and_rerun,
            can_revert_context,
            can_save_context,
            context_status_message,
            template_editor_open: state.template_editor_open(),
            template_snapshot_description,
            template_snapshot_expected_format,
            template_snapshot_source,
            template_snapshot_version,
            template_system_draft,
            template_user_draft,
            template_validation_errors,
            template_dirty,
            template_applied,
            template_saved_version,
            template_saved_path,
            draft_candidates,
            active_batch,
            can_add_candidate: !batch_running,
            can_reset_draft: !batch_running && !state.draft_candidates().is_empty(),
            selected_model_override: state.selected_model_override().cloned(),
            model_catalog: state.model_catalog().to_vec(),
            model_catalog_source: state.catalog_source(),
        }
    }
}

fn compare_batch_view(
    state: &PromptLabState,
    batch: &PromptLabCompareBatchRecord,
) -> PromptLabCompareBatchView {
    let status_label = match batch.status {
        PromptLabCompareBatchStatus::Running { dispatched, total } => {
            format!("Running {dispatched}/{total}")
        }
        PromptLabCompareBatchStatus::AllComplete => "AllComplete".to_string(),
        PromptLabCompareBatchStatus::PartialFailure => "PartialFailure".to_string(),
        PromptLabCompareBatchStatus::Cancelled => "Cancelled".to_string(),
    };
    let rows =
        batch
            .candidate_run_ids
            .iter()
            .filter_map(|(candidate_id, run_id)| {
                let candidate = batch
                    .candidates
                    .iter()
                    .find(|candidate| candidate.candidate_id == *candidate_id)?;
                let run = run_id.and_then(|run_id| state.run_by_id(run_id));
                let (
                    status_label,
                    cost_label,
                    wall_label,
                    tokens_label,
                    parse_ok,
                    rating,
                    model_label,
                ) = if let Some(run) = run {
                    match &run.status {
                        PromptLabRunStatus::Pending { .. } => (
                            "running".to_string(),
                            "—".to_string(),
                            "—".to_string(),
                            "—".to_string(),
                            None,
                            run.operator_rating,
                            run.model_override
                                .as_ref()
                                .map(|model| model.model_name().to_string())
                                .unwrap_or_else(|| "default".to_string()),
                        ),
                        PromptLabRunStatus::Completed { metadata, .. } => (
                            "ok".to_string(),
                            format!("${:.6}", metadata.cost_microdollars as f64 / 1_000_000.0),
                            format!("{} ms", metadata.wall_ms),
                            format!("{}/{}", metadata.input_tokens, metadata.output_tokens),
                            Some(metadata.parse_ok),
                            run.operator_rating,
                            metadata.resolved_model.clone(),
                        ),
                        PromptLabRunStatus::Failed { metadata, .. } => (
                            "failed".to_string(),
                            metadata
                                .as_ref()
                                .map(|meta| {
                                    format!("${:.6}", meta.cost_microdollars as f64 / 1_000_000.0)
                                })
                                .unwrap_or_else(|| "—".to_string()),
                            metadata
                                .as_ref()
                                .map(|meta| format!("{} ms", meta.wall_ms))
                                .unwrap_or_else(|| "—".to_string()),
                            metadata
                                .as_ref()
                                .map(|meta| format!("{}/{}", meta.input_tokens, meta.output_tokens))
                                .unwrap_or_else(|| "—".to_string()),
                            metadata.as_ref().map(|meta| meta.parse_ok),
                            run.operator_rating,
                            run.model_override
                                .as_ref()
                                .map(|model| model.model_name().to_string())
                                .unwrap_or_else(|| "default".to_string()),
                        ),
                    }
                } else {
                    (
                        "pending".to_string(),
                        "—".to_string(),
                        "—".to_string(),
                        "—".to_string(),
                        None,
                        None,
                        candidate
                            .model_override
                            .as_ref()
                            .map(|model| model.model_name().to_string())
                            .unwrap_or_else(|| "default".to_string()),
                    )
                };
                Some(PromptLabCompareRowView {
                    candidate_id: *candidate_id,
                    label: candidate.label.clone(),
                    run_id: *run_id,
                    status_label,
                    model_label,
                    cost_label,
                    wall_label,
                    tokens_label,
                    parse_ok,
                    rating,
                    is_manual_winner: batch.selected_run_id.is_some()
                        && batch.selected_run_id == *run_id,
                    is_auto_winner: batch.selected_run_id.is_none()
                        && batch.effective_winner().is_some()
                        && batch.effective_winner() == *run_id,
                    rank: None,
                })
            })
            .collect::<Vec<_>>();
    let policy = PromptLabComparePolicyView {
        require_parse_ok: batch.policy.require_parse_ok,
        max_cost_label: batch
            .policy
            .max_cost_microdollars
            .map_or_else(|| "Any".to_string(), |value| value.to_string()),
        max_wall_label: batch
            .policy
            .max_wall_ms
            .map_or_else(|| "Any".to_string(), |value| value.to_string()),
        rating_beats_cost: batch.policy.rating_beats_cost,
    };
    PromptLabCompareBatchView {
        batch_id_label: batch.batch_id.0.to_string(),
        status_label,
        warning: batch.warning.clone(),
        auto_select_warning: batch.auto_select_warning.clone(),
        rows,
        policy,
        can_start: false,
        can_cancel: matches!(batch.status, PromptLabCompareBatchStatus::Running { .. }),
        can_auto_select: true,
        pending_confirmation: false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::prompt_lab::PromptLabRunId;
    use harvester_engine::llm::prompt::PromptId;
    use harvester_engine::llm::run_metadata::LlmRunMetadata;

    fn add_pending_run(state: &mut PromptLabState) -> PromptLabRunId {
        let run_id = PromptLabRunId(1);
        state.add_pending_run(
            run_id,
            PromptLabStage::Triage,
            PromptId::ArticleTriage,
            "input".to_string(),
            1,
            crate::prompt_lab::PromptLabRunOverrides::default(),
        );
        run_id
    }

    fn add_completed_run(state: &mut PromptLabState) -> PromptLabRunId {
        let run_id = add_pending_run(state);
        state.complete_run(run_id, "{}".to_string(), LlmRunMetadata::stub());
        state.consume_ownership(1);
        run_id
    }

    fn add_failed_run(state: &mut PromptLabState, reason: String) -> PromptLabRunId {
        let run_id = add_pending_run(state);
        state.fail_run(run_id, reason, Some(LlmRunMetadata::stub()));
        state.consume_ownership(1);
        run_id
    }

    #[test]
    fn can_run_false_when_in_flight() {
        let mut state = PromptLabState::default();
        add_pending_run(&mut state);
        let contexts = HashMap::new();
        let view = PromptLabView::from_state(&state, &contexts, &HashMap::new(), true);
        assert!(!view.can_run);
        assert!(view.run_disabled_reason.is_some());
    }

    #[test]
    fn can_run_true_for_typeurl_when_snapshot_present() {
        let mut state = PromptLabState::default();
        state.select_input_source(PromptLabInputSource::TypeUrl);
        state.set_url_input("https://example.com".to_string());
        let resolve_id = 1;
        state.begin_url_resolution(resolve_id);
        state.finish_url_resolution(resolve_id, Ok("snapshot".to_string()));
        let contexts = HashMap::new();
        let view = PromptLabView::from_state(&state, &contexts, &HashMap::new(), false);
        assert!(view.can_run);
        assert_eq!(view.run_disabled_reason, None);
    }

    #[test]
    fn can_run_false_for_typeurl_when_snapshot_absent() {
        let mut state = PromptLabState::default();
        state.select_input_source(PromptLabInputSource::TypeUrl);
        let contexts = HashMap::new();
        let view = PromptLabView::from_state(&state, &contexts, &HashMap::new(), false);
        assert!(!view.can_run);
        assert!(view.run_disabled_reason.is_some());
    }

    #[test]
    fn can_run_false_for_typeurl_when_url_set_but_not_resolved() {
        let mut state = PromptLabState::default();
        state.select_input_source(PromptLabInputSource::TypeUrl);
        state.set_url_input("https://example.com".to_string());
        let contexts = HashMap::new();
        let view = PromptLabView::from_state(&state, &contexts, &HashMap::new(), false);
        assert!(!view.can_run);
        assert!(view.run_disabled_reason.is_some());
    }

    #[test]
    fn can_run_false_when_url_input_missing() {
        let state = PromptLabState::default();
        let contexts = HashMap::new();
        let view = PromptLabView::from_state(&state, &contexts, &HashMap::new(), false);
        assert!(!view.can_run);
        assert!(view.run_disabled_reason.is_some());
    }

    #[test]
    fn can_rerun_false_when_in_flight() {
        let mut state = PromptLabState::default();
        add_pending_run(&mut state);
        let contexts = HashMap::new();
        let view = PromptLabView::from_state(&state, &contexts, &HashMap::new(), true);
        assert!(!view.can_rerun);
    }

    #[test]
    fn can_rerun_true_when_latest_run_is_completed() {
        let mut state = PromptLabState::default();
        add_completed_run(&mut state);
        let contexts = HashMap::new();
        let view = PromptLabView::from_state(&state, &contexts, &HashMap::new(), true);
        assert!(view.can_rerun);
    }

    #[test]
    fn metadata_line_present_for_failed_run_with_metadata() {
        let mut state = PromptLabState::default();
        let _run_id = add_failed_run(&mut state, "error".to_string());
        let contexts = HashMap::new();
        let view = PromptLabView::from_state(&state, &contexts, &HashMap::new(), true);
        let latest = view
            .latest_run
            .as_ref()
            .expect("expected latest run summary");
        assert!(latest.failure_reason.is_some());
        assert!(latest.cost_microdollars.is_some());
        assert!(latest.input_tokens.is_some());
    }

    #[test]
    fn validation_error_extracted_when_relevant() {
        let mut state = PromptLabState::default();
        add_failed_run(&mut state, "validation failed: reason".to_string());
        let contexts = HashMap::new();
        let view = PromptLabView::from_state(&state, &contexts, &HashMap::new(), true);
        assert!(view.latest_validation_error.is_some());
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
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

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum JobFilterStatus {
    HardExcluded { reasons: Vec<FilterReason> },
    ReviewNeeded { reasons: Vec<FilterReason> },
    ManuallyExcluded,
    ManuallyIncluded,
    AutoIncluded,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LinkRowView {
    pub index: u32,
    pub url: String,
    pub label: String,
    pub kind: LinkKind,
    pub download_state: LinkDownloadState,
    pub age_suspect: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TriageAnnotationView {
    pub priority: u8,
    pub category: String,
    pub tags: Vec<String>,
}
