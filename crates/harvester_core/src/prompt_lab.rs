//! Prompt Lab domain: isolated state for running arbitrary LLM prompts.
//!
//! This module defines the Prompt Lab feature state, types, and invariants.
//! It is intentionally self-contained — it does not reference `BriefingSession`
//! or `TriageSession` directly.

use std::collections::HashMap;

use harvester_engine::llm::prompt::{PromptId, PromptVersion};
use harvester_engine::llm::run_metadata::LlmRunMetadata;
use harvester_engine::llm::types::ModelId;

use crate::context_draft::{parse_draft_text, serialize_pairs, ContextValidationError};

// ---------------------------------------------------------------------------
// Stage
// ---------------------------------------------------------------------------

/// Which workflow stage's prompt the lab targets.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum PromptLabStage {
    #[default]
    Triage,
    Summary,
    Briefing,
}

pub(crate) fn prompt_id_for_stage(stage: PromptLabStage) -> PromptId {
    match stage {
        PromptLabStage::Triage => PromptId::ArticleTriage,
        PromptLabStage::Summary => PromptId::ArticleSummary,
        PromptLabStage::Briefing => PromptId::AggregateBriefing,
    }
}

// ---------------------------------------------------------------------------
// IDs
// ---------------------------------------------------------------------------

/// Identifies a single Prompt Lab run (user-visible; distinct from `request_id`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct PromptLabRunId(pub u64);

// ---------------------------------------------------------------------------
// Run status
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PromptLabRunStatus {
    Pending {
        request_id: u64,
    },
    Completed {
        output_json: String,
        metadata: LlmRunMetadata,
    },
    Failed {
        reason: String,
        metadata: Option<LlmRunMetadata>,
    },
}

// ---------------------------------------------------------------------------
// Input source
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum PromptLabInputSource {
    #[default]
    FromTriageArticles,
    TypeUrl,
}

// ---------------------------------------------------------------------------
// Run record
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PromptLabRunRecord {
    pub run_id: PromptLabRunId,
    pub stage: PromptLabStage,
    pub prompt_id: PromptId,
    /// Snapshot of `input_content` at dispatch time.
    pub input_snapshot: String,
    pub status: PromptLabRunStatus,
    /// Prompt version override recorded at dispatch time (`None` = active version).
    pub prompt_version_used: Option<PromptVersion>,
    /// Model override recorded at dispatch time (`None` = stage/default model).
    pub model_override: Option<ModelId>,
}

// ---------------------------------------------------------------------------
// State
// ---------------------------------------------------------------------------

/// Draft state for the Prompt Lab context editor.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PromptLabContextDraft {
    base_snapshot: Vec<(String, String)>,
    draft_text: String,
    parsed_pairs: Option<Vec<(String, String)>>,
    validation_errors: Vec<ContextValidationError>,
    dirty: bool,
    applied: bool,
    loaded_snapshot: Option<Vec<(String, String)>>,
    status_message: Option<String>,
}

#[allow(dead_code)]
impl PromptLabContextDraft {
    fn new(base_snapshot: &[(String, String)]) -> Self {
        let stored_snapshot = base_snapshot.to_vec();
        let canonical = serialize_pairs(&stored_snapshot);
        Self {
            base_snapshot: stored_snapshot.clone(),
            draft_text: canonical,
            parsed_pairs: Some(stored_snapshot.clone()),
            validation_errors: Vec::new(),
            dirty: false,
            applied: false,
            loaded_snapshot: Some(stored_snapshot),
            status_message: None,
        }
    }

    fn update_text(&mut self, text: String) {
        self.draft_text = text.clone();
        self.applied = false;
        self.status_message = None;
        match parse_draft_text(&text) {
            Ok(parsed) => {
                self.parsed_pairs = Some(parsed.clone());
                self.validation_errors.clear();
            }
            Err(errors) => {
                self.parsed_pairs = None;
                self.validation_errors = errors;
            }
        }
        let canonical = serialize_pairs(&self.base_snapshot);
        self.dirty = canonical != text;
    }

    fn apply(&mut self) -> bool {
        if !self.dirty {
            return false;
        }
        let parsed = match self.parsed_pairs.clone() {
            Some(value) => value,
            None => return false,
        };
        self.base_snapshot = parsed.clone();
        self.draft_text = serialize_pairs(&parsed);
        self.parsed_pairs = Some(parsed.clone());
        self.validation_errors.clear();
        self.dirty = false;
        self.applied = true;
        self.status_message = None;
        true
    }

    fn revert(&mut self) {
        self.draft_text = serialize_pairs(&self.base_snapshot);
        self.parsed_pairs = Some(self.base_snapshot.clone());
        self.validation_errors.clear();
        self.dirty = false;
        self.status_message = None;
    }

    fn mark_saved(&mut self, message: Option<String>) {
        self.loaded_snapshot = Some(self.base_snapshot.clone());
        self.status_message = message;
    }

    pub(crate) fn applied_context(&self) -> Option<&[(String, String)]> {
        if self.applied {
            Some(self.base_snapshot.as_slice())
        } else {
            None
        }
    }

    pub(crate) fn differs_from_loaded(&self) -> bool {
        match &self.loaded_snapshot {
            Some(snapshot) => snapshot != &self.base_snapshot,
            None => true,
        }
    }

    pub(crate) fn dirty(&self) -> bool {
        self.dirty
    }

    pub(crate) fn applied(&self) -> bool {
        self.applied
    }

    pub(crate) fn text(&self) -> &str {
        &self.draft_text
    }

    pub(crate) fn validation_errors(&self) -> &[ContextValidationError] {
        &self.validation_errors
    }

    pub(crate) fn status_message(&self) -> Option<&str> {
        self.status_message.as_deref()
    }

    pub(crate) fn is_valid(&self) -> bool {
        self.parsed_pairs.is_some()
    }

    fn set_status_message(&mut self, message: Option<String>) {
        self.status_message = message;
    }
}

/// All mutable Prompt Lab state. Lives as a field on `AppState`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PromptLabState {
    pub(crate) visible: bool,
    pub(crate) selected_stage: PromptLabStage,
    /// Current text in the input buffer.
    pub(crate) input: String,
    pub(crate) selected_input_source: PromptLabInputSource,
    pub(crate) url_input: String,
    pub(crate) resolved_url_snapshot: Option<String>,
    pub(crate) pending_resolve_id: Option<u64>,
    pub(crate) last_resolve_failed: bool,
    /// Insertion-ordered run history. Uses Vec for simplicity (no IndexMap dependency).
    pub(crate) runs: Vec<(PromptLabRunId, PromptLabRunRecord)>,
    /// Maps LLM `request_id` → `PromptLabRunId` for completion routing.
    pub(crate) ownership: std::collections::HashMap<u64, PromptLabRunId>,
    pub(crate) latest_run_id: Option<PromptLabRunId>,
    /// Per-run prompt version override (`None` = use active version).
    pub(crate) selected_prompt_version: Option<PromptVersion>,
    /// Per-run model override (`None` = use stage/default model).
    pub(crate) selected_model_override: Option<ModelId>,
    pub(crate) context_overlays: HashMap<PromptId, PromptLabContextDraft>,
}

impl Default for PromptLabState {
    fn default() -> Self {
        Self {
            visible: false,
            selected_stage: PromptLabStage::Triage,
            input: String::new(),
            selected_input_source: PromptLabInputSource::default(),
            url_input: String::new(),
            resolved_url_snapshot: None,
            pending_resolve_id: None,
            last_resolve_failed: false,
            runs: Vec::new(),
            ownership: std::collections::HashMap::new(),
            latest_run_id: None,
            selected_prompt_version: None,
            selected_model_override: None,
            context_overlays: HashMap::new(),
        }
    }
}

impl PromptLabState {
    // ------------------------------------------------------------------
    // Visibility
    // ------------------------------------------------------------------

    pub fn open(&mut self) {
        self.visible = true;
    }

    pub fn close(&mut self) {
        self.visible = false;
    }

    pub fn is_visible(&self) -> bool {
        self.visible
    }

    // ------------------------------------------------------------------
    // Stage
    // ------------------------------------------------------------------

    pub fn select_stage(&mut self, stage: PromptLabStage) {
        self.selected_stage = stage;
    }

    pub fn selected_stage(&self) -> PromptLabStage {
        self.selected_stage
    }

    // ------------------------------------------------------------------
    // Input
    // ------------------------------------------------------------------

    pub fn set_input(&mut self, text: String) {
        self.input = text;
    }

    pub fn input(&self) -> &str {
        &self.input
    }

    pub fn select_input_source(&mut self, source: PromptLabInputSource) {
        self.selected_input_source = source;
        self.resolved_url_snapshot = None;
        self.pending_resolve_id = None;
        self.last_resolve_failed = false;
    }

    pub fn selected_input_source(&self) -> PromptLabInputSource {
        self.selected_input_source
    }

    pub fn url_input(&self) -> &str {
        &self.url_input
    }

    pub fn resolved_url_snapshot(&self) -> Option<&str> {
        self.resolved_url_snapshot.as_deref()
    }

    pub fn pending_resolve_id(&self) -> Option<u64> {
        self.pending_resolve_id
    }

    #[allow(dead_code)]
    pub fn last_resolve_failed(&self) -> bool {
        self.last_resolve_failed
    }

    pub fn set_url_input(&mut self, url: String) {
        self.url_input = url;
        self.resolved_url_snapshot = None;
        self.pending_resolve_id = None;
        self.last_resolve_failed = false;
    }

    pub fn begin_url_resolution(&mut self, resolve_id: u64) {
        self.pending_resolve_id = Some(resolve_id);
        self.last_resolve_failed = false;
    }

    pub fn finish_url_resolution(
        &mut self,
        resolve_id: u64,
        result: Result<String, String>,
    ) -> bool {
        if self.pending_resolve_id != Some(resolve_id) {
            return false;
        }
        self.pending_resolve_id = None;
        match result {
            Ok(snapshot) => {
                self.resolved_url_snapshot = Some(snapshot);
                self.last_resolve_failed = false;
            }
            Err(_) => {
                self.resolved_url_snapshot = None;
                self.last_resolve_failed = true;
            }
        }
        true
    }

    // ------------------------------------------------------------------
    // Per-run overrides
    // ------------------------------------------------------------------

    // These setters will be used by the UI (Step 4). Allow dead_code until then.
    #[allow(dead_code)]
    pub fn set_prompt_version_override(&mut self, version: Option<PromptVersion>) {
        self.selected_prompt_version = version;
    }

    #[allow(dead_code)]
    pub fn set_model_override(&mut self, model: Option<ModelId>) {
        self.selected_model_override = model;
    }

    #[allow(dead_code)]
    pub fn clear_overrides(&mut self) {
        self.selected_prompt_version = None;
        self.selected_model_override = None;
    }

    pub fn selected_prompt_version(&self) -> Option<PromptVersion> {
        self.selected_prompt_version
    }

    pub fn selected_model_override(&self) -> Option<&ModelId> {
        self.selected_model_override.as_ref()
    }

    // ------------------------------------------------------------------
    // Runs
    // ------------------------------------------------------------------

    /// Returns `true` if there is currently an in-flight (Pending) run.
    pub fn has_in_flight_run(&self) -> bool {
        !self.ownership.is_empty()
    }

    /// Register a new pending run record.
    #[allow(clippy::too_many_arguments)]
    pub fn add_pending_run(
        &mut self,
        run_id: PromptLabRunId,
        stage: PromptLabStage,
        prompt_id: PromptId,
        input_snapshot: String,
        request_id: u64,
        prompt_version_used: Option<PromptVersion>,
        model_override: Option<ModelId>,
    ) {
        let record = PromptLabRunRecord {
            run_id,
            stage,
            prompt_id,
            input_snapshot,
            status: PromptLabRunStatus::Pending { request_id },
            prompt_version_used,
            model_override,
        };
        self.runs.push((run_id, record));
        self.ownership.insert(request_id, run_id);
        self.latest_run_id = Some(run_id);
    }

    /// Look up which run owns `request_id`, if any.
    pub fn ownership_for(&self, request_id: u64) -> Option<PromptLabRunId> {
        self.ownership.get(&request_id).copied()
    }

    /// Transition the run to `Completed`. No-ops if the run is not `Pending`.
    pub fn complete_run(
        &mut self,
        run_id: PromptLabRunId,
        output_json: String,
        metadata: LlmRunMetadata,
    ) {
        if let Some((_, record)) = self.runs.iter_mut().find(|(id, _)| *id == run_id) {
            if matches!(record.status, PromptLabRunStatus::Pending { .. }) {
                record.status = PromptLabRunStatus::Completed {
                    output_json,
                    metadata,
                };
            }
        }
    }

    /// Transition the run to `Failed`. No-ops if the run is not `Pending`.
    pub fn fail_run(
        &mut self,
        run_id: PromptLabRunId,
        reason: String,
        metadata: Option<LlmRunMetadata>,
    ) {
        if let Some((_, record)) = self.runs.iter_mut().find(|(id, _)| *id == run_id) {
            if matches!(record.status, PromptLabRunStatus::Pending { .. }) {
                record.status = PromptLabRunStatus::Failed { reason, metadata };
            }
        }
    }

    /// Remove the ownership entry for `request_id` after completion/failure.
    pub fn consume_ownership(&mut self, request_id: u64) {
        self.ownership.remove(&request_id);
    }

    /// Remove all `Completed` and `Failed` runs. Preserves `Pending` runs and
    /// their ownership entries. Resets `latest_run_id` to the most recent
    /// remaining run or `None`.
    pub fn clear_history(&mut self) {
        self.runs
            .retain(|(_, r)| matches!(r.status, PromptLabRunStatus::Pending { .. }));
        self.latest_run_id = self.runs.last().map(|(id, _)| *id);
    }

    /// Number of recorded runs.
    pub fn run_count(&self) -> usize {
        self.runs.len()
    }

    /// Latest run record (if any).
    pub fn latest_run(&self) -> Option<&PromptLabRunRecord> {
        self.latest_run_id
            .and_then(|id| self.runs.iter().find(|(rid, _)| *rid == id).map(|(_, r)| r))
    }

    /// Initialize the context draft for `prompt_id` if it has not been created yet.
    pub(crate) fn initialize_context_draft(
        &mut self,
        prompt_id: PromptId,
        base_snapshot: &[(String, String)],
    ) {
        self.context_overlays
            .entry(prompt_id)
            .or_insert_with(|| PromptLabContextDraft::new(base_snapshot));
    }

    pub(crate) fn update_context_draft_text(&mut self, prompt_id: PromptId, text: String) -> bool {
        if let Some(draft) = self.context_overlays.get_mut(&prompt_id) {
            draft.update_text(text);
            true
        } else {
            false
        }
    }

    pub(crate) fn apply_context_draft(&mut self, prompt_id: PromptId) -> bool {
        if let Some(draft) = self.context_overlays.get_mut(&prompt_id) {
            draft.apply()
        } else {
            false
        }
    }

    pub(crate) fn revert_context_draft(&mut self, prompt_id: PromptId) -> bool {
        if let Some(draft) = self.context_overlays.get_mut(&prompt_id) {
            draft.revert();
            true
        } else {
            false
        }
    }

    pub(crate) fn drop_context_draft(&mut self, prompt_id: PromptId) {
        self.context_overlays.remove(&prompt_id);
    }

    pub(crate) fn clear_context_overlays(&mut self) {
        self.context_overlays.clear();
    }

    pub(crate) fn applied_context_pairs(&self, prompt_id: PromptId) -> Option<&[(String, String)]> {
        self.context_overlays
            .get(&prompt_id)
            .and_then(|draft| draft.applied_context())
    }

    pub(crate) fn can_save_context(&self, prompt_id: PromptId) -> bool {
        self.context_overlays
            .get(&prompt_id)
            .map(|draft| draft.applied() && draft.differs_from_loaded())
            .unwrap_or(false)
    }

    pub(crate) fn mark_context_saved(&mut self, prompt_id: PromptId, message: Option<String>) {
        if let Some(draft) = self.context_overlays.get_mut(&prompt_id) {
            draft.mark_saved(message);
        }
    }

    pub(crate) fn set_context_status_message(
        &mut self,
        prompt_id: PromptId,
        message: Option<String>,
    ) {
        if let Some(draft) = self.context_overlays.get_mut(&prompt_id) {
            draft.set_status_message(message);
        }
    }

    pub(crate) fn context_draft(&self, prompt_id: PromptId) -> Option<&PromptLabContextDraft> {
        self.context_overlays.get(&prompt_id)
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use harvester_engine::llm::prompt::PromptId;
    use harvester_engine::llm::run_metadata::LlmRunMetadata;

    fn make_prompt_id() -> PromptId {
        PromptId::ArticleTriage
    }

    #[test]
    fn default_is_closed_triage_empty() {
        let s = PromptLabState::default();
        assert!(!s.is_visible());
        assert_eq!(s.selected_stage(), PromptLabStage::Triage);
        assert_eq!(s.run_count(), 0);
        assert!(!s.has_in_flight_run());
    }

    #[test]
    fn open_close() {
        let mut s = PromptLabState::default();
        s.open();
        assert!(s.is_visible());
        s.close();
        assert!(!s.is_visible());
    }

    #[test]
    fn stage_selection() {
        let mut s = PromptLabState::default();
        s.select_stage(PromptLabStage::Summary);
        assert_eq!(s.selected_stage(), PromptLabStage::Summary);
    }

    #[test]
    fn run_record_pending_on_creation() {
        let mut s = PromptLabState::default();
        let run_id = PromptLabRunId(1);
        s.add_pending_run(
            run_id,
            PromptLabStage::Triage,
            make_prompt_id(),
            "hello".to_string(),
            42,
            None,
            None,
        );
        let r = s.latest_run().unwrap();
        assert!(matches!(
            r.status,
            PromptLabRunStatus::Pending { request_id: 42 }
        ));
        assert!(s.has_in_flight_run());
    }

    #[test]
    fn pending_to_completed() {
        let mut s = PromptLabState::default();
        let run_id = PromptLabRunId(1);
        s.add_pending_run(
            run_id,
            PromptLabStage::Triage,
            make_prompt_id(),
            "x".to_string(),
            10,
            None,
            None,
        );
        s.complete_run(run_id, "{}".to_string(), LlmRunMetadata::stub());
        let r = s.latest_run().unwrap();
        assert!(matches!(r.status, PromptLabRunStatus::Completed { .. }));
    }

    #[test]
    fn pending_to_failed() {
        let mut s = PromptLabState::default();
        let run_id = PromptLabRunId(1);
        s.add_pending_run(
            run_id,
            PromptLabStage::Triage,
            make_prompt_id(),
            "x".to_string(),
            10,
            None,
            None,
        );
        s.fail_run(run_id, "something broke".to_string(), None);
        let r = s.latest_run().unwrap();
        assert!(matches!(r.status, PromptLabRunStatus::Failed { .. }));
    }

    #[test]
    fn completed_record_is_immutable() {
        let mut s = PromptLabState::default();
        let run_id = PromptLabRunId(1);
        s.add_pending_run(
            run_id,
            PromptLabStage::Triage,
            make_prompt_id(),
            "x".to_string(),
            10,
            None,
            None,
        );
        s.complete_run(run_id, "{}".to_string(), LlmRunMetadata::stub());
        // Calling complete_run again on a Completed record is a no-op.
        s.complete_run(run_id, "overwrite?".to_string(), LlmRunMetadata::stub());
        if let PromptLabRunStatus::Completed { output_json, .. } = &s.latest_run().unwrap().status {
            assert_eq!(output_json, "{}");
        } else {
            panic!("expected Completed");
        }
    }

    #[test]
    fn clear_history_removes_completed_and_failed_but_not_pending() {
        let mut s = PromptLabState::default();
        // Add a completed run
        let r1 = PromptLabRunId(1);
        s.add_pending_run(
            r1,
            PromptLabStage::Triage,
            make_prompt_id(),
            "a".to_string(),
            10,
            None,
            None,
        );
        s.complete_run(r1, "{}".to_string(), LlmRunMetadata::stub());
        s.consume_ownership(10);
        // Add a failed run
        let r2 = PromptLabRunId(2);
        s.add_pending_run(
            r2,
            PromptLabStage::Triage,
            make_prompt_id(),
            "b".to_string(),
            11,
            None,
            None,
        );
        s.fail_run(r2, "err".to_string(), None);
        s.consume_ownership(11);
        // Add a still-pending run
        let r3 = PromptLabRunId(3);
        s.add_pending_run(
            r3,
            PromptLabStage::Triage,
            make_prompt_id(),
            "c".to_string(),
            12,
            None,
            None,
        );

        s.clear_history();

        assert_eq!(s.run_count(), 1);
        assert!(matches!(
            s.latest_run().unwrap().status,
            PromptLabRunStatus::Pending { .. }
        ));
        // Ownership entry for r3 still intact
        assert!(s.ownership_for(12).is_some());
    }

    #[test]
    fn ownership_for_unknown_returns_none() {
        let s = PromptLabState::default();
        assert!(s.ownership_for(999).is_none());
    }

    #[test]
    fn consume_ownership_removes_entry() {
        let mut s = PromptLabState::default();
        let run_id = PromptLabRunId(1);
        s.add_pending_run(
            run_id,
            PromptLabStage::Triage,
            make_prompt_id(),
            "x".to_string(),
            10,
            None,
            None,
        );
        s.consume_ownership(10);
        assert!(s.ownership_for(10).is_none());
        // has_in_flight is now false
        assert!(!s.has_in_flight_run());
    }

    #[test]
    fn url_input_change_invalidates_snapshot() {
        let mut s = PromptLabState {
            resolved_url_snapshot: Some("cached".to_string()),
            pending_resolve_id: Some(5),
            last_resolve_failed: true,
            ..Default::default()
        };
        s.set_url_input("https://example.com".to_string());
        assert_eq!(s.url_input(), "https://example.com");
        assert!(s.resolved_url_snapshot().is_none());
        assert!(s.pending_resolve_id().is_none());
        assert!(!s.last_resolve_failed());
    }

    #[test]
    fn stale_resolve_ignored() {
        let mut s = PromptLabState::default();
        s.begin_url_resolution(7);
        assert!(!s.finish_url_resolution(8, Ok("ignored".to_string())));
        // Pending ID should remain intact
        assert_eq!(s.pending_resolve_id(), Some(7));
        assert!(s.resolved_url_snapshot().is_none());
    }

    #[test]
    fn finish_resolve_with_matching_id_stores_snapshot() {
        let mut s = PromptLabState::default();
        s.begin_url_resolution(42);
        assert!(s.finish_url_resolution(42, Ok("snapshot".to_string())));
        assert_eq!(s.resolved_url_snapshot(), Some("snapshot"));
        assert!(s.pending_resolve_id().is_none());
        assert!(!s.last_resolve_failed());

        s.begin_url_resolution(99);
        assert!(s.finish_url_resolution(99, Err("boom".to_string())));
        assert!(s.resolved_url_snapshot().is_none());
        assert!(s.last_resolve_failed());
    }

    #[test]
    fn prompt_lab_failed_run_preserves_metadata() {
        let mut s = PromptLabState::default();
        let run_id = PromptLabRunId(1);
        let metadata = LlmRunMetadata::stub();
        s.add_pending_run(
            run_id,
            PromptLabStage::Triage,
            make_prompt_id(),
            "x".to_string(),
            10,
            None,
            None,
        );
        s.fail_run(run_id, "oops".to_string(), Some(metadata.clone()));
        let record = s.latest_run().unwrap();
        if let PromptLabRunStatus::Failed {
            metadata: stored, ..
        } = &record.status
        {
            assert_eq!(stored.as_ref(), Some(&metadata));
        } else {
            panic!("expected Failed status");
        }
    }

    #[test]
    fn context_draft_apply_updates_overlay_and_save_state() {
        let mut state = PromptLabState::default();
        let prompt_id = PromptId::ArticleTriage;
        state.initialize_context_draft(prompt_id, &[]);
        assert!(state.update_context_draft_text(prompt_id, "foo=bar".to_string()));
        assert!(state.apply_context_draft(prompt_id));
        let applied = state.applied_context_pairs(prompt_id).unwrap();
        assert_eq!(applied, &[("foo".to_string(), "bar".to_string())]);
        assert!(state.can_save_context(prompt_id));
    }

    #[test]
    fn context_draft_apply_fails_for_invalid_draft() {
        let mut state = PromptLabState::default();
        let prompt_id = PromptId::ArticleSummary;
        state.initialize_context_draft(prompt_id, &[]);
        assert!(state.update_context_draft_text(prompt_id, "invalid line".to_string()));
        assert!(!state.apply_context_draft(prompt_id));
        assert!(state.applied_context_pairs(prompt_id).is_none());
        assert!(!state.can_save_context(prompt_id));
    }

    #[test]
    fn context_draft_revert_returns_to_base() {
        let mut state = PromptLabState::default();
        let prompt_id = PromptId::AggregateBriefing;
        state.initialize_context_draft(prompt_id, &[("initial".into(), "value".into())]);
        assert!(state.update_context_draft_text(prompt_id, "foo=bar".to_string()));
        assert!(state.apply_context_draft(prompt_id));
        assert!(state.can_save_context(prompt_id));
        assert!(state.update_context_draft_text(prompt_id, "foo=baz".to_string()));
        assert!(state.revert_context_draft(prompt_id));
        let draft = state.context_overlays.get(&prompt_id).unwrap();
        assert_eq!(draft.text(), "foo=bar\n");
        assert!(!draft.dirty());
    }

    #[test]
    fn context_draft_stage_switch_preserves_each_prompt() {
        let mut state = PromptLabState::default();
        let primary = PromptId::ArticleTriage;
        let secondary = PromptId::ArticleSummary;
        state.initialize_context_draft(primary, &[]);
        state.initialize_context_draft(secondary, &[("k".into(), "v".into())]);
        assert_eq!(state.context_overlays.len(), 2);
        let first_clone = state.context_overlays.get(&primary).cloned().unwrap();
        state.initialize_context_draft(primary, &[]);
        assert_eq!(state.context_overlays.get(&primary).unwrap(), &first_clone);
    }

    #[test]
    fn context_draft_lazy_init_handles_empty_base() {
        let mut state = PromptLabState::default();
        let prompt_id = PromptId::ArticleSummary;
        state.initialize_context_draft(prompt_id, &[]);
        let draft = state.context_overlays.get(&prompt_id).unwrap();
        assert_eq!(draft.text(), "");
        assert!(!draft.dirty());
        assert!(!draft.applied());
    }
}
