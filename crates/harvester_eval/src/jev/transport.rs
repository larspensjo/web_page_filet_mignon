//! The small provider boundary used by the experiment runner.
//!
//! It deliberately lives here rather than in a production provider crate.  In
//! particular, its outcomes retain raw bytes so the runner can persist them
//! before treating a model response as data.

use std::collections::BTreeMap;
use std::env;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use serde::Deserialize;

use super::request::JevRequest;

/// Result of one attempt.  A body is retained even for non-success statuses.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TransportOutcome {
    Response {
        status: u16,
        body: Vec<u8>,
        model_latency: Duration,
    },
    Timeout {
        model_latency: Duration,
    },
    ConnectionError {
        detail: String,
        model_latency: Duration,
    },
}

/// Stable coordinates for one transport attempt. Fake scenarios use these to
/// restart per repetition and to script successive `--retry-failed` passes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AttemptContext {
    pub repetition: u32,
    pub attempt_sequence: u32,
    pub attempt: u32,
}

/// Sends one Jev request.  The runner owns retry policy and persistence.
pub trait JevTransport: Send + Sync {
    fn send(
        &self,
        request: &JevRequest,
        context: AttemptContext,
    ) -> impl std::future::Future<Output = TransportOutcome> + Send;
}

/// HTTP implementation for the user-executed live run only.
pub struct HttpTransport {
    client: reqwest::Client,
    endpoint: String,
    api_key: String,
}

impl HttpTransport {
    /// Reads the key once.  The error intentionally mentions the launcher, not
    /// the absent value, and this type never exposes the key through Debug.
    pub fn from_environment(endpoint: String, timeout: Duration) -> anyhow::Result<Self> {
        let api_key = env::var("TYPESAFE_AI_API_KEY").unwrap_or_default();
        if api_key.trim().is_empty() {
            anyhow::bail!(
                "TYPESAFE_AI_API_KEY is missing; use Start-HarvesterEval.ps1 for a live run"
            );
        }
        let client = reqwest::Client::builder().timeout(timeout).build()?;
        Ok(Self {
            client,
            endpoint,
            api_key,
        })
    }
}

impl JevTransport for HttpTransport {
    async fn send(&self, request: &JevRequest, _context: AttemptContext) -> TransportOutcome {
        let started = Instant::now();
        match self
            .client
            .post(&self.endpoint)
            .bearer_auth(&self.api_key)
            .json(request)
            .send()
            .await
        {
            Ok(response) => {
                let status = response.status().as_u16();
                match response.bytes().await {
                    Ok(body) => TransportOutcome::Response {
                        status,
                        body: body.to_vec(),
                        model_latency: started.elapsed(),
                    },
                    Err(error) if error.is_timeout() => TransportOutcome::Timeout {
                        model_latency: started.elapsed(),
                    },
                    Err(error) => TransportOutcome::ConnectionError {
                        detail: error.to_string(),
                        model_latency: started.elapsed(),
                    },
                }
            }
            Err(error) if error.is_timeout() => TransportOutcome::Timeout {
                model_latency: started.elapsed(),
            },
            Err(error) => TransportOutcome::ConnectionError {
                detail: error.to_string(),
                model_latency: started.elapsed(),
            },
        }
    }
}

/// File-backed responder used by all automated tests and offline rehearsals.
pub struct FakeTransport {
    directory: PathBuf,
    scenarios: BTreeMap<String, FakeScenario>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(untagged)]
enum FakeStep {
    Status {
        status: u16,
        #[serde(default)]
        latency_ms: u64,
    },
    Body {
        body: String,
        #[serde(default)]
        latency_ms: u64,
    },
    Timeout {
        timeout: bool,
        #[serde(default)]
        latency_ms: u64,
    },
    Connection {
        connection_error: String,
        #[serde(default)]
        latency_ms: u64,
    },
}

#[derive(Debug, Clone, Deserialize)]
#[serde(untagged)]
enum FakeScenario {
    Steps(Vec<FakeStep>),
    Passes { passes: Vec<Vec<FakeStep>> },
}

#[derive(Debug, Deserialize)]
#[serde(untagged)]
enum ScenarioFile {
    Direct(BTreeMap<String, FakeScenario>),
    Wrapped {
        articles: BTreeMap<String, FakeScenario>,
    },
}

impl FakeTransport {
    pub fn new(directory: impl Into<PathBuf>) -> anyhow::Result<Self> {
        let directory = directory.into();
        let scenario_path = directory.join("scenario.json");
        let scenarios = if scenario_path.exists() {
            let text = std::fs::read_to_string(&scenario_path)?;
            match serde_json::from_str::<ScenarioFile>(&text)? {
                ScenarioFile::Direct(articles) | ScenarioFile::Wrapped { articles } => articles,
            }
        } else {
            BTreeMap::new()
        };
        Ok(Self {
            directory,
            scenarios,
        })
    }

    fn body_for(&self, article_id: &str) -> TransportOutcome {
        let path = response_path(&self.directory, article_id);
        match std::fs::read(path) {
            Ok(body) => TransportOutcome::Response {
                status: 200,
                body,
                model_latency: Duration::ZERO,
            },
            Err(error) => TransportOutcome::ConnectionError {
                detail: format!("fake response unavailable for {article_id}: {error}"),
                model_latency: Duration::ZERO,
            },
        }
    }
}

fn response_path(directory: &Path, article_id: &str) -> PathBuf {
    let specific = directory.join(format!("{article_id}.json"));
    if specific.exists() {
        specific
    } else {
        directory.join("default.json")
    }
}

impl JevTransport for FakeTransport {
    async fn send(&self, request: &JevRequest, context: AttemptContext) -> TransportOutcome {
        let article_id = &request.state.article.id;
        let repetition_key = format!("{article_id}#{}", context.repetition);
        if let Some(scenario) = self
            .scenarios
            .get(&repetition_key)
            .or_else(|| self.scenarios.get(article_id))
        {
            let steps = match scenario {
                FakeScenario::Steps(steps) => Some(steps),
                FakeScenario::Passes { passes } => passes
                    .get(context.attempt_sequence.saturating_sub(1) as usize)
                    .or_else(|| passes.last()),
            };
            if let Some(step) = steps.and_then(|steps| {
                steps
                    .get(context.attempt.saturating_sub(1) as usize)
                    .or_else(|| steps.last())
            }) {
                let latency = Duration::from_millis(match step {
                    FakeStep::Status { latency_ms, .. }
                    | FakeStep::Body { latency_ms, .. }
                    | FakeStep::Timeout { latency_ms, .. }
                    | FakeStep::Connection { latency_ms, .. } => *latency_ms,
                });
                if !latency.is_zero() {
                    tokio::time::sleep(latency).await;
                }
                return match step {
                    FakeStep::Status { status, .. } => TransportOutcome::Response {
                        status: *status,
                        body: Vec::new(),
                        model_latency: latency,
                    },
                    FakeStep::Body { body, .. } => TransportOutcome::Response {
                        status: 200,
                        body: body.as_bytes().to_vec(),
                        model_latency: latency,
                    },
                    FakeStep::Timeout { timeout, .. } if *timeout => TransportOutcome::Timeout {
                        model_latency: latency,
                    },
                    FakeStep::Timeout { .. } => self.body_for(article_id),
                    FakeStep::Connection {
                        connection_error, ..
                    } => TransportOutcome::ConnectionError {
                        detail: connection_error.clone(),
                        model_latency: latency,
                    },
                };
            }
        }
        self.body_for(article_id)
    }
}
