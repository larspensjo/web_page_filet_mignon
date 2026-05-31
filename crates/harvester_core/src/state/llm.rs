use super::{AppState, LlmRequestState, MAX_IN_FLIGHT_LIMIT};
use crate::view_model::LlmModelUsageView;
use crate::{LlmQuotaLimits, LlmQuotaState, LlmQuotaUsage};
use harvester_engine::llm::prompt::PromptId;
use harvester_engine::llm::run_metadata::{CacheStatus, LlmRunMetadata};

impl AppState {
    /// Set the maximum concurrent triage LLM requests. Clamped to MAX_IN_FLIGHT_LIMIT.
    pub fn set_triage_max_in_flight(&mut self, limit: usize) {
        self.triage_max_in_flight = limit.clamp(1, MAX_IN_FLIGHT_LIMIT);
    }

    /// Set the maximum concurrent summary LLM requests. Clamped to MAX_IN_FLIGHT_LIMIT.
    pub fn set_summary_max_in_flight(&mut self, limit: usize) {
        self.summary_max_in_flight = limit.clamp(1, MAX_IN_FLIGHT_LIMIT);
    }

    pub fn triage_max_in_flight(&self) -> usize {
        self.triage_max_in_flight
    }

    pub fn summary_max_in_flight(&self) -> usize {
        self.summary_max_in_flight
    }

    pub fn llm_request_state(&self, request_id: u64) -> Option<&LlmRequestState> {
        self.llm_requests.get(&request_id)
    }

    pub fn allocate_next_llm_request_id(&mut self) -> u64 {
        let id = self.next_llm_request_id;
        self.next_llm_request_id = self.next_llm_request_id.saturating_add(1);
        id
    }

    /// Records LLM token usage from a completed run.
    /// Only CacheStatus::Miss runs are counted; empty or whitespace-only model names are ignored.
    pub fn record_llm_usage_from_metadata(&mut self, metadata: &LlmRunMetadata) {
        if metadata.cache_status != CacheStatus::Miss {
            return;
        }
        let model = metadata.resolved_model.trim();
        if model.is_empty() {
            return;
        }
        let entry = self
            .llm_usage_by_model
            .entry(model.to_string())
            .or_default();
        entry.0 = entry.0.saturating_add(u64::from(metadata.input_tokens));
        entry.1 = entry.1.saturating_add(u64::from(metadata.output_tokens));
    }

    /// Returns a sorted (alphabetical) snapshot of per-model token usage for rendering.
    pub fn llm_usage_rows(&self) -> Vec<LlmModelUsageView> {
        self.llm_usage_by_model
            .iter()
            .map(
                |(model, &(input_tokens, output_tokens))| LlmModelUsageView {
                    model: model.clone(),
                    input_tokens,
                    output_tokens,
                },
            )
            .collect()
    }

    pub fn llm_quota(&self) -> &LlmQuotaState {
        &self.llm_quota
    }

    pub(crate) fn set_llm_quota_limits(&mut self, limits: LlmQuotaLimits) {
        self.llm_quota.limits = Some(limits);
        self.llm_quota.ai_available = true;
    }

    pub(crate) fn set_llm_quota_usage(&mut self, usage: LlmQuotaUsage) {
        self.llm_quota.usage = usage;
    }

    pub fn record_pending_llm_request(&mut self, request_id: u64, prompt_id: PromptId) {
        self.llm_requests
            .insert(request_id, LlmRequestState::Pending { prompt_id });
    }

    pub fn record_llm_result(&mut self, request_id: u64, state: LlmRequestState) {
        self.llm_requests.insert(request_id, state);
    }

    pub fn reset_llm_requests(&mut self) {
        self.llm_requests.clear();
        self.next_llm_request_id = 1;
    }
}
