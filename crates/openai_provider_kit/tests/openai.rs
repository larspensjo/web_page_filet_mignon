use std::env;

use openai_provider_kit::{
    ChatMessage, ChatRole, FinishReason, LlmError, LlmProvider, LlmRequest, ModelId,
    OpenAiProvider, ProviderKind,
};
use wiremock::matchers::{header as header_matcher, method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

fn sample_request() -> LlmRequest {
    LlmRequest::new(
        ModelId::new(ProviderKind::OpenAi, "gpt-4.1"),
        vec![
            ChatMessage::new(ChatRole::System, "system prompt"),
            ChatMessage::new(ChatRole::User, "user prompt"),
        ],
    )
}

#[test]
fn from_env_missing_key_returns_configuration_error() {
    let original = env::var("OPENAI_API_KEY").ok();
    env::remove_var("OPENAI_API_KEY");

    match OpenAiProvider::from_env() {
        Err(LlmError::Configuration { detail }) if detail.contains("OPENAI_API_KEY missing") => {}
        Err(other) => panic!("unexpected error variant: {other:?}"),
        Ok(_) => panic!("expected configuration error"),
    }

    if let Some(value) = original {
        env::set_var("OPENAI_API_KEY", value);
    }
}

#[tokio::test]
async fn integration_round_trip_with_wiremock() {
    let server = MockServer::start().await;

    let response_body = serde_json::json!({
        "model": "gpt-4.1",
        "choices": [
            {
                "message": { "content": "ok" },
                "finish_reason": "stop"
            }
        ],
        "usage": { "prompt_tokens": 2, "completion_tokens": 1 }
    });

    Mock::given(method("POST"))
        .and(path("/v1/chat/completions"))
        .and(header_matcher("authorization", "Bearer test-key"))
        .respond_with(ResponseTemplate::new(200).set_body_json(&response_body))
        .mount(&server)
        .await;

    let provider =
        OpenAiProvider::new("test-key".into()).with_base_url(format!("{}/v1", server.uri()));
    let response = provider.complete(&sample_request()).await.unwrap();

    assert_eq!(response.content(), "ok");
    assert_eq!(response.finish_reason(), FinishReason::Stop);
    assert_eq!(response.usage().input_tokens, 2);
    assert_eq!(response.usage().output_tokens, 1);
}

#[tokio::test]
async fn completion_status_errors_map_to_public_error_variants() {
    let cases = [
        (401, ResponseTemplate::new(401), "auth"),
        (
            429,
            ResponseTemplate::new(429).insert_header("Retry-After", "7"),
            "rate",
        ),
        (
            500,
            ResponseTemplate::new(500).set_body_string("oops"),
            "http",
        ),
    ];

    for (status, template, expected) in cases {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/v1/chat/completions"))
            .respond_with(template)
            .mount(&server)
            .await;

        let provider =
            OpenAiProvider::new("test-key".into()).with_base_url(format!("{}/v1", server.uri()));
        let err = provider.complete(&sample_request()).await.unwrap_err();
        match (status, expected, err) {
            (401, "auth", LlmError::AuthenticationFailed) => {}
            (
                429,
                "rate",
                LlmError::RateLimited {
                    retry_after_secs: Some(7),
                },
            ) => {}
            (500, "http", LlmError::Http { status: 500, body }) if body == "oops" => {}
            (_, _, other) => panic!("unexpected error for HTTP {status}: {other:?}"),
        }
    }
}

#[tokio::test]
async fn network_error_from_completion_maps_to_network_variant() {
    let provider =
        OpenAiProvider::new("test-key".into()).with_base_url("http://127.0.0.1:1/v1".to_string());

    let err = provider.complete(&sample_request()).await.unwrap_err();
    assert!(matches!(err, LlmError::Network { .. }));
}

#[tokio::test]
#[ignore]
async fn live_openai_completion() {
    let _key =
        std::env::var("OPENAI_API_KEY").expect("set OPENAI_API_KEY to run live OpenAI check");
    let provider =
        OpenAiProvider::from_env().expect("provider should initialize when key is present");
    let response = provider
        .complete(&sample_request().with_max_output_tokens(16))
        .await
        .expect("live completion must succeed");

    assert!(!response.content().trim().is_empty());
    assert!(matches!(response.finish_reason(), FinishReason::Stop));
}

#[tokio::test]
async fn list_models_filters_supported_rolling_chat_categories() {
    let server = MockServer::start().await;

    let models_response = serde_json::json!({
        "data": [
            { "id": "gpt-synthetic-chat" },
            { "id": "o1-synthetic-preview" },
            { "id": "o3-synthetic-mini" },
            { "id": "o4-synthetic-general" },
            { "id": "gpt-synthetic-chat-0613" },
            { "id": "gpt-synthetic-chat-2024-08-06" },
            { "id": "whisper-synthetic" },
            { "id": "gpt-synthetic-audio-preview" },
            { "id": "gpt-synthetic-realtime-preview" },
            { "id": "dall-e-synthetic" },
            { "id": "tts-synthetic" },
            { "id": "text-embedding-synthetic" },
            { "id": "gpt-synthetic-transcribe" },
            { "id": "gpt-synthetic-search-preview" },
            { "id": "gpt-synthetic-instruct" },
            { "id": "text-davinci-synthetic" }
        ]
    });

    Mock::given(method("GET"))
        .and(path("/v1/models"))
        .and(header_matcher("authorization", "Bearer test-key"))
        .respond_with(ResponseTemplate::new(200).set_body_json(&models_response))
        .mount(&server)
        .await;

    let provider =
        OpenAiProvider::new("test-key".into()).with_base_url(format!("{}/v1", server.uri()));
    let models = provider.list_models().await.unwrap();

    assert_eq!(
        models,
        vec![
            "gpt-synthetic-chat".to_string(),
            "o1-synthetic-preview".to_string(),
            "o3-synthetic-mini".to_string(),
            "o4-synthetic-general".to_string(),
        ]
    );
}
