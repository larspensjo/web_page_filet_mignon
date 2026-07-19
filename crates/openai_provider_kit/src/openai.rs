use std::{env, time::Duration};

use async_trait::async_trait;
use reqwest::{header, StatusCode};
use serde::{Deserialize, Serialize};

use crate::provider::LlmProvider;
use crate::types::{
    ChatRole, FinishReason, LlmError, LlmRequest, LlmResponse, ModelId, ProviderKind,
    ResponseFormat, TokenUsage,
};

const HTTP_TIMEOUT: Duration = Duration::from_secs(120);

#[derive(Clone)]
pub struct OpenAiProvider {
    pub(crate) client: reqwest::Client,
    pub(crate) api_key: String,
    pub(crate) base_url: String,
}

impl OpenAiProvider {
    pub fn from_env() -> Result<Self, LlmError> {
        let api_key = env::var("OPENAI_API_KEY").map_err(|err| LlmError::Configuration {
            detail: format!("OPENAI_API_KEY missing: {err}"),
        })?;
        Ok(Self::new(api_key))
    }

    pub fn new(api_key: String) -> Self {
        let client = reqwest::Client::builder()
            .timeout(HTTP_TIMEOUT)
            .build()
            .expect("failed to build OpenAI HTTP client");

        Self {
            client,
            api_key,
            base_url: "https://api.openai.com/v1".to_string(),
        }
    }

    pub fn with_base_url(mut self, base_url: String) -> Self {
        self.base_url = base_url;
        self
    }

    #[cfg(feature = "reqwest-passthrough")]
    pub fn with_client(mut self, client: reqwest::Client) -> Self {
        self.client = client;
        self
    }

    fn endpoint(&self) -> String {
        format!("{}/chat/completions", self.base_url.trim_end_matches('/'))
    }

    fn build_request_body(request: &LlmRequest) -> OpenAiChatCompletionRequest {
        OpenAiChatCompletionRequest::from_llm_request(request)
    }

    pub(crate) fn map_status_code(
        status: StatusCode,
        headers: &header::HeaderMap,
        body: String,
    ) -> LlmError {
        match status.as_u16() {
            401 => LlmError::AuthenticationFailed,
            429 => match Self::insufficient_quota_description(&body) {
                Some(description) => LlmError::QuotaExhausted { description },
                None => LlmError::RateLimited {
                    retry_after_secs: headers
                        .get(header::RETRY_AFTER)
                        .and_then(|value| value.to_str().ok())
                        .and_then(|value| value.parse::<u64>().ok()),
                },
            },
            _ => LlmError::Http {
                status: status.as_u16(),
                body,
            },
        }
    }

    /// OpenAI returns 429 for both rate limiting and an exhausted credit
    /// balance; only the body's `error.code`/`error.type` distinguishes them.
    fn insufficient_quota_description(body: &str) -> Option<String> {
        #[derive(Deserialize)]
        struct ErrorBody {
            error: ErrorDetail,
        }
        #[derive(Deserialize)]
        struct ErrorDetail {
            #[serde(default)]
            code: Option<String>,
            #[serde(default, rename = "type")]
            kind: Option<String>,
            #[serde(default)]
            message: Option<String>,
        }

        let parsed: ErrorBody = serde_json::from_str(body).ok()?;
        let is_quota = parsed.error.code.as_deref() == Some("insufficient_quota")
            || parsed.error.kind.as_deref() == Some("insufficient_quota");
        is_quota.then(|| {
            parsed
                .error
                .message
                .unwrap_or_else(|| "insufficient quota".to_string())
        })
    }

    fn parse_response_body(bytes: &[u8]) -> Result<LlmResponse, LlmError> {
        let parsed: OpenAiChatResponse =
            serde_json::from_slice(bytes).map_err(|err| LlmError::InvalidResponse {
                detail: format!("response parse failure: {err}"),
            })?;

        let choice = parsed
            .choices
            .first()
            .ok_or_else(|| LlmError::InvalidResponse {
                detail: "response missing choices".to_string(),
            })?;

        let finish_reason = Self::finish_reason(choice.finish_reason.as_deref());
        let content =
            choice
                .message
                .extract_text()
                .map_err(|detail| LlmError::InvalidResponse {
                    detail: append_finish_reason(detail, choice.finish_reason.as_deref()),
                })?;

        let cached = parsed
            .usage
            .prompt_tokens_details
            .as_ref()
            .and_then(|d| d.cached_tokens)
            .unwrap_or(0);
        let usage = TokenUsage::new(
            parsed.usage.prompt_tokens.unwrap_or(0),
            parsed.usage.completion_tokens.unwrap_or(0),
        )
        .with_cached_input_tokens(cached);
        let model_id = ModelId::new(ProviderKind::OpenAi, parsed.model);

        Ok(LlmResponse::new(content, usage, model_id, finish_reason))
    }

    fn finish_reason(reason: Option<&str>) -> FinishReason {
        match reason {
            Some("stop") => FinishReason::Stop,
            Some("length") => FinishReason::MaxTokens,
            Some("max_tokens") => FinishReason::MaxTokens,
            Some("content_filter") => FinishReason::ContentFilter,
            _ => FinishReason::Unknown,
        }
    }

    pub(crate) fn map_reqwest_error(err: reqwest::Error) -> LlmError {
        if err.is_timeout() {
            LlmError::Timeout
        } else {
            LlmError::Network {
                detail: err.to_string(),
            }
        }
    }
}

#[async_trait]
impl LlmProvider for OpenAiProvider {
    async fn complete(&self, request: &LlmRequest) -> Result<LlmResponse, LlmError> {
        let body = serde_json::to_vec(&openai_chat_completion_body(request)).map_err(|err| {
            LlmError::InvalidResponse {
                detail: format!("request serialization failed: {err}"),
            }
        })?;

        let response = self
            .client
            .post(self.endpoint())
            .header(header::AUTHORIZATION, format!("Bearer {}", self.api_key))
            .header(header::CONTENT_TYPE, "application/json")
            .body(body)
            .send()
            .await
            .map_err(Self::map_reqwest_error)?;

        if !response.status().is_success() {
            let status = response.status();
            let headers = response.headers().clone();
            let body_text = response
                .text()
                .await
                .unwrap_or_else(|_| "<body read failed>".to_string());
            return Err(Self::map_status_code(status, &headers, body_text));
        }

        let bytes = response.bytes().await.map_err(Self::map_reqwest_error)?;
        Self::parse_response_body(&bytes)
    }

    fn provider_name(&self) -> &str {
        "openai"
    }

    async fn list_models(&self) -> Result<Vec<String>, LlmError> {
        let url = format!("{}/models", self.base_url.trim_end_matches('/'));

        let response = self
            .client
            .get(&url)
            .header(header::AUTHORIZATION, format!("Bearer {}", self.api_key))
            .send()
            .await
            .map_err(Self::map_reqwest_error)?;

        if !response.status().is_success() {
            let status = response.status();
            let headers = response.headers().clone();
            let body_text = response
                .text()
                .await
                .unwrap_or_else(|_| "<body read failed>".to_string());
            return Err(Self::map_status_code(status, &headers, body_text));
        }

        let bytes = response.bytes().await.map_err(Self::map_reqwest_error)?;

        let parsed: OpenAiModelsResponse =
            serde_json::from_slice(&bytes).map_err(|err| LlmError::InvalidResponse {
                detail: format!("models response parse failure: {err}"),
            })?;

        // Filter to chat-completion models using prefix allow-list
        let chat_models: Vec<String> = parsed
            .data
            .into_iter()
            .map(|model| model.id)
            .filter(|id| {
                let id_lower = id.to_lowercase();
                id_lower.starts_with("gpt-")
                    || id_lower.starts_with("o1-")
                    || id_lower.starts_with("o3-")
                    || id_lower.starts_with("o4-")
            })
            .filter(|id| {
                let id_lower = id.to_lowercase();
                !id_lower.contains("whisper")
                    && !id_lower.contains("dall-e")
                    && !id_lower.contains("tts")
                    && !id_lower.contains("text-embedding")
                    && !id_lower.contains("audio")
                    && !id_lower.contains("realtime")
                    && !id_lower.contains("transcribe")
                    && !id_lower.contains("search")
                    && !id_lower.contains("instruct")
                    && !is_dated_snapshot(id)
            })
            .collect();

        Ok(chat_models)
    }
}

/// Builds the exact JSON object sent to OpenAI's Chat Completions endpoint.
///
/// Batch callers can embed this object in a JSONL request line, guaranteeing it
/// stays byte-for-byte equivalent to the synchronous request payload.
pub fn openai_chat_completion_body(request: &LlmRequest) -> serde_json::Value {
    serde_json::to_value(OpenAiProvider::build_request_body(request))
        .expect("OpenAI chat completion request must be serializable")
}

#[derive(Serialize)]
struct OpenAiChatCompletionRequest {
    model: String,
    messages: Vec<OpenAiChatMessage>,
    #[serde(skip_serializing_if = "Option::is_none")]
    temperature: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    max_tokens: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    max_completion_tokens: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    response_format: Option<OpenAiResponseFormat>,
}

impl OpenAiChatCompletionRequest {
    fn from_llm_request(request: &LlmRequest) -> Self {
        let max_output_tokens = request.max_output_tokens();
        let model_name = request.model().model_name().to_string();
        let use_completion_tokens = prefers_max_completion_tokens(&model_name);
        Self {
            model: model_name,
            messages: request
                .messages()
                .iter()
                .map(|message| OpenAiChatMessage {
                    role: chat_role_to_str(message.role()),
                    content: message.content().to_string(),
                })
                .collect(),
            temperature: request.temperature(),
            max_tokens: if use_completion_tokens {
                None
            } else {
                max_output_tokens
            },
            max_completion_tokens: if use_completion_tokens {
                max_output_tokens
            } else {
                None
            },
            response_format: OpenAiResponseFormat::from_response_format(request.response_format()),
        }
    }
}

fn prefers_max_completion_tokens(model_name: &str) -> bool {
    let lower = model_name.to_ascii_lowercase();
    lower.starts_with("gpt-5")
        || lower.starts_with("o1")
        || lower.starts_with("o3")
        || lower.starts_with("o4")
}

#[derive(Serialize)]
pub(crate) struct OpenAiChatMessage {
    role: &'static str,
    content: String,
}

fn chat_role_to_str(role: ChatRole) -> &'static str {
    match role {
        ChatRole::System => "system",
        ChatRole::User => "user",
        ChatRole::Assistant => "assistant",
    }
}

#[derive(Serialize)]
pub(crate) struct OpenAiResponseFormat {
    #[serde(rename = "type")]
    kind: &'static str,
}

impl OpenAiResponseFormat {
    fn from_response_format(format: ResponseFormat) -> Option<Self> {
        match format {
            ResponseFormat::Json => Some(Self {
                kind: "json_object",
            }),
            ResponseFormat::Text => None,
        }
    }
}

#[derive(Deserialize)]
pub(crate) struct OpenAiChatResponse {
    model: String,
    choices: Vec<OpenAiChoice>,
    #[serde(default)]
    usage: OpenAiUsage,
}

#[derive(Deserialize)]
pub(crate) struct OpenAiChoice {
    message: OpenAiMessage,
    #[serde(rename = "finish_reason")]
    finish_reason: Option<String>,
}

#[derive(Deserialize)]
pub(crate) struct OpenAiMessage {
    #[serde(default)]
    content: Option<OpenAiMessageContent>,
    #[serde(default)]
    refusal: Option<String>,
}

impl OpenAiMessage {
    fn extract_text(&self) -> Result<String, String> {
        let mut text_parts = Vec::new();
        let mut refusals = Vec::new();

        match &self.content {
            Some(OpenAiMessageContent::Text(text)) if !text.is_empty() => {
                text_parts.push(text.clone());
            }
            Some(OpenAiMessageContent::Text(_)) => {}
            Some(OpenAiMessageContent::Parts(parts)) => {
                for part in parts {
                    match part.kind.as_str() {
                        "text" => {
                            if let Some(text) = part.text.as_ref().filter(|text| !text.is_empty()) {
                                text_parts.push(text.clone());
                            }
                        }
                        "refusal" => {
                            if let Some(refusal) =
                                part.refusal.as_ref().filter(|text| !text.is_empty())
                            {
                                refusals.push(refusal.clone());
                            }
                        }
                        _ => {}
                    }
                }
            }
            None => {}
        }

        let joined = text_parts.join("");
        if !joined.trim().is_empty() {
            return Ok(joined);
        }

        if let Some(refusal) = self
            .refusal
            .as_ref()
            .filter(|text| !text.trim().is_empty())
            .cloned()
            .or_else(|| refusals.into_iter().find(|text| !text.trim().is_empty()))
        {
            return Err(format!("assistant refusal: {refusal}"));
        }

        Err("choice missing content".to_string())
    }
}

#[derive(Deserialize)]
#[serde(untagged)]
pub(crate) enum OpenAiMessageContent {
    Text(String),
    Parts(Vec<OpenAiContentPart>),
}

#[derive(Deserialize)]
pub(crate) struct OpenAiContentPart {
    #[serde(rename = "type")]
    kind: String,
    #[serde(default)]
    text: Option<String>,
    #[serde(default)]
    refusal: Option<String>,
}

#[derive(Deserialize, Default)]
pub(crate) struct OpenAiPromptTokensDetails {
    #[serde(default)]
    cached_tokens: Option<u32>,
}

#[derive(Deserialize, Default)]
pub(crate) struct OpenAiUsage {
    #[serde(rename = "prompt_tokens")]
    prompt_tokens: Option<u32>,
    #[serde(rename = "completion_tokens")]
    completion_tokens: Option<u32>,
    #[serde(default)]
    prompt_tokens_details: Option<OpenAiPromptTokensDetails>,
}

#[derive(Deserialize)]
pub(crate) struct OpenAiModelsResponse {
    data: Vec<OpenAiModel>,
}

#[derive(Deserialize)]
pub(crate) struct OpenAiModel {
    id: String,
}

/// Detects dated snapshot model IDs like gpt-4-0613 or gpt-4-turbo-2024-08-06.
/// These are typically older frozen versions that should be filtered out
/// in favor of the latest rolling versions.
fn is_dated_snapshot(id: &str) -> bool {
    // Pattern: ends with -MMDD or -YYYY-MM-DD
    let parts: Vec<&str> = id.split('-').collect();
    if parts.len() < 2 {
        return false;
    }

    let last = parts[parts.len() - 1];

    // Check for MMDD format (e.g., gpt-4-0613)
    if last.len() == 4 && last.chars().all(|c| c.is_ascii_digit()) {
        return true;
    }

    // Check for YYYY-MM-DD format (e.g., gpt-4-turbo-2024-08-06)
    if parts.len() >= 3 {
        let year = parts[parts.len() - 3];
        let month = parts[parts.len() - 2];
        let day = last;

        if year.len() == 4
            && year.chars().all(|c| c.is_ascii_digit())
            && month.len() == 2
            && month.chars().all(|c| c.is_ascii_digit())
            && day.len() == 2
            && day.chars().all(|c| c.is_ascii_digit())
        {
            return true;
        }
    }

    false
}

fn append_finish_reason(detail: String, finish_reason: Option<&str>) -> String {
    match finish_reason {
        Some(reason) if !reason.is_empty() => format!("{detail} (finish_reason={reason})"),
        _ => detail,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::ChatMessage;
    use reqwest::header::{self, HeaderMap, HeaderValue};
    use wiremock::matchers::{method, path};
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
    fn response_parsing_handles_content_part_arrays() {
        let payload = serde_json::json!({
            "model": "gpt-5.4-nano",
            "choices": [
                {
                    "message": {
                        "content": [
                            { "type": "text", "text": "{\"synthesis\":\"" },
                            { "type": "text", "text": "hello\"}" }
                        ]
                    },
                    "finish_reason": "stop"
                }
            ],
            "usage": { "prompt_tokens": 8, "completion_tokens": 4 }
        });

        let bytes = serde_json::to_vec(&payload).unwrap();
        let response = OpenAiProvider::parse_response_body(&bytes).unwrap();

        assert_eq!(response.content(), "{\"synthesis\":\"hello\"}");
        assert_eq!(response.finish_reason(), FinishReason::Stop);
    }

    #[test]
    fn response_parsing_surfaces_refusal_messages() {
        let payload = serde_json::json!({
            "model": "gpt-5.4-nano",
            "choices": [
                {
                    "message": {
                        "content": null,
                        "refusal": "I can't help with that request."
                    },
                    "finish_reason": "stop"
                }
            ],
            "usage": { "prompt_tokens": 8, "completion_tokens": 0 }
        });

        let bytes = serde_json::to_vec(&payload).unwrap();
        let err = OpenAiProvider::parse_response_body(&bytes).unwrap_err();

        assert!(matches!(
            err,
            LlmError::InvalidResponse { detail }
                if detail.contains("assistant refusal")
                    && detail.contains("I can't help with that request.")
                    && detail.contains("finish_reason=stop")
        ));
    }

    #[test]
    fn response_parsing_marks_empty_length_responses_as_max_tokens() {
        let payload = serde_json::json!({
            "model": "gpt-5.4-nano",
            "choices": [
                {
                    "message": { "content": "" },
                    "finish_reason": "length"
                }
            ],
            "usage": { "prompt_tokens": 8, "completion_tokens": 220 }
        });

        let bytes = serde_json::to_vec(&payload).unwrap();
        let err = OpenAiProvider::parse_response_body(&bytes).unwrap_err();

        assert!(matches!(
            err,
            LlmError::InvalidResponse { detail }
                if detail.contains("choice missing content")
                    && detail.contains("finish_reason=length")
        ));
    }

    #[test]
    fn maps_401_status_to_authentication_failed() {
        let err = OpenAiProvider::map_status_code(
            StatusCode::UNAUTHORIZED,
            &HeaderMap::new(),
            "body".into(),
        );
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
    fn maps_429_insufficient_quota_body_to_quota_exhausted() {
        let body = r#"{"error":{"message":"You exceeded your current quota, please check your plan and billing details.","type":"insufficient_quota","param":null,"code":"insufficient_quota"}}"#;
        let err = OpenAiProvider::map_status_code(
            StatusCode::TOO_MANY_REQUESTS,
            &HeaderMap::new(),
            body.into(),
        );
        match err {
            LlmError::QuotaExhausted { description } => {
                assert!(description.contains("exceeded your current quota"));
            }
            other => panic!("expected QuotaExhausted, got {other:?}"),
        }
    }

    #[test]
    fn maps_429_rate_limit_body_to_rate_limited() {
        let body = r#"{"error":{"message":"Rate limit reached for gpt-5.4-mini","type":"requests","param":null,"code":"rate_limit_exceeded"}}"#;
        let err = OpenAiProvider::map_status_code(
            StatusCode::TOO_MANY_REQUESTS,
            &HeaderMap::new(),
            body.into(),
        );
        assert!(matches!(
            err,
            LlmError::RateLimited {
                retry_after_secs: None
            }
        ));
    }

    #[test]
    fn maps_429_unparseable_body_to_rate_limited() {
        let err = OpenAiProvider::map_status_code(
            StatusCode::TOO_MANY_REQUESTS,
            &HeaderMap::new(),
            "<html>not json</html>".into(),
        );
        assert!(matches!(
            err,
            LlmError::RateLimited {
                retry_after_secs: None
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

        let err = client
            .post(format!("{}/v1/chat/completions", server.uri()))
            .send()
            .await
            .unwrap_err();
        let err = OpenAiProvider::map_reqwest_error(err);
        assert!(matches!(err, LlmError::Timeout));
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

        assert_eq!(response.usage().cached_input_tokens, 100);
    }

    #[test]
    fn length_finish_reason_maps_to_max_tokens() {
        let payload = serde_json::json!({
            "model": "gpt-5.4-nano",
            "choices": [
                {
                    "message": { "content": "ok" },
                    "finish_reason": "length"
                }
            ],
            "usage": { "prompt_tokens": 2, "completion_tokens": 1 }
        });

        let bytes = serde_json::to_vec(&payload).unwrap();
        let response = OpenAiProvider::parse_response_body(&bytes).unwrap();

        assert_eq!(response.finish_reason(), FinishReason::MaxTokens);
    }

    #[test]
    fn dated_snapshot_detects_mmdd_format() {
        assert!(is_dated_snapshot("chat-family-0613"));
        assert!(is_dated_snapshot("rolling-model-1106"));
        assert!(is_dated_snapshot("preview-series-0912"));
    }

    #[test]
    fn dated_snapshot_detects_yyyy_mm_dd_format() {
        assert!(is_dated_snapshot("chat-family-2024-08-06"));
        assert!(is_dated_snapshot("rolling-model-2024-01-25"));
        assert!(is_dated_snapshot("reasoner-series-2025-02-14"));
    }

    #[test]
    fn dated_snapshot_rejects_current_rolling_versions() {
        assert!(!is_dated_snapshot("chat-family"));
        assert!(!is_dated_snapshot("chat-family-preview"));
        assert!(!is_dated_snapshot("rolling-model"));
        assert!(!is_dated_snapshot("reasoner-preview"));
        assert!(!is_dated_snapshot("reasoner-mini"));
    }

    #[test]
    fn dated_snapshot_rejects_non_date_suffixes() {
        assert!(!is_dated_snapshot("chat-family-vision"));
        assert!(!is_dated_snapshot("chat-family-preview"));
        assert!(!is_dated_snapshot("embedding-family"));
    }

    #[test]
    fn dated_snapshot_handles_edge_cases() {
        assert!(!is_dated_snapshot(""));
        assert!(!is_dated_snapshot("gpt"));
        assert!(!is_dated_snapshot("123"));
    }
}
