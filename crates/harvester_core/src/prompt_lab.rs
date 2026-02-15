//! Prompt Lab domain: isolated state for running arbitrary LLM prompts.
//!
//! This module defines the Prompt Lab feature state, types, and invariants.
//! It is intentionally self-contained — it does not reference `BriefingSession`
//! or `TriageSession` directly.

use harvester_engine::llm::prompt::{PromptId, PromptVersion};
use harvester_engine::llm::run_metadata::LlmRunMetadata;
use harvester_engine::llm::types::ModelId;

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
    },
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

/// All mutable Prompt Lab state. Lives as a field on `AppState`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PromptLabState {
    pub(crate) visible: bool,
    pub(crate) selected_stage: PromptLabStage,
    /// Current text in the input buffer.
    pub(crate) input: String,
    /// Insertion-ordered run history. Uses Vec for simplicity (no IndexMap dependency).
    pub(crate) runs: Vec<(PromptLabRunId, PromptLabRunRecord)>,
    /// Maps LLM `request_id` → `PromptLabRunId` for completion routing.
    pub(crate) ownership: std::collections::HashMap<u64, PromptLabRunId>,
    pub(crate) latest_run_id: Option<PromptLabRunId>,
    /// Per-run prompt version override (`None` = use active version).
    pub(crate) selected_prompt_version: Option<PromptVersion>,
    /// Per-run model override (`None` = use stage/default model).
    pub(crate) selected_model_override: Option<ModelId>,
}

impl Default for PromptLabState {
    fn default() -> Self {
        Self {
            visible: false,
            selected_stage: PromptLabStage::Triage,
            input: String::new(),
            runs: Vec::new(),
            ownership: std::collections::HashMap::new(),
            latest_run_id: None,
            selected_prompt_version: None,
            selected_model_override: None,
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

    // ------------------------------------------------------------------
    // Per-run overrides
    // ------------------------------------------------------------------

    pub fn set_prompt_version_override(&mut self, version: Option<PromptVersion>) {
        self.selected_prompt_version = version;
    }

    pub fn set_model_override(&mut self, model: Option<ModelId>) {
        self.selected_model_override = model;
    }

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
    pub fn fail_run(&mut self, run_id: PromptLabRunId, reason: String) {
        if let Some((_, record)) = self.runs.iter_mut().find(|(id, _)| *id == run_id) {
            if matches!(record.status, PromptLabRunStatus::Pending { .. }) {
                record.status = PromptLabRunStatus::Failed { reason };
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
        self.latest_run_id.and_then(|id| {
            self.runs
                .iter()
                .find(|(rid, _)| *rid == id)
                .map(|(_, r)| r)
        })
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use harvester_engine::llm::prompt::PromptId;

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
        s.add_pending_run(run_id, PromptLabStage::Triage, make_prompt_id(), "hello".to_string(), 42, None, None);
        let r = s.latest_run().unwrap();
        assert!(matches!(r.status, PromptLabRunStatus::Pending { request_id: 42 }));
        assert!(s.has_in_flight_run());
    }

    #[test]
    fn pending_to_completed() {
        let mut s = PromptLabState::default();
        let run_id = PromptLabRunId(1);
        s.add_pending_run(run_id, PromptLabStage::Triage, make_prompt_id(), "x".to_string(), 10, None, None);
        s.complete_run(run_id, "{}".to_string(), LlmRunMetadata::stub());
        let r = s.latest_run().unwrap();
        assert!(matches!(r.status, PromptLabRunStatus::Completed { .. }));
    }

    #[test]
    fn pending_to_failed() {
        let mut s = PromptLabState::default();
        let run_id = PromptLabRunId(1);
        s.add_pending_run(run_id, PromptLabStage::Triage, make_prompt_id(), "x".to_string(), 10, None, None);
        s.fail_run(run_id, "something broke".to_string());
        let r = s.latest_run().unwrap();
        assert!(matches!(r.status, PromptLabRunStatus::Failed { .. }));
    }

    #[test]
    fn completed_record_is_immutable() {
        let mut s = PromptLabState::default();
        let run_id = PromptLabRunId(1);
        s.add_pending_run(run_id, PromptLabStage::Triage, make_prompt_id(), "x".to_string(), 10, None, None);
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
        s.add_pending_run(r1, PromptLabStage::Triage, make_prompt_id(), "a".to_string(), 10, None, None);
        s.complete_run(r1, "{}".to_string(), LlmRunMetadata::stub());
        s.consume_ownership(10);
        // Add a failed run
        let r2 = PromptLabRunId(2);
        s.add_pending_run(r2, PromptLabStage::Triage, make_prompt_id(), "b".to_string(), 11, None, None);
        s.fail_run(r2, "err".to_string());
        s.consume_ownership(11);
        // Add a still-pending run
        let r3 = PromptLabRunId(3);
        s.add_pending_run(r3, PromptLabStage::Triage, make_prompt_id(), "c".to_string(), 12, None, None);

        s.clear_history();

        assert_eq!(s.run_count(), 1);
        assert!(matches!(s.latest_run().unwrap().status, PromptLabRunStatus::Pending { .. }));
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
        s.add_pending_run(run_id, PromptLabStage::Triage, make_prompt_id(), "x".to_string(), 10, None, None);
        s.consume_ownership(10);
        assert!(s.ownership_for(10).is_none());
        // has_in_flight is now false
        assert!(!s.has_in_flight_run());
    }
}
