use std::{env, time::Duration};

use harvester_engine::llm::{
    ChatMessage, ChatRole, FinishReason, LlmError, LlmProvider, LlmRequest, ModelId,
    OpenAiProvider, ProviderKind,
};
use reqwest::header::{self, HeaderMap, HeaderValue};
use reqwest::StatusCode;
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
fn request_serialization_matches_openai_api_format() {
    let request = sample_request()
        .with_temperature(0.2)
        .with_max_output_tokens(128)
        .with_json_response();

    let body = OpenAiProvider::build_request_body(&request);
    let json = serde_json::to_value(&body).unwrap();
    assert_eq!(json["model"], "gpt-4.1");
    assert_eq!(json["messages"][0]["role"], "system");
    assert_eq!(json["messages"][0]["content"], "system prompt");
    assert_eq!(json["messages"][1]["role"], "user");
    assert_eq!(json["messages"][1]["content"], "user prompt");
    assert_eq!(json["max_tokens"], 128);
    assert!(json["max_completion_tokens"].is_null());
    assert_eq!(json["response_format"]["type"], "json_object");
    let temperature = json["temperature"].as_f64().unwrap();
    assert!((temperature - 0.2).abs() < 1e-6);
}

#[test]
fn request_serialization_uses_max_completion_tokens_for_gpt5_models() {
    let request = LlmRequest::new(
        ModelId::new(ProviderKind::OpenAi, "gpt-5.4-nano"),
        vec![ChatMessage::new(ChatRole::User, "hello")],
    )
    .with_max_output_tokens(64);

    let body = OpenAiProvider::build_request_body(&request);
    let json = serde_json::to_value(&body).unwrap();

    assert_eq!(json["model"], "gpt-5.4-nano");
    assert_eq!(json["max_completion_tokens"], 64);
    assert!(json["max_tokens"].is_null());
}

#[test]
fn response_parsing_handles_all_fields_correctly() {
    let payload = serde_json::json!({
        "model": "gpt-4.1",
        "choices": [
            {
                "message": { "content": "hello" },
                "finish_reason": "stop"
            }
        ],
        "usage": { "prompt_tokens": 5, "completion_tokens": 3 }
    });

    let bytes = serde_json::to_vec(&payload).unwrap();
    let response = OpenAiProvider::parse_response_body(&bytes).unwrap();

    assert_eq!(response.content(), "hello");
    assert_eq!(response.finish_reason(), FinishReason::Stop);
    assert_eq!(response.usage().input_tokens, 5);
    assert_eq!(response.usage().output_tokens, 3);
    assert_eq!(response.model_id().model_name(), "gpt-4.1");
}

#[test]
fn maps_401_status_to_authentication_failed() {
    let err =
        OpenAiProvider::map_status_code(StatusCode::UNAUTHORIZED, &HeaderMap::new(), "body".into());
    assert!(matches!(err, LlmError::AuthenticationFailed));
}

#[test]
fn maps_429_status_to_rate_limited_with_retry_after() {
    let mut headers = HeaderMap::new();
    headers.insert(header::RETRY_AFTER, HeaderValue::from_static("7"));

    let err =
        OpenAiProvider::map_status_code(StatusCode::TOO_MANY_REQUESTS, &headers, "body".into());
    assert!(matches!(
        err,
        LlmError::RateLimited {
            retry_after_secs: Some(7)
        }
    ));
}

#[test]
fn maps_500_status_to_http_error() {
    let err = OpenAiProvider::map_status_code(
        StatusCode::INTERNAL_SERVER_ERROR,
        &HeaderMap::new(),
        "oops".into(),
    );
    assert!(matches!(err, LlmError::Http { status: 500, body } if body == "oops"));
}

#[test]
fn malformed_response_body_produces_invalid_response() {
    let err = OpenAiProvider::parse_response_body(b"not json").unwrap_err();
    assert!(
        matches!(err, LlmError::InvalidResponse { detail } if detail.contains("response parse failure"))
    );
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
async fn network_errors_map_to_network_variant() {
    let client = reqwest::Client::new();
    let err = client.get("http://127.0.0.1:1").send().await.unwrap_err();

    let mapped = OpenAiProvider::map_reqwest_error(err);
    assert!(matches!(mapped, LlmError::Network { .. }));
}

#[tokio::test]
async fn timeout_errors_translate_to_llm_timeout() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/chat/completions"))
        .respond_with(ResponseTemplate::new(200).set_delay(Duration::from_millis(200)))
        .mount(&server)
        .await;

    let client = reqwest::Client::builder()
        .timeout(Duration::from_millis(20))
        .build()
        .unwrap();

    let provider = OpenAiProvider::new("key".into())
        .with_client(client)
        .with_base_url(format!("{}/v1", server.uri()));

    let err = provider.complete(&sample_request()).await.unwrap_err();
    assert!(matches!(err, LlmError::Timeout));
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
    assert_eq!(response.usage().input_tokens, 2);
    assert_eq!(response.usage().output_tokens, 1);
}

#[test]
fn parse_response_with_cached_tokens() {
    let payload = serde_json::json!({
        "model": "gpt-4.1",
        "choices": [
            {
                "message": { "content": "hello" },
                "finish_reason": "stop"
            }
        ],
        "usage": {
            "prompt_tokens": 1000,
            "completion_tokens": 50,
            "prompt_tokens_details": { "cached_tokens": 800 }
        }
    });

    let bytes = serde_json::to_vec(&payload).unwrap();
    let response = OpenAiProvider::parse_response_body(&bytes).unwrap();

    assert_eq!(response.usage().cached_input_tokens, 800);
}

#[test]
fn parse_response_without_cached_tokens() {
    let payload = serde_json::json!({
        "model": "gpt-4.1",
        "choices": [
            {
                "message": { "content": "hello" },
                "finish_reason": "stop"
            }
        ],
        "usage": { "prompt_tokens": 10, "completion_tokens": 5 }
    });

    let bytes = serde_json::to_vec(&payload).unwrap();
    let response = OpenAiProvider::parse_response_body(&bytes).unwrap();

    assert_eq!(response.usage().cached_input_tokens, 0);
}

#[test]
fn parse_response_with_oversized_cached_tokens() {
    let payload = serde_json::json!({
        "model": "gpt-4.1",
        "choices": [
            {
                "message": { "content": "hello" },
                "finish_reason": "stop"
            }
        ],
        "usage": {
            "prompt_tokens": 100,
            "completion_tokens": 20,
            "prompt_tokens_details": { "cached_tokens": 9999 }
        }
    });

    let bytes = serde_json::to_vec(&payload).unwrap();
    let response = OpenAiProvider::parse_response_body(&bytes).unwrap();

    // Clamping is done inside TokenUsage::with_cached_input_tokens, not in the parser;
    // cached_input_tokens must not exceed prompt_tokens (100)
    assert_eq!(response.usage().cached_input_tokens, 100);
}

#[tokio::test]
#[ignore]
async fn live_openai_completion() {
    // This test is ignored by default since it requires a real API key and makes a network request.
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
