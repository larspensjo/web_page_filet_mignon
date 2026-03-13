use serde::{Deserialize, Serialize};
use thiserror::Error;

/// Identifies an LLM model along with its provider.
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct ModelId {
    provider: ProviderKind,
    model_name: String,
}

impl ModelId {
    pub fn new(provider: ProviderKind, model_name: impl Into<String>) -> Self {
        Self {
            provider,
            model_name: model_name.into(),
        }
    }

    pub fn provider(&self) -> ProviderKind {
        self.provider
    }

    pub fn model_name(&self) -> &str {
        &self.model_name
    }
}

/// External LLM vendors supported today.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProviderKind {
    OpenAi,
    Anthropic,
    Google,
}

/// Roles for chat-based payloads.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ChatRole {
    System,
    User,
    Assistant,
}

/// A single chat message that can be rendered into completion requests.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ChatMessage {
    role: ChatRole,
    content: String,
}

impl ChatMessage {
    pub fn new(role: ChatRole, content: impl Into<String>) -> Self {
        Self {
            role,
            content: content.into(),
        }
    }

    pub fn role(&self) -> ChatRole {
        self.role
    }

    pub fn content(&self) -> &str {
        &self.content
    }
}

/// Format hint for LLM responses.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ResponseFormat {
    Text,
    Json,
}

/// Reason describing why a completion stream finished.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FinishReason {
    Stop,
    MaxTokens,
    ContentFilter,
    Unknown,
}

/// Token counts returned by the provider.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct TokenUsage {
    pub input_tokens: u32,
    pub output_tokens: u32,
    /// Portion of `input_tokens` served from the provider's prompt cache.
    /// Always <= `input_tokens`; zero when the provider does not report cache hits.
    #[serde(default)]
    pub cached_input_tokens: u32,
}

impl TokenUsage {
    pub fn new(input_tokens: u32, output_tokens: u32) -> Self {
        Self {
            input_tokens,
            output_tokens,
            cached_input_tokens: 0,
        }
    }

    /// Builder that sets `cached_input_tokens`, clamping the value to `input_tokens`
    /// so that the invariant `cached_input_tokens <= input_tokens` always holds.
    pub fn with_cached_input_tokens(mut self, cached: u32) -> Self {
        self.cached_input_tokens = cached.min(self.input_tokens);
        self
    }

    pub fn total(&self) -> u32 {
        self.input_tokens.saturating_add(self.output_tokens)
    }
}

/// Core completion payload shared by all providers.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct LlmRequest {
    model: ModelId,
    messages: Vec<ChatMessage>,
    temperature: Option<f32>,
    max_output_tokens: Option<u32>,
    response_format: ResponseFormat,
}

impl LlmRequest {
    pub fn new(model: ModelId, messages: Vec<ChatMessage>) -> Self {
        Self {
            model,
            messages,
            temperature: None,
            max_output_tokens: None,
            response_format: ResponseFormat::Text,
        }
    }

    pub fn model(&self) -> &ModelId {
        &self.model
    }

    pub fn messages(&self) -> &[ChatMessage] {
        &self.messages
    }

    pub fn temperature(&self) -> Option<f32> {
        self.temperature
    }

    pub fn max_output_tokens(&self) -> Option<u32> {
        self.max_output_tokens
    }

    pub fn response_format(&self) -> ResponseFormat {
        self.response_format
    }

    pub fn with_temperature(mut self, temperature: f32) -> Self {
        self.temperature = Some(temperature);
        self
    }

    pub fn with_max_output_tokens(mut self, tokens: u32) -> Self {
        self.max_output_tokens = Some(tokens);
        self
    }

    pub fn with_response_format(mut self, response_format: ResponseFormat) -> Self {
        self.response_format = response_format;
        self
    }

    pub fn with_json_response(self) -> Self {
        self.with_response_format(ResponseFormat::Json)
    }
}

/// Provider-agnostic completion response.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct LlmResponse {
    content: String,
    usage: TokenUsage,
    model_id: ModelId,
    finish_reason: FinishReason,
}

impl LlmResponse {
    pub fn new(
        content: impl Into<String>,
        usage: TokenUsage,
        model_id: ModelId,
        finish_reason: FinishReason,
    ) -> Self {
        Self {
            content: content.into(),
            usage,
            model_id,
            finish_reason,
        }
    }

    pub fn content(&self) -> &str {
        &self.content
    }

    pub fn usage(&self) -> TokenUsage {
        self.usage
    }

    pub fn model_id(&self) -> &ModelId {
        &self.model_id
    }

    pub fn finish_reason(&self) -> FinishReason {
        self.finish_reason
    }
}

/// Rich error categories emitted by provider bindings.
#[derive(Clone, Debug, PartialEq, Eq, Error, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum LlmError {
    #[error("HTTP {status} error: {body}")]
    Http { status: u16, body: String },

    #[error("rate limited; retry after {retry_after_secs:?} seconds")]
    RateLimited { retry_after_secs: Option<u64> },

    #[error("authentication failed")]
    AuthenticationFailed,

    #[error("invalid response: {detail}")]
    InvalidResponse { detail: String },

    #[error("network failure: {detail}")]
    Network { detail: String },

    #[error("request timed out")]
    Timeout,

    #[error("quota exhausted: {description}")]
    QuotaExhausted { description: String },

    #[error("configuration error: {detail}")]
    Configuration { detail: String },

    #[error("content filtered by provider policy")]
    ContentFiltered,
}

impl LlmError {
    /// Returns `true` when the error is transient and the request may be retried.
    ///
    /// Retryable: `Timeout`, `Network`, and `Http` with status 500/502/503/504.
    /// Non-retryable: `AuthenticationFailed`, `RateLimited`, `QuotaExhausted`,
    /// `Configuration`, `InvalidResponse`, `ContentFiltered`.
    pub fn is_retryable(&self) -> bool {
        match self {
            LlmError::Timeout | LlmError::Network { .. } => true,
            LlmError::Http { status, .. } => {
                matches!(status, 500 | 502 | 503 | 504)
            }
            LlmError::RateLimited { .. }
            | LlmError::AuthenticationFailed
            | LlmError::InvalidResponse { .. }
            | LlmError::QuotaExhausted { .. }
            | LlmError::Configuration { .. }
            | LlmError::ContentFiltered => false,
        }
    }
}
