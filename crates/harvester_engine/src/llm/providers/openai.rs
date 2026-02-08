use std::{env, time::Duration};

use async_trait::async_trait;
use reqwest::{header, StatusCode};
use serde::{Deserialize, Serialize};

use crate::llm::provider::LlmProvider;
use crate::llm::types::{
    ChatRole, FinishReason, LlmError, LlmRequest, LlmResponse, ModelId, ProviderKind,
    ResponseFormat, TokenUsage,
};

#[derive(Clone)]
pub struct OpenAiProvider {
    client: reqwest::Client,
    api_key: String,
    base_url: String,
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
            .timeout(Duration::from_secs(60))
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

    pub fn with_client(mut self, client: reqwest::Client) -> Self {
        self.client = client;
        self
    }

    fn endpoint(&self) -> String {
        format!("{}/chat/completions", self.base_url.trim_end_matches('/'))
    }

    pub fn build_request_body(request: &LlmRequest) -> OpenAiChatCompletionRequest {
        OpenAiChatCompletionRequest::from_llm_request(request)
    }

    pub fn map_status_code(
        status: StatusCode,
        headers: &header::HeaderMap,
        body: String,
    ) -> LlmError {
        match status.as_u16() {
            401 => LlmError::AuthenticationFailed,
            429 => LlmError::RateLimited {
                retry_after_secs: headers
                    .get(header::RETRY_AFTER)
                    .and_then(|value| value.to_str().ok())
                    .and_then(|value| value.parse::<u64>().ok()),
            },
            _ => LlmError::Http {
                status: status.as_u16(),
                body,
            },
        }
    }

    pub fn parse_response_body(bytes: &[u8]) -> Result<LlmResponse, LlmError> {
        let parsed: OpenAiChatResponse =
            serde_json::from_slice(bytes).map_err(|err| LlmError::InvalidResponse {
                detail: format!("response parse failure: {err}"),
            })?;

        let choice = parsed
            .choices
            .get(0)
            .ok_or_else(|| LlmError::InvalidResponse {
                detail: "response missing choices".to_string(),
            })?;

        let content = choice.message.content.clone();
        if content.is_empty() {
            return Err(LlmError::InvalidResponse {
                detail: "choice missing content".to_string(),
            });
        }

        let usage = TokenUsage::new(
            parsed.usage.prompt_tokens.unwrap_or(0),
            parsed.usage.completion_tokens.unwrap_or(0),
        );
        let model_id = ModelId::new(ProviderKind::OpenAi, parsed.model);

        Ok(LlmResponse::new(
            content,
            usage,
            model_id,
            Self::finish_reason(choice.finish_reason.as_deref()),
        ))
    }

    fn finish_reason(reason: Option<&str>) -> FinishReason {
        match reason {
            Some("stop") => FinishReason::Stop,
            Some("max_tokens") => FinishReason::MaxTokens,
            Some("content_filter") => FinishReason::ContentFilter,
            _ => FinishReason::Unknown,
        }
    }

    pub fn map_reqwest_error(err: reqwest::Error) -> LlmError {
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
        let payload = Self::build_request_body(request);
        let body = serde_json::to_vec(&payload).map_err(|err| LlmError::InvalidResponse {
            detail: format!("request serialization failed: {err}"),
        })?;

        let response = self
            .client
            .post(&self.endpoint())
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
}

#[derive(Serialize)]
pub struct OpenAiChatCompletionRequest {
    model: String,
    messages: Vec<OpenAiChatMessage>,
    #[serde(skip_serializing_if = "Option::is_none")]
    temperature: Option<f32>,
    #[serde(rename = "max_tokens", skip_serializing_if = "Option::is_none")]
    max_tokens: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    response_format: Option<OpenAiResponseFormat>,
}

impl OpenAiChatCompletionRequest {
    fn from_llm_request(request: &LlmRequest) -> Self {
        Self {
            model: request.model().model_name().to_string(),
            messages: request
                .messages()
                .iter()
                .map(|message| OpenAiChatMessage {
                    role: chat_role_to_str(message.role()),
                    content: message.content().to_string(),
                })
                .collect(),
            temperature: request.temperature(),
            max_tokens: request.max_output_tokens(),
            response_format: OpenAiResponseFormat::from_response_format(request.response_format()),
        }
    }
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
    content: String,
}

#[derive(Deserialize, Default)]
pub(crate) struct OpenAiUsage {
    #[serde(rename = "prompt_tokens")]
    prompt_tokens: Option<u32>,
    #[serde(rename = "completion_tokens")]
    completion_tokens: Option<u32>,
}
