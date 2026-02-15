use crate::prompt_lab::{
    PromptLabInputSource, PromptLabRunId, PromptLabRunStatus, PromptLabStage, PromptLabState,
};
use crate::state::LinkDownloadState;
use crate::{JobId, JobResultKind, SessionState, Stage};
use harvester_engine::LinkKind;

pub const TOKEN_LIMIT: u64 = 200_000;
pub const INPUT_PANEL_FIXED_WIDTH: i32 = 160;
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

// Default left panel width when the input panel is shown (PANEL_INPUT + PANEL_JOBS = 160 + 440)
pub const DEFAULT_LEFT_PANEL_WIDTH: i32 = 600;
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
}

impl Default for PromptLabView {
    fn default() -> Self {
        Self {
            visible: false,
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
        }
    }
}

impl PromptLabView {
    pub(crate) fn from_state(state: &PromptLabState, triage_articles_available: bool) -> Self {
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
                    metadata
                        .map(|m| format!("{:?}", m.cache_status).to_lowercase()),
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
        let source_reason = match selected_input_source {
            PromptLabInputSource::FromTriageArticles => {
                if triage_articles_available {
                    None
                } else {
                    Some("No triage articles available")
                }
            }
            PromptLabInputSource::TypeUrl => {
                if state.resolved_url_snapshot().is_some() {
                    None
                } else {
                    Some("Resolve URL input")
                }
            }
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
        Self {
            visible: state.is_visible(),
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
        }
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
            None,
            None,
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
        let view = PromptLabView::from_state(&state, true);
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
        let view = PromptLabView::from_state(&state, false);
        assert!(view.can_run);
        assert_eq!(view.run_disabled_reason, None);
    }

    #[test]
    fn can_run_false_for_typeurl_when_snapshot_absent() {
        let mut state = PromptLabState::default();
        state.select_input_source(PromptLabInputSource::TypeUrl);
        let view = PromptLabView::from_state(&state, false);
        assert!(!view.can_run);
        assert_eq!(view.run_disabled_reason, Some("Resolve URL input"));
    }

    #[test]
    fn can_rerun_false_when_in_flight() {
        let mut state = PromptLabState::default();
        add_pending_run(&mut state);
        let view = PromptLabView::from_state(&state, true);
        assert!(!view.can_rerun);
    }

    #[test]
    fn can_rerun_true_when_latest_run_is_completed() {
        let mut state = PromptLabState::default();
        add_completed_run(&mut state);
        let view = PromptLabView::from_state(&state, true);
        assert!(view.can_rerun);
    }

    #[test]
    fn metadata_line_present_for_failed_run_with_metadata() {
        let mut state = PromptLabState::default();
        let _run_id = add_failed_run(&mut state, "error".to_string());
        let view = PromptLabView::from_state(&state, true);
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
        let view = PromptLabView::from_state(&state, true);
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
