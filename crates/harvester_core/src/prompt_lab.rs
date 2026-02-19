//! Prompt Lab domain: isolated state for running arbitrary LLM prompts.
//!
//! This module defines the Prompt Lab feature state, types, and invariants.
//! It is intentionally self-contained — it does not reference `BriefingSession`
//! or `TriageSession` directly.

use std::collections::HashMap;
use std::path::PathBuf;

use harvester_engine::llm::prompt::{
    PromptId, PromptTemplateOwned, PromptVersion, TemplateSource, PROMPT_VERSION_DRAFT,
};
use harvester_engine::llm::run_metadata::LlmRunMetadata;
use harvester_engine::llm::types::ModelId;
use harvester_engine::llm::TemplateValidationError;

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

/// Identifies a compare batch.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct PromptLabCompareBatchId(pub u64);

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
// Model catalog
// ---------------------------------------------------------------------------

/// Source of the model catalog for Prompt Lab model selection.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ModelCatalogSource {
    #[default]
    NotLoaded,
    Remote,
    LocalFallback,
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
    pub compare_batch_id: Option<PromptLabCompareBatchId>,
    pub compare_candidate_id: Option<u64>,
    pub operator_rating: Option<u8>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct PromptLabRunOverrides {
    pub prompt_version_used: Option<PromptVersion>,
    pub model_override: Option<ModelId>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PromptLabCompareBatchStatus {
    Running { dispatched: u32, total: u32 },
    AllComplete,
    PartialFailure,
    Cancelled,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PromptLabCompareCandidate {
    pub candidate_id: u64,
    pub stage: PromptLabStage,
    pub prompt_id: PromptId,
    pub prompt_version: Option<PromptVersion>,
    pub model_override: Option<ModelId>,
    pub context_snapshot: Vec<(String, String)>,
    pub template_snapshot: Option<PromptTemplateOwned>,
    pub label: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PromptLabComparePolicy {
    pub require_parse_ok: bool,
    pub max_cost_microdollars: Option<u64>,
    pub max_wall_ms: Option<u64>,
    pub rating_beats_cost: bool,
}

impl Default for PromptLabComparePolicy {
    fn default() -> Self {
        Self {
            require_parse_ok: true,
            max_cost_microdollars: None,
            max_wall_ms: None,
            rating_beats_cost: false,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PromptLabCompareBatchRecord {
    pub batch_id: PromptLabCompareBatchId,
    pub created_utc: String,
    pub input_snapshot: String,
    pub candidates: Vec<PromptLabCompareCandidate>,
    pub candidate_run_ids: Vec<(u64, Option<PromptLabRunId>)>,
    pub status: PromptLabCompareBatchStatus,
    pub policy: PromptLabComparePolicy,
    pub selected_run_id: Option<PromptLabRunId>,
    pub auto_selected_run_id: Option<PromptLabRunId>,
    pub auto_select_warning: Option<String>,
    pub warning: Option<String>,
}

impl PromptLabCompareBatchRecord {
    pub fn effective_winner(&self) -> Option<PromptLabRunId> {
        self.selected_run_id.or(self.auto_selected_run_id)
    }

    pub fn pending_candidate_count(&self) -> usize {
        self.candidate_run_ids
            .iter()
            .filter(|(_, run_id)| run_id.is_none())
            .count()
    }

    pub fn next_undispatched_candidate(&self) -> Option<&PromptLabCompareCandidate> {
        let next = self
            .candidate_run_ids
            .iter()
            .find_map(|(candidate_id, run_id)| run_id.is_none().then_some(*candidate_id))?;
        self.candidates.iter().find(|c| c.candidate_id == next)
    }
}

/// Snapshot of an effective template for UI consumption.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PromptLabTemplateSnapshot {
    pub template: PromptTemplateOwned,
    pub source: TemplateSource,
}

/// Draft state for the Prompt Lab template editor.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PromptLabTemplateDraft {
    system_base: String,
    user_base: String,
    system_draft: String,
    user_draft: String,
    validation_errors: Vec<TemplateValidationError>,
    dirty: bool,
    applied: bool,
    saved_version: Option<PromptVersion>,
    saved_path: Option<PathBuf>,
    description: String,
    expected_format: String,
}

impl PromptLabTemplateDraft {
    fn new(system: &str, user: &str, description: &str, expected_format: &str) -> Self {
        Self {
            system_base: system.to_string(),
            user_base: user.to_string(),
            system_draft: system.to_string(),
            user_draft: user.to_string(),
            validation_errors: Vec::new(),
            dirty: false,
            applied: false,
            saved_version: None,
            saved_path: None,
            description: description.to_string(),
            expected_format: expected_format.to_string(),
        }
    }

    fn update_dirty(&mut self) {
        self.dirty = self.system_draft != self.system_base || self.user_draft != self.user_base;
    }

    fn update_system(&mut self, text: String) {
        self.system_draft = text;
        self.applied = false;
        self.validation_errors.clear();
        self.update_dirty();
    }

    fn update_user(&mut self, text: String) {
        self.user_draft = text;
        self.applied = false;
        self.validation_errors.clear();
        self.update_dirty();
    }

    fn apply(&mut self, errors: Vec<TemplateValidationError>) -> bool {
        self.validation_errors = errors;
        if !self.validation_errors.is_empty() {
            self.applied = false;
            return false;
        }
        self.system_base = self.system_draft.clone();
        self.user_base = self.user_draft.clone();
        self.dirty = false;
        self.applied = true;
        true
    }

    fn revert(&mut self) {
        self.system_draft = self.system_base.clone();
        self.user_draft = self.user_base.clone();
        self.validation_errors.clear();
        self.dirty = false;
        self.applied = false;
    }

    fn mark_saved(&mut self, version: PromptVersion, path: PathBuf) {
        self.saved_version = Some(version);
        self.saved_path = Some(path);
    }

    pub(crate) fn description(&self) -> &str {
        &self.description
    }

    pub(crate) fn expected_format(&self) -> &str {
        &self.expected_format
    }

    pub(crate) fn system_draft(&self) -> &str {
        &self.system_draft
    }

    pub(crate) fn user_draft(&self) -> &str {
        &self.user_draft
    }

    pub(crate) fn validation_errors(&self) -> &[TemplateValidationError] {
        &self.validation_errors
    }

    pub(crate) fn is_dirty(&self) -> bool {
        self.dirty
    }

    pub(crate) fn is_applied(&self) -> bool {
        self.applied
    }

    pub(crate) fn saved_version(&self) -> Option<PromptVersion> {
        self.saved_version
    }

    pub(crate) fn saved_path(&self) -> Option<&PathBuf> {
        self.saved_path.as_ref()
    }
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
    /// Catalog of available models for override selection.
    pub(crate) model_catalog: Vec<ModelId>,
    /// Source of the model catalog (remote discovery or local fallback).
    pub(crate) catalog_source: ModelCatalogSource,
    pub(crate) context_overlays: HashMap<PromptId, PromptLabContextDraft>,
    pub(crate) template_drafts: HashMap<PromptId, PromptLabTemplateDraft>,
    pub(crate) template_editor_open: bool,
    pub(crate) advanced_mode: bool,
    pub(crate) compare_section_open: bool,
    pub(crate) context_section_open: bool,
    pub(crate) template_section_open: bool,
    pub(crate) run_details_section_open: bool,
    pub(crate) next_compare_batch_id: u64,
    pub(crate) batches: Vec<PromptLabCompareBatchRecord>,
    pub(crate) active_batch_idx: Option<usize>,
    pub(crate) draft_candidates: Vec<PromptLabCompareCandidate>,
    pub(crate) next_draft_candidate_id: u64,
    pub(crate) compare_policy: PromptLabComparePolicy,
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
            model_catalog: Vec::new(),
            catalog_source: ModelCatalogSource::default(),
            context_overlays: HashMap::new(),
            template_drafts: HashMap::new(),
            template_editor_open: false,
            advanced_mode: false,
            compare_section_open: false,
            context_section_open: false,
            template_section_open: false,
            run_details_section_open: false,
            next_compare_batch_id: 1,
            batches: Vec::new(),
            active_batch_idx: None,
            draft_candidates: Vec::new(),
            next_draft_candidate_id: 1,
            compare_policy: PromptLabComparePolicy::default(),
        }
    }
}

impl PromptLabState {
    fn normalize_visibility_invariants(&mut self) {
        if !self.visible || !self.advanced_mode || !self.template_section_open {
            self.template_editor_open = false;
        }
    }

    // ------------------------------------------------------------------
    // Visibility
    // ------------------------------------------------------------------

    pub fn open(&mut self) {
        self.visible = true;
    }

    pub fn close(&mut self) {
        self.visible = false;
        self.normalize_visibility_invariants();
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

    /// Sets the model override after validating it against the catalog.
    /// Logs a warning and rejects the override if the model is not in the catalog.
    pub fn set_model_override_checked(&mut self, model: Option<ModelId>) {
        if let Some(ref m) = model {
            if !self.model_catalog.iter().any(|cat_m| cat_m == m) {
                engine_logging::engine_warn!(
                    "[prompt-lab-model] attempted to set model override to {:?} which is not in catalog",
                    m
                );
                return;
            }
        }
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

    pub fn model_catalog(&self) -> &[ModelId] {
        &self.model_catalog
    }

    pub fn catalog_source(&self) -> ModelCatalogSource {
        self.catalog_source
    }

    /// Sets the model catalog and source.
    /// If the current selection is not in the new catalog, it is cleared with a warning.
    pub fn set_model_catalog(&mut self, models: Vec<ModelId>, source: ModelCatalogSource) {
        let sample = models
            .iter()
            .take(5)
            .map(|m| m.model_name().to_string())
            .collect::<Vec<_>>()
            .join(", ");
        engine_logging::engine_info!(
            "[prompt-lab-model] state set_model_catalog source={:?} count={} sample=[{}]",
            source,
            models.len(),
            sample
        );
        self.model_catalog = models;
        self.catalog_source = source;

        // Clear stale selection
        if let Some(ref current) = self.selected_model_override {
            if !self.model_catalog.iter().any(|m| m == current) {
                engine_logging::engine_warn!(
                    "[prompt-lab-model] cleared stale model override {:?} after catalog refresh",
                    current
                );
                self.selected_model_override = None;
            }
        }
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
        overrides: PromptLabRunOverrides,
    ) {
        let record = PromptLabRunRecord {
            run_id,
            stage,
            prompt_id,
            input_snapshot,
            status: PromptLabRunStatus::Pending { request_id },
            prompt_version_used: overrides.prompt_version_used,
            model_override: overrides.model_override,
            compare_batch_id: None,
            compare_candidate_id: None,
            operator_rating: None,
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

    pub fn run_by_id(&self, run_id: PromptLabRunId) -> Option<&PromptLabRunRecord> {
        self.runs
            .iter()
            .find_map(|(id, run)| (*id == run_id).then_some(run))
    }

    pub fn run_by_id_mut(&mut self, run_id: PromptLabRunId) -> Option<&mut PromptLabRunRecord> {
        self.runs
            .iter_mut()
            .find_map(|(id, run)| (*id == run_id).then_some(run))
    }

    pub fn has_active_batch(&self) -> bool {
        self.active_batch_idx
            .and_then(|idx| self.batches.get(idx))
            .map(|batch| matches!(batch.status, PromptLabCompareBatchStatus::Running { .. }))
            .unwrap_or(false)
    }

    pub fn add_draft_candidate_from_current(
        &mut self,
        label: Option<String>,
    ) -> Result<u64, String> {
        if self.has_active_batch() {
            return Err("batch already running".to_string());
        }
        let stage = self.selected_stage;
        let prompt_id = prompt_id_for_stage(stage);
        let candidate_id = self.next_draft_candidate_id;
        self.next_draft_candidate_id = self.next_draft_candidate_id.saturating_add(1);
        let context_snapshot = self
            .applied_context_pairs(prompt_id)
            .map_or_else(Vec::new, |pairs| pairs.to_vec());
        let candidate = PromptLabCompareCandidate {
            candidate_id,
            stage,
            prompt_id,
            prompt_version: self.selected_prompt_version,
            model_override: self.selected_model_override.clone(),
            context_snapshot,
            template_snapshot: self.applied_template_override(prompt_id),
            label: label.unwrap_or_else(|| format!("Candidate {candidate_id}")),
        };
        self.draft_candidates.push(candidate);
        Ok(candidate_id)
    }

    pub fn add_baseline_candidate(&mut self, label: Option<String>) -> Result<u64, String> {
        if self.has_active_batch() {
            return Err("batch already running".to_string());
        }
        let stage = self.selected_stage;
        let prompt_id = prompt_id_for_stage(stage);
        let candidate_id = self.next_draft_candidate_id;
        self.next_draft_candidate_id = self.next_draft_candidate_id.saturating_add(1);
        self.draft_candidates.push(PromptLabCompareCandidate {
            candidate_id,
            stage,
            prompt_id,
            prompt_version: None,
            model_override: None,
            context_snapshot: Vec::new(),
            template_snapshot: None,
            label: label.unwrap_or_else(|| format!("Baseline {candidate_id}")),
        });
        Ok(candidate_id)
    }

    pub fn freeze_batch(
        &mut self,
        input_snapshot: String,
    ) -> Result<PromptLabCompareBatchId, String> {
        if self.draft_candidates.len() < 2 {
            return Err("compare needs at least two candidates".to_string());
        }
        let batch_id = PromptLabCompareBatchId(self.next_compare_batch_id);
        self.next_compare_batch_id = self.next_compare_batch_id.saturating_add(1);
        let candidates = self.draft_candidates.clone();
        let total = candidates.len() as u32;
        let candidate_run_ids = candidates
            .iter()
            .map(|c| (c.candidate_id, None))
            .collect::<Vec<_>>();
        let record = PromptLabCompareBatchRecord {
            batch_id,
            created_utc: chrono::Utc::now().to_rfc3339(),
            input_snapshot,
            candidates,
            candidate_run_ids,
            status: PromptLabCompareBatchStatus::Running {
                dispatched: 0,
                total,
            },
            policy: self.compare_policy.clone(),
            selected_run_id: None,
            auto_selected_run_id: None,
            auto_select_warning: None,
            warning: None,
        };
        self.batches.push(record);
        self.active_batch_idx = Some(self.batches.len() - 1);
        self.draft_candidates.clear();
        Ok(batch_id)
    }

    pub fn batches(&self) -> &[PromptLabCompareBatchRecord] {
        &self.batches
    }

    pub fn draft_candidates(&self) -> &[PromptLabCompareCandidate] {
        &self.draft_candidates
    }

    pub fn clear_draft_candidates(&mut self) {
        self.draft_candidates.clear();
    }

    pub fn remove_draft_candidate(&mut self, candidate_id: u64) -> bool {
        let before = self.draft_candidates.len();
        self.draft_candidates
            .retain(|candidate| candidate.candidate_id != candidate_id);
        before != self.draft_candidates.len()
    }

    pub fn rename_draft_candidate(&mut self, candidate_id: u64, label: String) -> bool {
        if let Some(candidate) = self
            .draft_candidates
            .iter_mut()
            .find(|candidate| candidate.candidate_id == candidate_id)
        {
            candidate.label = label;
            return true;
        }
        false
    }

    pub fn active_batch(&self) -> Option<&PromptLabCompareBatchRecord> {
        self.active_batch_idx.and_then(|idx| self.batches.get(idx))
    }

    pub fn active_batch_mut(&mut self) -> Option<&mut PromptLabCompareBatchRecord> {
        self.active_batch_idx
            .and_then(|idx| self.batches.get_mut(idx))
    }

    pub fn record_compare_dispatch(
        &mut self,
        batch_id: PromptLabCompareBatchId,
        candidate_id: u64,
        run_id: PromptLabRunId,
    ) -> bool {
        let Some(batch) = self.batches.iter_mut().find(|b| b.batch_id == batch_id) else {
            return false;
        };
        let mut updated = false;
        for (cid, maybe_run) in &mut batch.candidate_run_ids {
            if *cid == candidate_id && maybe_run.is_none() {
                *maybe_run = Some(run_id);
                updated = true;
                break;
            }
        }
        if let PromptLabCompareBatchStatus::Running { dispatched, total } = batch.status {
            let new_dispatched = if updated {
                dispatched.saturating_add(1)
            } else {
                dispatched
            };
            batch.status = PromptLabCompareBatchStatus::Running {
                dispatched: new_dispatched,
                total,
            };
        }
        updated
    }

    pub fn set_batch_status(
        &mut self,
        batch_id: PromptLabCompareBatchId,
        status: PromptLabCompareBatchStatus,
    ) -> bool {
        if let Some(batch) = self.batches.iter_mut().find(|b| b.batch_id == batch_id) {
            batch.status = status;
            return true;
        }
        false
    }

    pub fn clear_active_batch_if(&mut self, batch_id: PromptLabCompareBatchId) {
        if self
            .active_batch_idx
            .and_then(|idx| self.batches.get(idx))
            .map(|batch| batch.batch_id == batch_id)
            .unwrap_or(false)
        {
            self.active_batch_idx = None;
        }
    }

    pub fn recompute_auto_select_for_batch(&mut self, batch_id: PromptLabCompareBatchId) {
        let Some(batch_idx) = self.batches.iter().position(|b| b.batch_id == batch_id) else {
            return;
        };
        let policy = self.compare_policy.clone();
        let batch = self.batches[batch_idx].clone();
        let scored = batch
            .candidate_run_ids
            .iter()
            .filter_map(|(candidate_id, run_id)| {
                let run_id = (*run_id)?;
                let run = self.run_by_id(run_id)?;
                let candidate = batch
                    .candidates
                    .iter()
                    .find(|c| c.candidate_id == *candidate_id)?;
                Some((run_id, run, candidate))
            });
        let (winner, warning) = cheap_enough_select(scored, &policy);
        if let Some(batch_mut) = self.batches.get_mut(batch_idx) {
            batch_mut.auto_selected_run_id = winner;
            batch_mut.auto_select_warning = warning;
        }
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

    pub(crate) fn set_template_editor_open(&mut self, open: bool) {
        if open {
            self.advanced_mode = true;
            self.template_section_open = true;
        }
        self.template_editor_open = open;
        self.normalize_visibility_invariants();
    }

    pub(crate) fn template_editor_open(&self) -> bool {
        self.template_editor_open
    }

    pub fn set_advanced_mode(&mut self, enabled: bool) {
        self.advanced_mode = enabled;
        self.normalize_visibility_invariants();
    }

    pub fn advanced_mode(&self) -> bool {
        self.advanced_mode
    }

    pub fn toggle_compare_section(&mut self) {
        self.compare_section_open = !self.compare_section_open;
    }

    pub fn compare_section_open(&self) -> bool {
        self.compare_section_open
    }

    pub fn toggle_context_section(&mut self) {
        self.context_section_open = !self.context_section_open;
    }

    pub fn context_section_open(&self) -> bool {
        self.context_section_open
    }

    pub fn toggle_template_section(&mut self) {
        self.template_section_open = !self.template_section_open;
        self.normalize_visibility_invariants();
    }

    pub fn template_section_open(&self) -> bool {
        self.template_section_open
    }

    pub fn toggle_run_details_section(&mut self) {
        self.run_details_section_open = !self.run_details_section_open;
    }

    pub fn run_details_section_open(&self) -> bool {
        self.run_details_section_open
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

    pub(crate) fn open_template_draft(
        &mut self,
        prompt_id: PromptId,
        system_template: &str,
        user_template: &str,
        description: &str,
        expected_format: &str,
    ) {
        self.template_drafts.insert(
            prompt_id,
            PromptLabTemplateDraft::new(
                system_template,
                user_template,
                description,
                expected_format,
            ),
        );
    }

    pub(crate) fn update_template_system(&mut self, prompt_id: PromptId, text: String) -> bool {
        if let Some(draft) = self.template_drafts.get_mut(&prompt_id) {
            draft.update_system(text);
            true
        } else {
            false
        }
    }

    pub(crate) fn update_template_user(&mut self, prompt_id: PromptId, text: String) -> bool {
        if let Some(draft) = self.template_drafts.get_mut(&prompt_id) {
            draft.update_user(text);
            true
        } else {
            false
        }
    }

    pub(crate) fn apply_template(
        &mut self,
        prompt_id: PromptId,
        errors: Vec<TemplateValidationError>,
    ) -> bool {
        if let Some(draft) = self.template_drafts.get_mut(&prompt_id) {
            draft.apply(errors)
        } else {
            false
        }
    }

    pub(crate) fn revert_template(&mut self, prompt_id: PromptId) -> bool {
        if let Some(draft) = self.template_drafts.get_mut(&prompt_id) {
            draft.revert();
            true
        } else {
            false
        }
    }

    pub(crate) fn mark_template_saved(
        &mut self,
        prompt_id: PromptId,
        version: PromptVersion,
        path: PathBuf,
    ) {
        if let Some(draft) = self.template_drafts.get_mut(&prompt_id) {
            draft.mark_saved(version, path);
        }
    }

    pub(crate) fn template_draft(&self, prompt_id: PromptId) -> Option<&PromptLabTemplateDraft> {
        self.template_drafts.get(&prompt_id)
    }

    pub(crate) fn applied_template_override(
        &self,
        prompt_id: PromptId,
    ) -> Option<PromptTemplateOwned> {
        self.template_drafts.get(&prompt_id).and_then(|draft| {
            if draft.is_applied() && draft.validation_errors().is_empty() {
                Some(PromptTemplateOwned {
                    id: prompt_id,
                    version: PROMPT_VERSION_DRAFT,
                    system_template: draft.system_draft.clone(),
                    user_template: draft.user_draft.clone(),
                    description: draft.description.clone(),
                    expected_format: draft.expected_format.clone(),
                })
            } else {
                None
            }
        })
    }
}

pub fn cheap_enough_select<'a>(
    candidates: impl Iterator<
        Item = (
            PromptLabRunId,
            &'a PromptLabRunRecord,
            &'a PromptLabCompareCandidate,
        ),
    >,
    policy: &PromptLabComparePolicy,
) -> (Option<PromptLabRunId>, Option<String>) {
    let rows = candidates.collect::<Vec<_>>();
    if rows.is_empty() {
        return (None, Some("no candidates".to_string()));
    }
    let terminal = rows
        .into_iter()
        .filter(|(_, run, _)| !matches!(run.status, PromptLabRunStatus::Pending { .. }))
        .collect::<Vec<_>>();
    if terminal.is_empty() {
        return (None, Some("no terminal runs".to_string()));
    }
    let succeeded = terminal
        .into_iter()
        .filter(|(_, run, _)| !matches!(run.status, PromptLabRunStatus::Failed { .. }))
        .collect::<Vec<_>>();
    if succeeded.is_empty() {
        return (None, Some("all failed".to_string()));
    }
    let parse_ok = succeeded
        .into_iter()
        .filter(|(_, run, _)| {
            if !policy.require_parse_ok {
                return true;
            }
            let parse_ok = match &run.status {
                PromptLabRunStatus::Completed { metadata, .. } => metadata.parse_ok,
                PromptLabRunStatus::Failed { metadata, .. } => {
                    metadata.as_ref().map(|m| m.parse_ok).unwrap_or(false)
                }
                PromptLabRunStatus::Pending { .. } => false,
            };
            parse_ok
        })
        .collect::<Vec<_>>();
    if parse_ok.is_empty() {
        return (None, Some("parse gate".to_string()));
    }
    let cost_ok = parse_ok
        .into_iter()
        .filter(|(_, run, _)| match policy.max_cost_microdollars {
            Some(limit) => run_metadata(run)
                .map(|m| m.cost_microdollars <= limit)
                .unwrap_or(false),
            None => true,
        })
        .collect::<Vec<_>>();
    if cost_ok.is_empty() {
        return (None, Some("cost gate".to_string()));
    }
    let mut wall_ok = cost_ok
        .into_iter()
        .filter(|(_, run, _)| match policy.max_wall_ms {
            Some(limit) => run_metadata(run)
                .map(|m| m.wall_ms <= limit)
                .unwrap_or(false),
            None => true,
        })
        .collect::<Vec<_>>();
    if wall_ok.is_empty() {
        return (None, Some("wall gate".to_string()));
    }
    wall_ok.sort_by(|(run_id_a, run_a, _), (run_id_b, run_b, _)| {
        let meta_a = run_metadata(run_a).expect("filtered to terminal successful runs");
        let meta_b = run_metadata(run_b).expect("filtered to terminal successful runs");
        let rating_a = run_a.operator_rating.unwrap_or(0);
        let rating_b = run_b.operator_rating.unwrap_or(0);
        if policy.rating_beats_cost {
            rating_b
                .cmp(&rating_a)
                .then(meta_a.cost_microdollars.cmp(&meta_b.cost_microdollars))
                .then(meta_a.wall_ms.cmp(&meta_b.wall_ms))
                .then(run_id_a.0.cmp(&run_id_b.0))
        } else {
            meta_a
                .cost_microdollars
                .cmp(&meta_b.cost_microdollars)
                .then(meta_a.wall_ms.cmp(&meta_b.wall_ms))
                .then(rating_b.cmp(&rating_a))
                .then(run_id_a.0.cmp(&run_id_b.0))
        }
    });
    (wall_ok.first().map(|(run_id, _, _)| *run_id), None)
}

fn run_metadata(run: &PromptLabRunRecord) -> Option<&LlmRunMetadata> {
    match &run.status {
        PromptLabRunStatus::Completed { metadata, .. } => Some(metadata),
        PromptLabRunStatus::Failed { metadata, .. } => metadata.as_ref(),
        PromptLabRunStatus::Pending { .. } => None,
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
    fn close_resets_template_editor_open_invariant() {
        let mut s = PromptLabState::default();
        s.open();
        s.set_template_editor_open(true);
        assert!(s.template_editor_open());
        s.close();
        assert!(!s.template_editor_open());
    }

    #[test]
    fn disabling_advanced_mode_closes_template_editor() {
        let mut s = PromptLabState::default();
        s.open();
        s.set_template_editor_open(true);
        assert!(s.template_editor_open());
        s.set_advanced_mode(false);
        assert!(!s.template_editor_open());
    }

    #[test]
    fn closing_template_section_closes_template_editor() {
        let mut s = PromptLabState::default();
        s.open();
        s.set_template_editor_open(true);
        assert!(s.template_editor_open());
        assert!(s.template_section_open());
        s.toggle_template_section();
        assert!(!s.template_section_open());
        assert!(!s.template_editor_open());
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
            PromptLabRunOverrides::default(),
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
            PromptLabRunOverrides::default(),
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
            PromptLabRunOverrides::default(),
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
            PromptLabRunOverrides::default(),
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
            PromptLabRunOverrides::default(),
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
            PromptLabRunOverrides::default(),
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
            PromptLabRunOverrides::default(),
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
            PromptLabRunOverrides::default(),
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
            PromptLabRunOverrides::default(),
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

    #[test]
    fn freeze_batch_rejects_less_than_two_candidates() {
        let mut state = PromptLabState::default();
        assert!(state.freeze_batch("input".to_string()).is_err());
        state.add_baseline_candidate(None).expect("baseline");
        assert!(state.freeze_batch("input".to_string()).is_err());
    }

    #[test]
    fn freeze_batch_with_two_candidates_creates_running_batch() {
        let mut state = PromptLabState::default();
        state
            .add_draft_candidate_from_current(Some("A".to_string()))
            .expect("candidate A");
        state
            .add_baseline_candidate(Some("B".to_string()))
            .expect("candidate B");
        let batch_id = state.freeze_batch("input".to_string()).expect("batch");
        let batch = state
            .batches()
            .iter()
            .find(|batch| batch.batch_id == batch_id)
            .expect("batch exists");
        assert!(matches!(
            batch.status,
            PromptLabCompareBatchStatus::Running { .. }
        ));
        assert_eq!(batch.pending_candidate_count(), 2);
        assert!(state.active_batch().is_some());
    }

    #[test]
    fn cheap_enough_prefers_cheapest_with_deterministic_tie_break() {
        let mut state = PromptLabState::default();
        let run_1 = PromptLabRunId(1);
        state.add_pending_run(
            run_1,
            PromptLabStage::Triage,
            PromptId::ArticleTriage,
            "in".to_string(),
            10,
            PromptLabRunOverrides::default(),
        );
        state.complete_run(run_1, "{}".to_string(), LlmRunMetadata::stub());
        state.consume_ownership(10);
        let run_2 = PromptLabRunId(2);
        state.add_pending_run(
            run_2,
            PromptLabStage::Triage,
            PromptId::ArticleTriage,
            "in".to_string(),
            11,
            PromptLabRunOverrides::default(),
        );
        state.complete_run(run_2, "{}".to_string(), LlmRunMetadata::stub());
        state.consume_ownership(11);
        let candidates = [
            PromptLabCompareCandidate {
                candidate_id: 1,
                stage: PromptLabStage::Triage,
                prompt_id: PromptId::ArticleTriage,
                prompt_version: None,
                model_override: None,
                context_snapshot: Vec::new(),
                template_snapshot: None,
                label: "c1".to_string(),
            },
            PromptLabCompareCandidate {
                candidate_id: 2,
                stage: PromptLabStage::Triage,
                prompt_id: PromptId::ArticleTriage,
                prompt_version: None,
                model_override: None,
                context_snapshot: Vec::new(),
                template_snapshot: None,
                label: "c2".to_string(),
            },
        ];
        let rows = vec![
            (
                run_1,
                state.run_by_id(run_1).expect("run_1"),
                candidates.first().expect("candidate 1"),
            ),
            (
                run_2,
                state.run_by_id(run_2).expect("run_2"),
                candidates.get(1).expect("candidate 2"),
            ),
        ];
        let policy = PromptLabComparePolicy::default();
        let (winner, warning) = cheap_enough_select(rows.into_iter(), &policy);
        assert_eq!(winner, Some(run_1));
        assert!(warning.is_none());
    }
}
