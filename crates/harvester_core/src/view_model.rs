use crate::pre_triage_filter::FilterReason;
use crate::preview::PreviewContentKind;
use crate::prompt_lab::{
    prompt_id_for_stage, ModelCatalogSource, PromptLabCompareBatchRecord,
    PromptLabCompareBatchStatus, PromptLabInputSource, PromptLabRunId, PromptLabRunStatus,
    PromptLabStage, PromptLabState, PromptLabTemplateSnapshot,
};
use crate::state::LinkDownloadState;
use crate::{serialize_pairs, JobId, JobResultKind, SessionState, Stage};
use harvester_engine::llm::prompt::{PromptId, PromptVersion, TemplateSource};
use harvester_engine::llm::types::ModelId;
use harvester_engine::LinkKind;
use std::collections::HashMap;

pub const TOKEN_LIMIT: u64 = 200_000;

/// Per-model LLM token usage snapshot for rendering.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LlmModelUsageView {
    pub model: String,
    pub input_tokens: u64,
    pub output_tokens: u64,
}

pub const INPUT_PANEL_FIXED_WIDTH: i32 = 500;
pub const MIN_JOBS_PANEL_WIDTH: i32 = 200;

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct LastPasteStats {
    pub enqueued: usize,
    pub skipped: usize,
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

// Default left panel width when the input panel is shown (PANEL_INPUT + PANEL_JOBS = 240 + 440)
pub const DEFAULT_LEFT_PANEL_WIDTH: i32 = 680;
pub const DEFAULT_JOBS_PANEL_WIDTH: i32 = DEFAULT_LEFT_PANEL_WIDTH - INPUT_PANEL_FIXED_WIDTH;
// Default window width
pub const DEFAULT_WINDOW_WIDTH: i32 = 960;

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
    pub preview_text: Option<String>,
    pub preview_header: Option<PreviewHeaderView>,
    pub preview_source: Option<PreviewContentKind>,
    pub briefing_can_start: bool,
    pub briefing_progress: Option<String>,
    pub briefing_preview: Option<String>,
    pub triage_can_start: bool,
    pub triage_progress: Option<String>,
    pub poll_sources_enabled: bool,
    /// Width of the left panels region (PANEL_INPUT + PANEL_JOBS).
    pub left_panel_width: i32,
    /// Whether the dropbox/input panel is currently visible.
    pub input_panel_visible: bool,
    /// Current window width.
    pub window_width: i32,
    /// URL of the currently selected job, only when it has a completed summary.
    pub selected_url: Option<String>,
    pub prompt_lab: PromptLabView,
    pub is_pre_triage_reviewing: bool,
    /// Per-model LLM token usage, sorted alphabetically by model name. Only Miss runs counted.
    pub llm_usage_by_model: Vec<LlmModelUsageView>,
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
            preview_text: None,
            preview_header: None,
            preview_source: None,
            briefing_can_start: true,
            briefing_progress: None,
            briefing_preview: None,
            triage_can_start: false,
            triage_progress: None,
            poll_sources_enabled: false,
            left_panel_width: DEFAULT_LEFT_PANEL_WIDTH,
            input_panel_visible: false,
            window_width: DEFAULT_WINDOW_WIDTH,
            selected_url: None,
            prompt_lab: PromptLabView::default(),
            is_pre_triage_reviewing: false,
            llm_usage_by_model: Vec::new(),
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
        assert_eq!(view.run_disabled_reason, Some("Running…"));
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
        assert_eq!(
            view.run_disabled_reason,
            Some("Enter URL and resolve input")
        );
    }

    #[test]
    fn can_run_false_for_typeurl_when_url_set_but_not_resolved() {
        let mut state = PromptLabState::default();
        state.select_input_source(PromptLabInputSource::TypeUrl);
        state.set_url_input("https://example.com".to_string());
        let contexts = HashMap::new();
        let view = PromptLabView::from_state(&state, &contexts, &HashMap::new(), false);
        assert!(!view.can_run);
        assert_eq!(view.run_disabled_reason, Some("Resolve URL input"));
    }

    #[test]
    fn can_run_false_when_url_input_missing() {
        let state = PromptLabState::default();
        let contexts = HashMap::new();
        let view = PromptLabView::from_state(&state, &contexts, &HashMap::new(), false);
        assert!(!view.can_run);
        assert_eq!(
            view.run_disabled_reason,
            Some("Enter URL and resolve input")
        );
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
    pub triage_annotation: Option<TriageAnnotationView>,
    pub has_summary: bool,
    pub summary_title: Option<String>,
    pub filter_status: Option<JobFilterStatus>,
    pub has_analysis: bool,
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
