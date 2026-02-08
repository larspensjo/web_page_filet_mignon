use std::{collections::VecDeque, sync::Mutex};

use super::provider::LlmProvider;
use super::types::{
    FinishReason, LlmError, LlmRequest, LlmResponse, ModelId, ProviderKind, TokenUsage,
};

/// In-memory LLM provider used for testing or development.
pub struct MockLlmProvider {
    responses: Mutex<VecDeque<Result<LlmResponse, LlmError>>>,
    recorded_requests: Mutex<Vec<LlmRequest>>,
}

impl Default for MockLlmProvider {
    fn default() -> Self {
        Self::new()
    }
}

impl MockLlmProvider {
    pub fn new() -> Self {
        Self {
            responses: Mutex::new(VecDeque::new()),
            recorded_requests: Mutex::new(Vec::new()),
        }
    }

    pub fn queue_response(&self, response: Result<LlmResponse, LlmError>) -> &Self {
        let mut guard = self
            .responses
            .lock()
            .expect("MockLlmProvider response queue mutex poisoned");
        guard.push_back(response);
        self
    }

    pub fn queue_json_success(&self, content: impl Into<String>) -> &Self {
        let response = LlmResponse::new(
            content,
            TokenUsage::new(0, 0),
            ModelId::new(ProviderKind::OpenAi, "mock"),
            FinishReason::Stop,
        );
        self.queue_response(Ok(response))
    }

    pub fn recorded_requests(&self) -> Vec<LlmRequest> {
        let guard = self
            .recorded_requests
            .lock()
            .expect("MockLlmProvider request log mutex poisoned");
        guard.clone()
    }
}

#[async_trait::async_trait]
impl LlmProvider for MockLlmProvider {
    async fn complete(&self, request: &LlmRequest) -> Result<LlmResponse, LlmError> {
        self.recorded_requests
            .lock()
            .expect("MockLlmProvider request log mutex poisoned")
            .push(request.clone());
        let mut responses = self
            .responses
            .lock()
            .expect("MockLlmProvider response queue mutex poisoned");
        responses
            .pop_front()
            .ok_or_else(|| LlmError::InvalidResponse {
                detail: "mock response queue empty".to_string(),
            })
            .and_then(|res| res)
    }

    fn provider_name(&self) -> &str {
        "mock"
    }
}
