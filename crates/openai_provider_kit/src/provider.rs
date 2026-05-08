use async_trait::async_trait;

use crate::types::{LlmError, LlmRequest, LlmResponse};

/// Trait that abstracts over different LLM backends.
#[async_trait]
pub trait LlmProvider: Send + Sync {
    /// Performs a single completion request.
    async fn complete(&self, request: &LlmRequest) -> Result<LlmResponse, LlmError>;

    /// A human-readable provider identifier (e.g., "openai").
    fn provider_name(&self) -> &str;

    /// Discover available models from the provider.
    /// Returns a list of model name strings that can be used in requests.
    /// Default implementation returns an error; providers opt in to discovery.
    async fn list_models(&self) -> Result<Vec<String>, LlmError> {
        Err(LlmError::Configuration {
            detail: "model discovery not supported by this provider".into(),
        })
    }
}
