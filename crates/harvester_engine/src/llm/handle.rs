use std::{
    path::PathBuf,
    sync::{mpsc, Arc, Mutex, RwLock},
    thread,
    time::Instant,
};
use tokio::sync::Semaphore;

use engine_logging::{engine_error, engine_info, engine_warn};
use serde_json::{json, Value};
use tokio::runtime::Runtime;

use crate::llm::{
    pricing::PricingRegistry,
    prompt::{
        render_template, PromptId, PromptRegistry, PromptTemplate, PromptVersion, TemplateVars,
    },
    quota::{LlmQuotaTracker, LlmQuotas},
    replay::{content_hash, persist_replay_record, ReplayProvider, ReplayRecord},
    types::{ChatMessage, ChatRole, LlmError, LlmRequest, ModelId, ProviderKind, TokenUsage},
    validation::{validate_briefing, validate_summary, validate_triage, ValidationError},
};

/// Configuration for the LLM worker + handle.
pub struct LlmConfig {
    pub provider: Arc<dyn crate::llm::provider::LlmProvider>,
    pub default_model: ModelId,
    pub triage_model: Option<ModelId>,
    pub summary_model: Option<ModelId>,
    pub briefing_model: Option<ModelId>,
    pub registry: PromptRegistry,
    pub quotas: LlmQuotas,
    pub output_dir: PathBuf,
    pub pricing: PricingRegistry,
    pub max_input_chars: usize,
    pub timestamp_utc: Arc<dyn Fn() -> String + Send + Sync>,
    pub session_id: String,
    pub replay_cache: Option<Arc<RwLock<ReplayProvider>>>,
    /// Maximum number of concurrent LLM requests. Defaults to 1 (serial). Max enforced at 10.
    pub max_concurrent_requests: usize,
}

impl LlmConfig {
    pub fn replay_output_dir(&self) -> PathBuf {
        self.output_dir.join("llm_results")
    }
}

/// Handle allowing effect runners to submit requests to the worker.
pub struct LlmHandle {
    cmd_tx: mpsc::Sender<LlmCommand>,
    event_rx: Arc<Mutex<mpsc::Receiver<LlmEvent>>>,
}

impl Clone for LlmHandle {
    fn clone(&self) -> Self {
        Self {
            cmd_tx: self.cmd_tx.clone(),
            event_rx: Arc::clone(&self.event_rx),
        }
    }
}

impl LlmHandle {
    pub fn new(config: LlmConfig) -> Self {
        let (cmd_tx, cmd_rx) = mpsc::channel();
        let (event_tx, event_rx) = mpsc::channel();
        let event_rx = Arc::new(Mutex::new(event_rx));

        thread::spawn(move || worker_loop(cmd_rx, event_tx, config));

        Self { cmd_tx, event_rx }
    }

    pub fn send(&self, cmd: LlmCommand) -> Result<(), mpsc::SendError<LlmCommand>> {
        self.cmd_tx.send(cmd)
    }

    pub fn event_receiver(&self) -> Arc<Mutex<mpsc::Receiver<LlmEvent>>> {
        Arc::clone(&self.event_rx)
    }

    /// Send a stop signal and drop the sender, causing the worker thread to drain all
    /// in-flight requests and shut down cleanly.
    pub fn drain_and_stop(self) {
        // Sending Stop signals the worker to break its receive loop after draining.
        let _ = self.cmd_tx.send(LlmCommand::Stop);
        // cmd_tx is dropped here, which also closes the channel.
    }
}

pub enum LlmCommand {
    Complete {
        request_id: u64,
        prompt_id: PromptId,
        prompt_version: Option<PromptVersion>,
        input_content: String,
        context: Vec<(String, String)>,
    },
    Stop,
}

pub struct LlmCompletionResult {
    pub output_json: String,
    pub usage: TokenUsage,
    pub model_id: ModelId,
    pub prompt_id: PromptId,
    pub prompt_version: PromptVersion,
}

/// Typed error categories surfaced by the worker.
pub enum LlmCompletionError {
    ProviderError(LlmError),
    ValidationFailed {
        reason: String,
        raw_response: String,
    },
    QuotaExhausted {
        description: String,
    },
    PromptNotFound {
        prompt_id: PromptId,
    },
    PersistenceFailed {
        detail: String,
    },
    InputTooLarge {
        size: usize,
        limit: usize,
    },
    TemplateRenderFailed {
        detail: String,
    },
}

pub enum LlmEvent {
    Completed {
        request_id: u64,
        result: Result<LlmCompletionResult, LlmCompletionError>,
    },
}

fn worker_loop(
    cmd_rx: mpsc::Receiver<LlmCommand>,
    event_tx: mpsc::Sender<LlmEvent>,
    config: LlmConfig,
) {
    let max_concurrent = config.max_concurrent_requests.clamp(1, 10);
    let runtime = Arc::new(Runtime::new().expect("failed to build tokio runtime for LLM worker"));
    let quota_tracker = Arc::new(Mutex::new(LlmQuotaTracker::new(config.quotas.clone())));
    let config = Arc::new(config);
    let semaphore = Arc::new(Semaphore::new(max_concurrent));

    let mut join_handles: Vec<tokio::task::JoinHandle<()>> = Vec::new();

    for cmd in &cmd_rx {
        if let LlmCommand::Stop = cmd {
            break;
        }

        // Acquire a semaphore permit synchronously, bounding concurrency.
        let permit = runtime
            .block_on(semaphore.clone().acquire_owned())
            .expect("semaphore closed");

        let quota = quota_tracker.clone();
        let cfg = config.clone();
        let ev_tx = event_tx.clone();

        let handle = runtime.spawn(async move {
            let _permit = permit; // held for the lifetime of this request
            handle_completion_concurrent(cmd, &cfg, &quota, &ev_tx).await;
        });
        join_handles.push(handle);
    }

    // Drain: wait for all in-flight tasks to emit their completion events.
    runtime.block_on(async {
        for handle in join_handles {
            let _ = handle.await;
        }
    });
}


async fn handle_completion_concurrent(
    command: LlmCommand,
    config: &Arc<LlmConfig>,
    quota_tracker: &Mutex<LlmQuotaTracker>,
    event_tx: &mpsc::Sender<LlmEvent>,
) {
    let LlmCommand::Complete {
        request_id,
        prompt_id,
        prompt_version,
        input_content,
        context,
    } = command
    else {
        return;
    };

    let input_len = input_content.len();
    if input_len > config.max_input_chars {
        let err = LlmCompletionError::InputTooLarge {
            size: input_len,
            limit: config.max_input_chars,
        };
        let _ = event_tx.send(LlmEvent::Completed {
            request_id,
            result: Err(err),
        });
        return;
    }

    engine_info!(
        "[llm-dispatch] request_id={} prompt_id={:?} chars={}",
        request_id,
        prompt_id,
        input_len
    );

    // Pre-call quota reservation: atomically check and reserve call slot.
    {
        let mut tracker = quota_tracker.lock().unwrap();
        if let Err(failure) = tracker.reserve_call() {
            engine_warn!(
                "[llm-quota] request_id={} rejected=pre-call reason={}",
                request_id,
                failure
            );
            let _ = event_tx.send(LlmEvent::Completed {
                request_id,
                result: Err(LlmCompletionError::QuotaExhausted {
                    description: failure.to_string(),
                }),
            });
            return;
        }
        engine_info!(
            "[llm-quota] request_id={} pre-call reservation accepted calls={}",
            request_id,
            tracker.totals().calls
        );
    }

    let model = resolve_model(prompt_id, config);
    let (template, version) = match fetch_prompt_template(config, prompt_id, prompt_version) {
        Some(pair) => pair,
        None => {
            quota_tracker.lock().unwrap().release_call();
            let _ = event_tx.send(LlmEvent::Completed {
                request_id,
                result: Err(LlmCompletionError::PromptNotFound { prompt_id }),
            });
            return;
        }
    };

    let document_key = match prompt_id {
        PromptId::AggregateBriefing => "collection",
        _ => "content",
    };

    let mut vars = TemplateVars::new();
    vars.set_document(document_key, &input_content);

    // Build context string from key-value pairs
    let context_text = if context.is_empty() {
        String::new()
    } else {
        context
            .iter()
            .map(|(k, v)| format!("{}: {}", k, v))
            .collect::<Vec<_>>()
            .join("\n")
    };
    vars.insert("context".to_string(), context_text);

    for (key, value) in context.iter() {
        vars.insert(key.clone(), value.clone());
    }
    let rendered = vars.to_map();
    let system_message = match render_template(template.system_template, &rendered) {
        Ok(msg) => msg,
        Err(e) => {
            engine_error!("[llm-render] Failed to render system template: {}", e);
            quota_tracker.lock().unwrap().release_call();
            let _ = event_tx.send(LlmEvent::Completed {
                request_id,
                result: Err(LlmCompletionError::TemplateRenderFailed {
                    detail: format!("system template: {}", e),
                }),
            });
            return;
        }
    };
    let user_message = match render_template(template.user_template, &rendered) {
        Ok(msg) => msg,
        Err(e) => {
            engine_error!("[llm-render] Failed to render user template: {}", e);
            quota_tracker.lock().unwrap().release_call();
            let _ = event_tx.send(LlmEvent::Completed {
                request_id,
                result: Err(LlmCompletionError::TemplateRenderFailed {
                    detail: format!("user template: {}", e),
                }),
            });
            return;
        }
    };
    let input_hash = content_hash(&input_content);

    if let Some(ref cache) = config.replay_cache {
        let guard = cache.read().unwrap();
        if let Some(record) = guard.lookup(&input_hash, prompt_id, version) {
            if record.validated_output.is_some() {
                engine_info!(
                    "[llm-replay] cache hit request_id={} hash={}",
                    request_id,
                    &input_hash[..8]
                );
                let output_json = record
                    .validated_output
                    .as_ref()
                    .and_then(|value| serde_json::to_string(value).ok())
                    .unwrap_or_else(|| record.raw_response.clone());
                let result = LlmCompletionResult {
                    output_json,
                    usage: record.usage,
                    model_id: model.clone(),
                    prompt_id,
                    prompt_version: version,
                };
                let _ = event_tx.send(LlmEvent::Completed {
                    request_id,
                    result: Ok(result),
                });
                return;
            }
        }
    }

    let request = LlmRequest::new(
        model.clone(),
        vec![
            ChatMessage::new(ChatRole::System, system_message.clone()),
            ChatMessage::new(ChatRole::User, user_message.clone()),
        ],
    )
    .with_json_response();

    engine_info!(
        "[llm-worker] request_id={} model={} start",
        request_id,
        model.model_name()
    );

    let start_at = Instant::now();
    let run_result = config.provider.complete(&request).await;
    let duration_ms = start_at.elapsed().as_millis();

    if let Err(err) = run_result {
        engine_warn!(
            "[llm-worker] request_id={} provider error={}",
            request_id,
            err
        );
        quota_tracker.lock().unwrap().release_call();
        let _ = event_tx.send(LlmEvent::Completed {
            request_id,
            result: Err(LlmCompletionError::ProviderError(err)),
        });
        return;
    }

    let response = run_result.unwrap();
    engine_info!(
        "[llm-worker] request_id={} model={} duration_ms={}",
        request_id,
        response.model_id().model_name(),
        duration_ms
    );

    let usage = response.usage();
    let cost = config
        .pricing
        .cost_microdollars(response.model_id().model_name(), &usage);

    // Post-call: record actual token/cost usage and check token/cost limits.
    {
        let mut tracker = quota_tracker.lock().unwrap();
        if let Err(failure) =
            tracker.record_call_usage(usage.input_tokens as u64, usage.output_tokens as u64, cost)
        {
            // Release the pre-reserved call slot since we're rejecting this result.
            tracker.release_call();
            drop(tracker);
            engine_warn!(
                "[llm-quota] request_id={} rejected=post-call reason={}",
                request_id,
                failure
            );
            let _ = event_tx.send(LlmEvent::Completed {
                request_id,
                result: Err(LlmCompletionError::QuotaExhausted {
                    description: failure.to_string(),
                }),
            });
            return;
        }
        let totals = tracker.totals();
        engine_info!(
            "[llm-quota] request_id={} calls={} input={} output={} cost={}",
            request_id,
            totals.calls,
            totals.input_tokens,
            totals.output_tokens,
            totals.cost_microdollars
        );
    }

    let raw_response = response.content().to_string();
    let validation_result = validate_response(prompt_id, &raw_response);

    let record = ReplayRecord {
        request_id: format!("{}-{request_id}", config.session_id),
        input_content_hash: input_hash.clone(),
        prompt_id,
        prompt_version: version,
        model_id: format_model_identifier(response.model_id()),
        timestamp_utc: (config.timestamp_utc)(),
        rendered_system_message: system_message.clone(),
        rendered_user_message: user_message.clone(),
        raw_response: raw_response.clone(),
        usage,
        validated_output: None,
        validation_error: None,
        cost_microdollars: cost,
    };

    match validation_result {
        Ok(validated_output_json) => {
            let validated_output = match serde_json::from_str::<Value>(&validated_output_json) {
                Ok(value) => Some(value),
                Err(err) => {
                    engine_warn!(
                        "[llm-validation] request_id={} prompt_id={:?} serialization failed: {}",
                        request_id,
                        prompt_id,
                        err
                    );
                    None
                }
            };

            let success_record = ReplayRecord {
                validated_output,
                ..record
            };

            if let Err(err) =
                persist_replay_record(&config.replay_output_dir(), &success_record)
            {
                engine_error!(
                    "[llm-replay] request_id={} persist failed: {}",
                    request_id,
                    err
                );
                let _ = event_tx.send(LlmEvent::Completed {
                    request_id,
                    result: Err(LlmCompletionError::PersistenceFailed {
                        detail: err.to_string(),
                    }),
                });
                return;
            }

            if let Some(ref cache) = config.replay_cache {
                let mut guard = cache.write().unwrap();
                guard.insert(success_record.clone());
            }

            let result = LlmCompletionResult {
                output_json: validated_output_json,
                usage,
                model_id: response.model_id().clone(),
                prompt_id,
                prompt_version: version,
            };

            let _ = event_tx.send(LlmEvent::Completed {
                request_id,
                result: Ok(result),
            });
        }
        Err(validation_error) => {
            let reason = validation_error.to_string();
            log_validation_failure(
                request_id,
                prompt_id,
                &validation_error,
                raw_response.chars().count(),
            );

            let failure_record = ReplayRecord {
                validation_error: Some(reason.clone()),
                ..record
            };

            if let Err(err) =
                persist_replay_record(&config.replay_output_dir(), &failure_record)
            {
                engine_error!(
                    "[llm-replay] request_id={} persist failed: {}",
                    request_id,
                    err
                );
                let _ = event_tx.send(LlmEvent::Completed {
                    request_id,
                    result: Err(LlmCompletionError::PersistenceFailed {
                        detail: err.to_string(),
                    }),
                });
                return;
            }

            let _ = event_tx.send(LlmEvent::Completed {
                request_id,
                result: Err(LlmCompletionError::ValidationFailed {
                    reason,
                    raw_response,
                }),
            });
        }
    }
}

fn resolve_model(prompt_id: PromptId, config: &LlmConfig) -> ModelId {
    match prompt_id {
        PromptId::ArticleTriage => config
            .triage_model
            .as_ref()
            .unwrap_or(&config.default_model)
            .clone(),
        PromptId::ArticleSummary => config
            .summary_model
            .as_ref()
            .unwrap_or(&config.default_model)
            .clone(),
        PromptId::AggregateBriefing => config
            .briefing_model
            .as_ref()
            .unwrap_or(&config.default_model)
            .clone(),
    }
}

fn fetch_prompt_template(
    config: &LlmConfig,
    prompt_id: PromptId,
    prompt_version: Option<PromptVersion>,
) -> Option<(&PromptTemplate, PromptVersion)> {
    if let Some(version) = prompt_version {
        config
            .registry
            .get(prompt_id, version)
            .map(|template| (template, version))
    } else {
        config
            .registry
            .active(prompt_id)
            .map(|template| (template, template.version))
    }
}

fn validate_response(prompt_id: PromptId, content: &str) -> Result<String, ValidationError> {
    match prompt_id {
        PromptId::ArticleTriage => {
            let _ = validate_triage(content)?;
            Ok(content.to_string())
        }
        PromptId::ArticleSummary => {
            let _ = validate_summary(content)?;
            Ok(content.to_string())
        }
        PromptId::AggregateBriefing => {
            let validated = validate_briefing(content)?;
            let normalized = json!({
                "executive_summary": validated.executive_summary,
                "themes": validated.themes.iter().map(|theme| {
                    json!({
                        "name": theme.name,
                        "description": theme.description,
                    })
                }).collect::<Vec<_>>(),
                "article_count": validated.article_count,
            });
            Ok(normalized.to_string())
        }
    }
}

fn log_validation_failure(
    request_id: u64,
    prompt_id: PromptId,
    error: &ValidationError,
    raw_response_chars: usize,
) {
    match error {
        ValidationError::FieldTooLong {
            field,
            max_chars,
            actual_chars,
        } => {
            engine_warn!(
                "[llm-validation] request_id={} prompt_id={:?} failure=field too long field={} actual_chars={} max_chars={} raw_response_chars={}",
                request_id,
                prompt_id,
                field,
                actual_chars,
                max_chars,
                raw_response_chars
            );
        }
        _ => {
            engine_warn!(
                "[llm-validation] request_id={} prompt_id={:?} failure={} raw_response_chars={}",
                request_id,
                prompt_id,
                error,
                raw_response_chars
            );
        }
    }
}

fn format_model_identifier(model_id: &ModelId) -> String {
    format!(
        "{}::{}",
        provider_kind_to_str(model_id.provider()),
        model_id.model_name()
    )
}

fn provider_kind_to_str(kind: ProviderKind) -> &'static str {
    match kind {
        ProviderKind::OpenAi => "openai",
        ProviderKind::Anthropic => "anthropic",
        ProviderKind::Google => "google",
    }
}
