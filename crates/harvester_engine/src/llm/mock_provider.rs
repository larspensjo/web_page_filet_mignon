use std::{
    collections::VecDeque,
    sync::{
        atomic::{AtomicUsize, Ordering},
        Arc, Mutex,
    },
};

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

/// A mock provider that tracks concurrent in-flight requests and can be
/// gated to block responses until explicitly released. Used in tests that
/// verify concurrency cap invariants.
pub struct BlockingMockProvider {
    /// Response to return for every request.
    response_content: String,
    /// Gate semaphore: each permit allows one queued request to return.
    gate: Arc<tokio::sync::Semaphore>,
    /// Current number of in-flight requests.
    current_in_flight: Arc<AtomicUsize>,
    /// Peak simultaneous in-flight count observed.
    peak_in_flight: Arc<AtomicUsize>,
}

impl BlockingMockProvider {
    pub fn new(response_content: impl Into<String>) -> Self {
        Self {
            response_content: response_content.into(),
            gate: Arc::new(tokio::sync::Semaphore::new(0)),
            current_in_flight: Arc::new(AtomicUsize::new(0)),
            peak_in_flight: Arc::new(AtomicUsize::new(0)),
        }
    }

    /// Release `count` blocked requests to return.
    pub fn release(&self, count: usize) {
        self.gate.add_permits(count);
    }

    /// Current in-flight count.
    pub fn current_in_flight(&self) -> usize {
        self.current_in_flight.load(Ordering::SeqCst)
    }

    /// Peak simultaneous in-flight count observed across all requests.
    pub fn peak_in_flight(&self) -> usize {
        self.peak_in_flight.load(Ordering::SeqCst)
    }

    /// Clones that share the same counters and gate (for moving into the worker thread).
    pub fn shared_counters(&self) -> (Arc<AtomicUsize>, Arc<AtomicUsize>) {
        (self.current_in_flight.clone(), self.peak_in_flight.clone())
    }
}

#[async_trait::async_trait]
impl LlmProvider for BlockingMockProvider {
    async fn complete(&self, _request: &LlmRequest) -> Result<LlmResponse, LlmError> {
        // Track entry.
        let prev = self.current_in_flight.fetch_add(1, Ordering::SeqCst);
        let current = prev + 1;
        // Update peak.
        let mut peak = self.peak_in_flight.load(Ordering::SeqCst);
        while current > peak {
            match self.peak_in_flight.compare_exchange(
                peak,
                current,
                Ordering::SeqCst,
                Ordering::SeqCst,
            ) {
                Ok(_) => break,
                Err(actual) => peak = actual,
            }
        }
        // Block until a permit is available.
        let _permit = self.gate.acquire().await.expect("gate closed");
        self.current_in_flight.fetch_sub(1, Ordering::SeqCst);

        Ok(LlmResponse::new(
            self.response_content.clone(),
            TokenUsage::new(0, 0),
            ModelId::new(ProviderKind::OpenAi, "mock"),
            FinishReason::Stop,
        ))
    }

    fn provider_name(&self) -> &str {
        "blocking-mock"
    }
}
