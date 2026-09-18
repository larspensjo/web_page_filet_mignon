use super::AppState;
use harvester_engine::llm::prompt::{PromptId, PromptVersion};
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

    pub(crate) fn prompt_contexts_loaded(&self) -> bool {
        self.prompt_contexts
            .contains_key(&PromptId::BriefingExecutiveSummary)
            && self
                .prompt_contexts
                .contains_key(&PromptId::BriefingNextItem)
    }

    pub(crate) fn mark_prompt_template_files_loaded(&mut self) {
        self.prompt_template_files_loaded = true;
    }

    pub(crate) fn prompt_templates_loaded(&self) -> bool {
        self.prompt_template_files_loaded
    }

    /// Get the active prompt version for a specific prompt.
    pub fn active_version_for(&self, prompt_id: PromptId) -> Option<PromptVersion> {
        self.active_prompt_versions.get(&prompt_id).copied()
    }

    /// Get the effective model for a specific prompt.
    pub fn effective_model_for(&self, prompt_id: PromptId) -> Option<&str> {
        self.effective_models.get(&prompt_id).map(|s| s.as_str())
    }

    pub(crate) fn llm_metadata_loaded(&self) -> bool {
        self.active_prompt_versions
            .contains_key(&PromptId::BriefingExecutiveSummary)
            && self
                .active_prompt_versions
                .contains_key(&PromptId::BriefingNextItem)
            && self
                .effective_models
                .contains_key(&PromptId::BriefingExecutiveSummary)
            && self
                .effective_models
                .contains_key(&PromptId::BriefingNextItem)
    }

    pub(crate) fn set_llm_metadata(
        &mut self,
        active_versions: HashMap<PromptId, PromptVersion>,
        effective_models: HashMap<PromptId, String>,
    ) {
        self.active_prompt_versions = active_versions;
        self.effective_models = effective_models;
    }
}
