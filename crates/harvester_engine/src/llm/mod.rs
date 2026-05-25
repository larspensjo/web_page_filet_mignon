pub mod dto;
pub mod handle;
pub mod mock_provider;
pub mod pricing;
pub mod prompt;
pub mod prompt_context;
pub mod prompts;
pub mod provider;
pub mod providers;
pub mod quota;
pub mod replay;
pub mod run_metadata;
pub mod template_validation;
pub mod types;
pub mod validation;

pub const OPENAI_MODEL_GPT_4O: &str = "gpt-4o";
pub const OPENAI_MODEL_GPT_4O_MINI: &str = "gpt-4o-mini";
pub const OPENAI_MODEL_GPT_5_4: &str = "gpt-5.4";
pub const OPENAI_MODEL_GPT_5_4_MINI: &str = "gpt-5.4-mini";
pub const OPENAI_MODEL_GPT_5_4_NANO: &str = "gpt-5.4-nano";
pub const OPENAI_MODEL_GPT_5_4_PRO: &str = "gpt-5.4-pro";

pub const DEFAULT_TRIAGE_MODEL: &str = OPENAI_MODEL_GPT_5_4_NANO;
pub const DEFAULT_SUMMARY_MODEL: &str = OPENAI_MODEL_GPT_5_4_MINI;
pub const DEFAULT_BRIEFING_MODEL: &str = OPENAI_MODEL_GPT_5_4_MINI;

pub use dto::{
    AggregateBriefing, ArticleSummary, BriefingStory, Confidence, SignalCandidateResult,
    SourceTier, SummaryEntities, TriagePriority, TriageResult,
};
pub use handle::{
    LlmCommand, LlmCompletionCommand, LlmCompletionError, LlmCompletionResult, LlmConfig, LlmEvent,
    LlmHandle,
};
pub use mock_provider::{BlockingMockProvider, MockLlmProvider};
pub use pricing::{ModelPricing, PricingRegistry};
pub use prompt::{
    is_draft_version, EffectiveTemplate, PromptId, PromptRegistry, PromptTemplate,
    PromptTemplateOwned, PromptVersion, TemplateSource, TemplateVars, PROMPT_VERSION_DRAFT,
};
pub use prompt_context::{
    load_context_file, validate_context_covers_template, ContextLoadError, ContextMeta,
    PromptContextFile,
};
pub use provider::LlmProvider;
pub use providers::OpenAiProvider;
pub use quota::{LlmQuotaTracker, LlmQuotas, LlmUsageTotals};
pub use replay::{
    content_hash, load_replay_record, persist_replay_record, ReplayProvider, ReplayRecord,
};
pub use run_metadata::{CacheStatus, LlmFailureMetadata, LlmRunMetadata};
pub use template_validation::{validate_template, TemplateField, TemplateValidationError};
pub use types::{
    ChatMessage, ChatRole, FinishReason, LlmError, LlmRequest, LlmResponse, ModelId, ProviderKind,
    ResponseFormat, TokenUsage,
};
pub use validation::{
    validate_briefing, validate_signal_candidate, validate_summary, validate_triage,
    ValidationError,
};
