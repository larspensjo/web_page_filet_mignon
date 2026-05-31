use super::{AppState, PromptLabPendingRunRegistration};
use crate::prompt_lab::{PromptLabRunId, PromptLabStage, PromptLabState};
use crate::PromptLabTemplateSnapshot;
use harvester_engine::llm::prompt::{PromptId, PromptVersion};
use harvester_engine::llm::run_metadata::LlmRunMetadata;
use std::collections::HashMap;

impl AppState {
    /// Get the context variables for a specific prompt, if loaded.
    /// Returns an empty slice if no context has been loaded for this prompt.
    pub fn context_for(&self, prompt_id: PromptId) -> &[(String, String)] {
        self.prompt_contexts
            .get(&prompt_id)
            .map(|v| v.as_slice())
            .unwrap_or(&[])
    }

    pub(crate) fn set_prompt_contexts(
        &mut self,
        contexts: HashMap<PromptId, Vec<(String, String)>>,
    ) {
        self.prompt_contexts = contexts;
        self.prompt_contexts_load_failed = false;
    }

    pub(crate) fn mark_prompt_contexts_load_failed(&mut self) {
        self.prompt_contexts_load_failed = true;
    }

    pub(crate) fn prompt_contexts_load_failed(&self) -> bool {
        self.prompt_contexts_load_failed
    }

    /// Get the active prompt version for a specific prompt.
    pub fn active_version_for(&self, prompt_id: PromptId) -> Option<PromptVersion> {
        self.active_prompt_versions.get(&prompt_id).copied()
    }

    /// Get the effective model for a specific prompt.
    pub fn effective_model_for(&self, prompt_id: PromptId) -> Option<&str> {
        self.effective_models.get(&prompt_id).map(|s| s.as_str())
    }

    pub(crate) fn set_llm_metadata(
        &mut self,
        active_versions: HashMap<PromptId, PromptVersion>,
        effective_models: HashMap<PromptId, String>,
        templates: HashMap<PromptId, PromptLabTemplateSnapshot>,
    ) {
        self.active_prompt_versions = active_versions;
        self.effective_models = effective_models;
        self.prompt_lab_templates = templates;
    }

    pub fn prompt_lab_template_snapshot(
        &self,
        prompt_id: PromptId,
    ) -> Option<&PromptLabTemplateSnapshot> {
        self.prompt_lab_templates.get(&prompt_id)
    }

    pub(crate) fn prompt_lab(&self) -> &PromptLabState {
        &self.prompt_lab
    }

    // Used in tests; will be used by the reducer when UI override messages are added.
    #[allow(dead_code)]
    pub(crate) fn prompt_lab_mut(&mut self) -> &mut PromptLabState {
        &mut self.prompt_lab
    }

    pub(crate) fn open_prompt_lab(&mut self) {
        self.select_left_tab(crate::tabs::LeftTab::PromptLab);
        self.prompt_lab.open();
    }

    /// Close Prompt Lab internal state (panel state, etc.) without changing `left_tab`.
    pub(crate) fn close_prompt_lab_internals(&mut self) {
        self.prompt_lab.close();
    }

    pub(crate) fn select_prompt_lab_stage(&mut self, stage: PromptLabStage) {
        self.prompt_lab.select_stage(stage);
        self.dirty = true;
    }

    pub(crate) fn set_prompt_lab_input(&mut self, text: String) {
        self.prompt_lab.set_input(text);
    }

    pub(crate) fn allocate_next_prompt_lab_run_id(&mut self) -> PromptLabRunId {
        let id = PromptLabRunId(self.next_prompt_lab_run_id);
        self.next_prompt_lab_run_id = self.next_prompt_lab_run_id.saturating_add(1);
        id
    }

    pub(crate) fn allocate_next_prompt_lab_resolve_id(&mut self) -> u64 {
        let id = self.prompt_lab_next_resolve_id;
        self.prompt_lab_next_resolve_id = self.prompt_lab_next_resolve_id.saturating_add(1);
        id
    }

    pub(crate) fn add_prompt_lab_pending_run(
        &mut self,
        registration: PromptLabPendingRunRegistration,
    ) {
        self.prompt_lab.add_pending_run(
            registration.run_id,
            registration.stage,
            registration.prompt_id,
            registration.input_snapshot,
            registration.request_id,
            registration.overrides,
        );
        if let Some(record) = self.prompt_lab.run_by_id_mut(registration.run_id) {
            record.compare_batch_id = registration.compare_batch_id;
            record.compare_candidate_id = registration.compare_candidate_id;
        }
    }

    pub(crate) fn complete_prompt_lab_run(
        &mut self,
        run_id: PromptLabRunId,
        output_json: String,
        metadata: LlmRunMetadata,
    ) {
        self.prompt_lab.complete_run(run_id, output_json, metadata);
    }

    pub(crate) fn fail_prompt_lab_run(
        &mut self,
        run_id: PromptLabRunId,
        reason: String,
        metadata: Option<LlmRunMetadata>,
    ) {
        self.prompt_lab.fail_run(run_id, reason, metadata);
    }

    pub(crate) fn consume_prompt_lab_ownership(&mut self, request_id: u64) {
        self.prompt_lab.consume_ownership(request_id);
    }

    pub(crate) fn clear_prompt_lab_history(&mut self) {
        self.prompt_lab.clear_history();
        self.dirty = true;
    }
}
