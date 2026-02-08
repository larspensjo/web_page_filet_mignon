use async_trait::async_trait;

use super::types::{LlmError, LlmRequest, LlmResponse};

/// Trait that abstracts over different LLM backends.
#[async_trait]
pub trait LlmProvider: Send + Sync {
    /// Performs a single completion request.
    async fn complete(&self, request: &LlmRequest) -> Result<LlmResponse, LlmError>;

    /// A human-readable provider identifier (e.g., "openai").
    fn provider_name(&self) -> &str;
}
