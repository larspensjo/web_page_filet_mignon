use async_trait::async_trait;
use reqwest::header;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::{LlmError, OpenAiProvider};

pub type FileId = String;
pub type BatchId = String;

/// A single JSONL request accepted by OpenAI's Batch API.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct BatchInputLine {
    pub custom_id: String,
    pub method: String,
    pub url: String,
    pub body: Value,
}

impl BatchInputLine {
    pub fn to_jsonl(lines: &[Self]) -> Result<Vec<u8>, LlmError> {
        let mut encoded = Vec::new();
        for line in lines {
            serde_json::to_writer(&mut encoded, line).map_err(|err| LlmError::InvalidResponse {
                detail: format!("batch input serialization failed: {err}"),
            })?;
            encoded.push(b'\n');
        }
        Ok(encoded)
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BatchLifecycle {
    Validating,
    InProgress,
    Finalizing,
    Completed,
    Failed,
    Expired,
    Cancelling,
    Cancelled,
}

impl BatchLifecycle {
    fn parse(value: &str) -> Result<Self, LlmError> {
        match value {
            "validating" => Ok(Self::Validating),
            "in_progress" => Ok(Self::InProgress),
            "finalizing" => Ok(Self::Finalizing),
            "completed" => Ok(Self::Completed),
            "failed" => Ok(Self::Failed),
            "expired" => Ok(Self::Expired),
            "cancelling" => Ok(Self::Cancelling),
            "cancelled" => Ok(Self::Cancelled),
            other => Err(LlmError::InvalidResponse {
                detail: format!("unknown OpenAI batch lifecycle: {other}"),
            }),
        }
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct BatchRequestCounts {
    pub total: u32,
    pub completed: u32,
    pub failed: u32,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct BatchHandle {
    pub id: BatchId,
    pub status: BatchLifecycle,
    pub input_file_id: FileId,
    pub output_file_id: Option<FileId>,
    pub error_file_id: Option<FileId>,
    pub request_counts: BatchRequestCounts,
}

impl BatchHandle {
    pub fn from_openai_json(value: Value) -> Result<Self, LlmError> {
        let raw: OpenAiBatchHandle =
            serde_json::from_value(value).map_err(|err| LlmError::InvalidResponse {
                detail: format!("batch response parse failure: {err}"),
            })?;
        Ok(Self {
            id: raw.id,
            status: BatchLifecycle::parse(&raw.status)?,
            input_file_id: raw.input_file_id,
            output_file_id: raw.output_file_id,
            error_file_id: raw.error_file_id,
            request_counts: raw.request_counts.unwrap_or_default(),
        })
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct BatchOutputResponse {
    pub status_code: u16,
    pub body: Value,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct BatchOutputLine {
    pub custom_id: String,
    pub response: Option<BatchOutputResponse>,
    pub error: Option<Value>,
}

pub fn parse_batch_output_jsonl(bytes: &[u8]) -> Result<Vec<BatchOutputLine>, LlmError> {
    bytes
        .split(|byte| *byte == b'\n')
        .filter(|line| !line.is_empty())
        .map(|line| {
            serde_json::from_slice(line).map_err(|err| LlmError::InvalidResponse {
                detail: format!("batch output JSONL parse failure: {err}"),
            })
        })
        .collect()
}

/// Generic transport seam for OpenAI Batch API operations.
#[async_trait]
pub trait BatchTransport: Send + Sync {
    async fn upload_input(&self, jsonl: &[u8]) -> Result<FileId, LlmError>;
    async fn create_batch(
        &self,
        input_file_id: &str,
        endpoint: &str,
        completion_window: &str,
    ) -> Result<BatchHandle, LlmError>;
    async fn retrieve_batch(&self, batch_id: &str) -> Result<BatchHandle, LlmError>;
    async fn list_batches(&self, after: Option<&str>) -> Result<Vec<BatchHandle>, LlmError>;
    async fn download_file(&self, file_id: &str) -> Result<Vec<u8>, LlmError>;
    async fn cancel_batch(&self, batch_id: &str) -> Result<BatchHandle, LlmError>;
}

#[async_trait]
impl BatchTransport for OpenAiProvider {
    async fn upload_input(&self, jsonl: &[u8]) -> Result<FileId, LlmError> {
        let url = format!("{}/files", self.base_url.trim_end_matches('/'));
        let form = reqwest::multipart::Form::new()
            .text("purpose", "batch")
            .part(
                "file",
                reqwest::multipart::Part::bytes(jsonl.to_vec()).file_name("batch.jsonl"),
            );
        let response = self
            .client
            .post(url)
            .header(header::AUTHORIZATION, format!("Bearer {}", self.api_key))
            .multipart(form)
            .send()
            .await
            .map_err(Self::map_reqwest_error)?;
        let value = Self::batch_response_value(response).await?;
        value
            .get("id")
            .and_then(Value::as_str)
            .map(ToString::to_string)
            .ok_or_else(|| LlmError::InvalidResponse {
                detail: "file upload response missing id".to_string(),
            })
    }

    async fn create_batch(
        &self,
        input_file_id: &str,
        endpoint: &str,
        completion_window: &str,
    ) -> Result<BatchHandle, LlmError> {
        self.batch_request(reqwest::Method::POST, "/batches", Some(serde_json::json!({
            "input_file_id": input_file_id, "endpoint": endpoint, "completion_window": completion_window,
        }))).await
    }

    async fn retrieve_batch(&self, batch_id: &str) -> Result<BatchHandle, LlmError> {
        self.batch_request(reqwest::Method::GET, &format!("/batches/{batch_id}"), None)
            .await
    }

    async fn list_batches(&self, after: Option<&str>) -> Result<Vec<BatchHandle>, LlmError> {
        let mut url = format!("{}/batches?limit=100", self.base_url.trim_end_matches('/'));
        if let Some(after) = after {
            url.push_str(&format!("&after={after}"));
        }
        let response = self
            .client
            .get(url)
            .header(header::AUTHORIZATION, format!("Bearer {}", self.api_key))
            .send()
            .await
            .map_err(Self::map_reqwest_error)?;
        let value = Self::batch_response_value(response).await?;
        let records = value.get("data").and_then(Value::as_array).ok_or_else(|| {
            LlmError::InvalidResponse {
                detail: "batch list response missing data".to_string(),
            }
        })?;
        records
            .iter()
            .cloned()
            .map(BatchHandle::from_openai_json)
            .collect()
    }

    async fn download_file(&self, file_id: &str) -> Result<Vec<u8>, LlmError> {
        let url = format!(
            "{}/files/{file_id}/content",
            self.base_url.trim_end_matches('/')
        );
        let response = self
            .client
            .get(url)
            .header(header::AUTHORIZATION, format!("Bearer {}", self.api_key))
            .send()
            .await
            .map_err(Self::map_reqwest_error)?;
        if !response.status().is_success() {
            let status = response.status();
            let headers = response.headers().clone();
            let body = response
                .text()
                .await
                .unwrap_or_else(|_| "<body read failed>".to_string());
            return Err(Self::map_status_code(status, &headers, body));
        }
        response
            .bytes()
            .await
            .map(|bytes| bytes.to_vec())
            .map_err(Self::map_reqwest_error)
    }

    async fn cancel_batch(&self, batch_id: &str) -> Result<BatchHandle, LlmError> {
        self.batch_request(
            reqwest::Method::POST,
            &format!("/batches/{batch_id}/cancel"),
            None,
        )
        .await
    }
}

impl OpenAiProvider {
    async fn batch_request(
        &self,
        method: reqwest::Method,
        path: &str,
        body: Option<Value>,
    ) -> Result<BatchHandle, LlmError> {
        let url = format!("{}{}", self.base_url.trim_end_matches('/'), path);
        let mut request = self
            .client
            .request(method, url)
            .header(header::AUTHORIZATION, format!("Bearer {}", self.api_key));
        if let Some(body) = body {
            request = request.json(&body);
        }
        let response = request.send().await.map_err(Self::map_reqwest_error)?;
        BatchHandle::from_openai_json(Self::batch_response_value(response).await?)
    }

    async fn batch_response_value(response: reqwest::Response) -> Result<Value, LlmError> {
        if !response.status().is_success() {
            let status = response.status();
            let headers = response.headers().clone();
            let body = response
                .text()
                .await
                .unwrap_or_else(|_| "<body read failed>".to_string());
            return Err(Self::map_status_code(status, &headers, body));
        }
        response.json().await.map_err(Self::map_reqwest_error)
    }
}

#[derive(Deserialize)]
struct OpenAiBatchHandle {
    id: String,
    status: String,
    input_file_id: String,
    #[serde(default)]
    output_file_id: Option<String>,
    #[serde(default)]
    error_file_id: Option<String>,
    #[serde(default)]
    request_counts: Option<BatchRequestCounts>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn jsonl_round_trip() {
        let lines = vec![BatchInputLine {
            custom_id: "one".into(),
            method: "POST".into(),
            url: "/v1/chat/completions".into(),
            body: serde_json::json!({"model":"gpt"}),
        }];
        let encoded = BatchInputLine::to_jsonl(&lines).unwrap();
        let parsed: Vec<BatchInputLine> = encoded
            .split(|byte| *byte == b'\n')
            .filter(|line| !line.is_empty())
            .map(serde_json::from_slice)
            .collect::<Result<_, _>>()
            .unwrap();
        assert_eq!(parsed, lines);
    }

    #[test]
    fn completed_batch_parses() {
        let batch = BatchHandle::from_openai_json(serde_json::json!({"id":"batch_1","status":"completed","input_file_id":"file_1","output_file_id":"file_out","error_file_id":null,"request_counts":{"total":2,"completed":1,"failed":1}})).unwrap();
        assert_eq!(batch.status, BatchLifecycle::Completed);
        assert_eq!(batch.request_counts.failed, 1);
    }

    #[test]
    fn mixed_output_jsonl_parses() {
        let lines = parse_batch_output_jsonl(
            br#"{"custom_id":"ok","response":{"status_code":200,"body":{"ok":true}},"error":null}
{"custom_id":"bad","response":null,"error":{"code":"bad_request"}}
"#,
        )
        .unwrap();
        assert_eq!(lines.len(), 2);
        assert_eq!(lines[1].error.as_ref().unwrap()["code"], "bad_request");
    }

    #[test]
    fn unknown_lifecycle_is_rejected() {
        assert!(BatchHandle::from_openai_json(
            serde_json::json!({"id":"b","status":"mystery","input_file_id":"f"})
        )
        .is_err());
    }
}
