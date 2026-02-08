use harvester_engine::llm::{
    ChatMessage, ChatRole, FinishReason, LlmError, LlmRequest, LlmResponse, ModelId, ProviderKind,
    ResponseFormat, TokenUsage,
};

#[test]
fn request_builder_sets_optional_fields() {
    let model = ModelId::new(ProviderKind::OpenAi, "gpt-4.1");
    let messages = vec![ChatMessage::new(ChatRole::System, "Initial")];
    let request = LlmRequest::new(model.clone(), messages.clone())
        .with_temperature(0.2)
        .with_max_output_tokens(128)
        .with_json_response();

    assert_eq!(request.model(), &model);
    assert_eq!(request.messages(), messages.as_slice());
    assert_eq!(request.temperature(), Some(0.2));
    assert_eq!(request.max_output_tokens(), Some(128));
    assert_eq!(request.response_format(), ResponseFormat::Json);
}

#[test]
fn serde_roundtrips_request_and_response() {
    let model = ModelId::new(ProviderKind::Anthropic, "claude-v1");
    let request = LlmRequest::new(
        model.clone(),
        vec![ChatMessage::new(ChatRole::User, "Hello")],
    )
    .with_temperature(1.0)
    .with_max_output_tokens(32);

    let request_bytes = serde_json::to_vec(&request).expect("serialize request");
    let restored_request: LlmRequest =
        serde_json::from_slice(&request_bytes).expect("deserialize request");
    assert_eq!(request, restored_request);

    let response = LlmResponse::new(
        "OK",
        TokenUsage::new(10, 5),
        model.clone(),
        FinishReason::Stop,
    );

    let response_bytes = serde_json::to_vec(&response).expect("serialize response");
    let restored_response: LlmResponse =
        serde_json::from_slice(&response_bytes).expect("deserialize response");
    assert_eq!(response, restored_response);
}

#[test]
fn token_usage_total_saturates() {
    let usage = TokenUsage::new(u32::MAX, 1);
    assert_eq!(usage.total(), u32::MAX);
}

#[test]
fn llm_error_serde_roundtrip() {
    let error = LlmError::RateLimited {
        retry_after_secs: Some(42),
    };
    let bytes = serde_json::to_vec(&error).expect("serialize error");
    let restored: LlmError = serde_json::from_slice(&bytes).expect("deserialize error");
    assert_eq!(error, restored);
}
