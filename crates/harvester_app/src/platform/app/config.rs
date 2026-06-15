use std::collections::HashMap;

use engine_logging::engine_info;
use harvester_core::LlmQuotaLimits;
use harvester_engine::llm::prompt::PromptId;
use harvester_engine::llm::{LlmConfig, LlmQuotas};

pub(super) const DEFAULT_LLM_MAX_CONCURRENT_REQUESTS: usize = 3;
pub(super) const LLM_MAX_CONCURRENT_REQUESTS_ENV: &str = "LLM_MAX_CONCURRENT_REQUESTS";
pub(super) const MAX_LLM_CONCURRENT_REQUESTS: usize = 10;

pub(super) fn parse_llm_max_concurrency_requests(raw: Option<&str>) -> usize {
    match raw {
        None => DEFAULT_LLM_MAX_CONCURRENT_REQUESTS,
        Some(value) => match value.trim().parse::<usize>() {
            Ok(parsed) => parsed.clamp(1, MAX_LLM_CONCURRENT_REQUESTS),
            Err(_) => DEFAULT_LLM_MAX_CONCURRENT_REQUESTS,
        },
    }
}

pub(super) fn llm_max_concurrency_requests_from_env() -> usize {
    let raw = std::env::var(LLM_MAX_CONCURRENT_REQUESTS_ENV).ok();
    let parsed = parse_llm_max_concurrency_requests(raw.as_deref());
    if let Some(value) = raw {
        engine_info!(
            "[llm-concurrency] {}='{}' -> {}",
            LLM_MAX_CONCURRENT_REQUESTS_ENV,
            value,
            parsed
        );
    }
    parsed
}

pub(super) fn effective_model_map(config: &LlmConfig) -> HashMap<PromptId, String> {
    let mut map = HashMap::new();

    let triage_model = config
        .triage_model
        .as_ref()
        .unwrap_or(&config.default_model)
        .model_name()
        .to_string();
    map.insert(PromptId::ArticleTriage, triage_model);

    let summary_model = config
        .summary_model
        .as_ref()
        .unwrap_or(&config.default_model)
        .model_name()
        .to_string();
    map.insert(PromptId::ArticleSummary, summary_model);

    let signal_candidate_model = config
        .signal_candidate_model
        .as_ref()
        .or(config.summary_model.as_ref())
        .unwrap_or(&config.default_model)
        .model_name()
        .to_string();
    map.insert(PromptId::ArticleSignalCandidate, signal_candidate_model);

    let briefing_model = config
        .briefing_model
        .as_ref()
        .unwrap_or(&config.default_model)
        .model_name()
        .to_string();
    map.insert(PromptId::AggregateBriefing, briefing_model.clone());
    map.insert(PromptId::BriefingExecutiveSummary, briefing_model.clone());
    map.insert(PromptId::BriefingNextItem, briefing_model);

    map
}

pub(super) fn llm_quota_limits_from_engine(quotas: &LlmQuotas) -> LlmQuotaLimits {
    LlmQuotaLimits {
        max_calls_per_session: quotas.max_calls_per_session.map(u64::from),
        max_input_tokens_per_session: quotas.max_input_tokens_per_session,
        max_output_tokens_per_session: quotas.max_output_tokens_per_session,
        max_cost_microdollars_per_session: quotas.max_cost_microdollars_per_session,
    }
}
