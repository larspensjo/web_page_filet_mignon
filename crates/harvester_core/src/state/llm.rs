use super::{AppState, LlmRequestState};
use crate::view_model::LlmModelUsageView;
use crate::{LlmQuotaLimits, LlmQuotaState, LlmQuotaUsage};
use harvester_engine::llm::prompt::PromptId;
use harvester_engine::llm::run_metadata::{CacheStatus, LlmRunMetadata};

/// Latest adaptive cap change, observed and logged at the host boundary.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LlmCapChange {
    pub from: usize,
    pub to: usize,
    pub reason: &'static str,
    pub request_id: u64,
}

impl AppState {
    /// Budget shared by triage, summaries and signal scoring.
    pub fn set_llm_max_in_flight(&mut self, limit: usize) {
        self.llm_max_in_flight = limit.clamp(1, harvester_engine::llm::MAX_LLM_CONCURRENT_REQUESTS);
        self.reset_effective_llm_cap();
        self.log_model_budget();
    }
    fn log_model_budget(&self) {
        engine_logging::engine_info!(
            "[model-budget] llm_max_in_flight={}",
            self.llm_max_in_flight
        );
    }
    pub fn llm_max_in_flight(&self) -> usize {
        self.llm_max_in_flight
    }

    pub fn effective_llm_cap(&self) -> usize {
        self.effective_llm_cap
    }

    /// Read-only diagnostics for the host's reducer observer.
    pub fn llm_cap_change_count(&self) -> u64 {
        self.llm_cap_change_count
    }

    /// Latest change for the host's reducer observer.
    pub fn last_llm_cap_change(&self) -> Option<LlmCapChange> {
        self.last_llm_cap_change
    }

    pub(crate) fn reset_effective_llm_cap(&mut self) {
        self.effective_llm_cap = self.llm_max_in_flight;
        self.cap_epoch_request_id = self.next_llm_request_id;
        self.clean_llm_completions = 0;
        self.llm_cap_change_count = 0;
        self.last_llm_cap_change = None;
    }

    pub(crate) fn note_article_model_pressure(&mut self, request_id: u64, rate_limited: bool) {
        let from = self.effective_llm_cap;
        let reason = if rate_limited {
            self.clean_llm_completions = 0;
            if request_id < self.cap_epoch_request_id {
                return;
            }
            self.effective_llm_cap = (from / 2).max(1);
            // Older in-flight completions cannot reduce the cap a second time.
            self.cap_epoch_request_id = self.next_llm_request_id;
            "rate_limited"
        } else {
            self.clean_llm_completions += 1;
            if self.clean_llm_completions < 10 {
                return;
            }
            self.clean_llm_completions = 0;
            self.effective_llm_cap = (from + 1).min(self.llm_max_in_flight);
            "recovered"
        };
        if from != self.effective_llm_cap {
            self.llm_cap_change_count += 1;
            self.last_llm_cap_change = Some(LlmCapChange {
                from,
                to: self.effective_llm_cap,
                reason,
                request_id,
            });
        }
    }
    pub(crate) fn model_dispatch_halt_reason(&self) -> Option<&str> {
        self.model_dispatch_halt_reason
            .as_ref()
            .map(|halt| halt.reason())
    }
    /// Existing provider halt status, exposed only for host-boundary diagnostics.
    pub fn provider_dispatch_halted(&self) -> bool {
        matches!(
            self.model_dispatch_halt_reason,
            Some(ModelDispatchHalt::ProviderCredits(_) | ModelDispatchHalt::RateLimit(_))
        )
    }
    pub(crate) fn halt_model_dispatch(&mut self, halt: ModelDispatchHalt) {
        if self.model_dispatch_halt_reason.is_none()
            || (matches!(halt, ModelDispatchHalt::SessionQuota(_))
                && self.session_quota_halt_reason().is_none())
        {
            engine_logging::engine_warn!(
                "[model-budget] article dispatch halted kind={:?} reason={}",
                halt.kind(),
                halt.reason()
            );
            self.model_dispatch_halt_reason = Some(halt);
            self.note_unfinished_global_inputs_changed();
            self.mark_dirty();
        }
    }
    pub(crate) fn reset_provider_model_dispatch_halt(&mut self) {
        if matches!(
            self.model_dispatch_halt_reason,
            Some(ModelDispatchHalt::ProviderCredits(_) | ModelDispatchHalt::RateLimit(_))
        ) {
            self.model_dispatch_halt_reason = None;
            self.mark_dirty();
        }
    }
    pub(crate) fn session_quota_halt_reason(&self) -> Option<&str> {
        match self.model_dispatch_halt_reason.as_ref() {
            Some(ModelDispatchHalt::SessionQuota(reason)) => Some(reason),
            _ => None,
        }
    }
    /// Article requests currently holding the shared model budget, for read-only observation.
    pub fn article_model_requests_in_flight(&self) -> usize {
        self.llm_requests
            .values()
            .filter(|request| {
                matches!(
                    request,
                    LlmRequestState::Pending {
                        prompt_id: PromptId::ArticleTriage
                            | PromptId::ArticleSummary
                            | PromptId::ArticleSignalCandidate
                    }
                )
            })
            .count()
    }

    pub fn llm_request_state(&self, request_id: u64) -> Option<&LlmRequestState> {
        self.llm_requests.get(&request_id)
    }

    /// Pending request ids for host monitoring and fixtures.
    pub fn pending_llm_request_ids(&self) -> impl Iterator<Item = u64> + '_ {
        self.llm_requests.iter().filter_map(|(request_id, state)| {
            matches!(state, LlmRequestState::Pending { .. }).then_some(*request_id)
        })
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

#[derive(Debug, Clone, PartialEq)]
pub(crate) enum ModelDispatchHalt {
    SessionQuota(String),
    ProviderCredits(String),
    RateLimit(String),
}

impl ModelDispatchHalt {
    fn reason(&self) -> &str {
        match self {
            Self::SessionQuota(reason)
            | Self::ProviderCredits(reason)
            | Self::RateLimit(reason) => reason,
        }
    }

    fn kind(&self) -> &'static str {
        match self {
            Self::SessionQuota(_) => "session_quota",
            Self::ProviderCredits(_) => "provider_credits",
            Self::RateLimit(_) => "rate_limit",
        }
    }
}
