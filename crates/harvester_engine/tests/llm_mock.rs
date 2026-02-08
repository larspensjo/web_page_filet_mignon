use harvester_engine::llm::{
    ChatMessage, ChatRole, FinishReason, LlmError, LlmProvider, LlmRequest, LlmResponse,
    MockLlmProvider, ModelId, ProviderKind, TokenUsage,
};

fn sample_request(model: ModelId) -> LlmRequest {
    LlmRequest::new(model, vec![ChatMessage::new(ChatRole::System, "prompt")])
}

fn sample_response(content: &str, model: &ModelId) -> LlmResponse {
    LlmResponse::new(
        content,
        TokenUsage::new(1, 2),
        model.clone(),
        FinishReason::Stop,
    )
}

#[tokio::test]
async fn fifo_responses_are_returned_first_in_first_out() {
    let provider = MockLlmProvider::new();
    let model = ModelId::new(ProviderKind::OpenAi, "mock-model");
    let response_a = sample_response("alpha", &model);
    let response_b = sample_response("beta", &model);
    provider.queue_response(Ok(response_a.clone()));
    provider.queue_response(Ok(response_b.clone()));

    let request = sample_request(model.clone());
    let first = provider.complete(&request).await.unwrap();
    let second = provider.complete(&request).await.unwrap();

    assert_eq!(first, response_a);
    assert_eq!(second, response_b);
}

#[tokio::test]
async fn records_all_requests_for_assertions() {
    let provider = MockLlmProvider::new();
    provider.queue_json_success("done");

    let model = ModelId::new(ProviderKind::OpenAi, "mock-model");
    let request = sample_request(model.clone());
    let _ = provider.complete(&request).await;

    assert_eq!(provider.recorded_requests(), vec![request]);
}

#[tokio::test]
async fn empty_queue_errors_immediately() {
    let provider = MockLlmProvider::new();
    let model = ModelId::new(ProviderKind::OpenAi, "mock-model");
    let request = sample_request(model);

    let err = provider.complete(&request).await.unwrap_err();
    assert!(matches!(err, LlmError::InvalidResponse { detail } if detail.contains("empty")));
}
