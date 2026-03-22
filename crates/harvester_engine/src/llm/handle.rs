use std::{
    path::PathBuf,
    sync::{mpsc, Arc, Mutex, RwLock},
    thread,
    time::{Duration, Instant},
};
use tokio::sync::Semaphore;

use engine_logging::{engine_error, engine_info, engine_warn};
use serde_json::{json, Value};
use tokio::runtime::Runtime;

use crate::llm::{
    pricing::PricingRegistry,
    prompt::{
        is_draft_version, render_template, PromptId, PromptRegistry, PromptTemplateOwned,
        PromptVersion, TemplateVars,
    },
    quota::{LlmQuotaTracker, LlmQuotas, LlmUsageTotals},
    replay::{content_hash, persist_replay_record, ReplayProvider, ReplayRecord},
    run_metadata::{CacheStatus, LlmFailureMetadata, LlmRunMetadata, LlmRunMetadataInit},
    types::{ChatMessage, ChatRole, LlmError, LlmRequest, ModelId, ProviderKind},
    validation::{validate_briefing, validate_summary, validate_triage, ValidationError},
};

/// Configuration for the LLM worker + handle.
pub struct LlmConfig {
    pub provider: Arc<dyn crate::llm::provider::LlmProvider>,
    pub default_model: ModelId,
    pub triage_model: Option<ModelId>,
    pub summary_model: Option<ModelId>,
    pub briefing_model: Option<ModelId>,
    pub registry: Arc<RwLock<PromptRegistry>>,
    pub quotas: LlmQuotas,
    pub output_dir: PathBuf,
    pub pricing: PricingRegistry,
    /// Maximum byte length of the input content passed to the worker.
    pub max_input_bytes: usize,
    /// Deprecated alias for `max_input_bytes`. Will be removed in a future release.
    #[deprecated(note = "use max_input_bytes")]
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

    /// Effective byte limit, resolving the deprecated `max_input_chars` alias.
    fn effective_max_input_bytes(&self) -> usize {
        #[allow(deprecated)]
        if self.max_input_bytes != 0 {
            self.max_input_bytes
        } else {
            self.max_input_chars
        }
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

    /// Retrieve a snapshot of current LLM usage totals (calls, tokens, cost).
    /// Returns immutable copy; blocks briefly to query the worker thread.
    pub fn usage_totals(&self) -> Option<LlmUsageTotals> {
        let (response_tx, response_rx) = mpsc::channel();
        self.cmd_tx
            .send(LlmCommand::GetUsageTotals { response_tx })
            .ok()?;
        response_rx.recv().ok()
    }

    /// Send a stop signal and drop the sender, causing the worker thread to drain all
    /// in-flight requests and shut down cleanly.
    pub fn drain_and_stop(self) {
        // Sending Stop signals the worker to break its receive loop after draining.
        let _ = self.cmd_tx.send(LlmCommand::Stop);
        // cmd_tx is dropped here, which also closes the channel.
    }
}

pub struct LlmCompletionCommand {
    pub request_id: u64,
    pub prompt_id: PromptId,
    pub prompt_version: Option<PromptVersion>,
    /// Per-run model override; `None` means use the stage/default model.
    pub model_override: Option<ModelId>,
    pub input_content: String,
    pub context: Vec<(String, String)>,
    pub template_override: Option<PromptTemplateOwned>,
    /// Extra key-value pairs inserted as individual template variables ({{key}}).
    /// NOT concatenated into the {{context}} block.
    pub extra_template_vars: Vec<(String, String)>,
}

pub enum LlmCommand {
    Complete(Box<LlmCompletionCommand>),
    GetUsageTotals {
        response_tx: mpsc::Sender<LlmUsageTotals>,
    },
    Stop,
}

pub struct LlmCompletionResult {
    pub output_json: String,
    pub metadata: LlmRunMetadata,
}

/// Typed error categories surfaced by the worker.
#[derive(Debug)]
pub enum LlmCompletionError {
    ProviderError(LlmError),
    ValidationFailed {
        reason: String,
        raw_response: String,
        failure_metadata: Option<LlmFailureMetadata>,
    },
    QuotaExhausted {
        description: String,
        failure_metadata: Option<LlmFailureMetadata>,
    },
    PromptNotFound {
        prompt_id: PromptId,
    },
    PersistenceFailed {
        detail: String,
        failure_metadata: Option<LlmFailureMetadata>,
    },
    InputTooLarge {
        size: usize,
        limit: usize,
    },
    TemplateRenderFailed {
        detail: String,
    },
    /// The caller supplied a `model_override` that is not supported by the
    /// configured provider or is not in the known-model allow-list.
    /// This is a pre-flight check; `failure_metadata` is always `None`.
    UnsupportedModel {
        model: ModelId,
        reason: String,
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
    let runtime = Runtime::new().expect("failed to build tokio runtime for LLM worker");
    let quota_tracker = Arc::new(Mutex::new(LlmQuotaTracker::new(config.quotas.clone())));
    let config = Arc::new(config);
    let semaphore = Arc::new(Semaphore::new(max_concurrent));

    let mut join_handles: Vec<tokio::task::JoinHandle<()>> = Vec::new();

    for cmd in &cmd_rx {
        match cmd {
            LlmCommand::Stop => break,
            LlmCommand::GetUsageTotals { response_tx } => {
                let totals = quota_tracker.lock().unwrap().totals();
                let _ = response_tx.send(totals);
            }
            LlmCommand::Complete(_) => {
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
        }
    }

    wait_for_inflight_tasks(&runtime, join_handles);
    shutdown_runtime(runtime);
}

fn wait_for_inflight_tasks(runtime: &Runtime, join_handles: Vec<tokio::task::JoinHandle<()>>) {
    runtime.block_on(async move {
        for (index, handle) in join_handles.into_iter().enumerate() {
            match tokio::time::timeout(LLM_TASK_DRAIN_TIMEOUT, handle).await {
                Ok(Ok(())) => {}
                Ok(Err(err)) => {
                    engine_warn!(
                        "[llm-shutdown] task join failed index={} error={}",
                        index,
                        err
                    );
                }
                Err(_) => {
                    engine_warn!(
                        "[llm-shutdown] timed out waiting for task index={} timeout_ms={}",
                        index,
                        LLM_TASK_DRAIN_TIMEOUT.as_millis()
                    );
                    break;
                }
            }
        }
    });
}

fn shutdown_runtime(runtime: Runtime) {
    runtime.shutdown_timeout(LLM_RUNTIME_SHUTDOWN_TIMEOUT);
}

async fn handle_completion_concurrent(
    command: LlmCommand,
    config: &Arc<LlmConfig>,
    quota_tracker: &Mutex<LlmQuotaTracker>,
    event_tx: &mpsc::Sender<LlmEvent>,
) {
    let LlmCommand::Complete(command) = command else {
        return;
    };
    let LlmCompletionCommand {
        request_id,
        prompt_id,
        prompt_version,
        model_override,
        input_content,
        context,
        template_override,
        extra_template_vars,
    } = *command;

    // Capture wall clock before any work so the full span is covered.
    let start_at = Instant::now();
    let timestamp_utc = (config.timestamp_utc)();

    let input_bytes = input_content.len();
    let max_bytes = config.effective_max_input_bytes();
    if input_bytes > max_bytes {
        let err = LlmCompletionError::InputTooLarge {
            size: input_bytes,
            limit: max_bytes,
        };
        let _ = event_tx.send(LlmEvent::Completed {
            request_id,
            result: Err(err),
        });
        return;
    }

    engine_info!(
        "[llm-dispatch] request_id={} prompt_id={:?} bytes={}",
        request_id,
        prompt_id,
        input_bytes
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
                    failure_metadata: None,
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

    // Validate the model override (if any) before doing any further work.
    if let Some(ref override_model) = model_override {
        if let Err(validation_err) = validate_model_override(override_model, config, None) {
            quota_tracker.lock().unwrap().release_call();
            let _ = event_tx.send(LlmEvent::Completed {
                request_id,
                result: Err(validation_err.into_completion_error(override_model.clone())),
            });
            return;
        }
    }
    let model = resolve_model(prompt_id, model_override.as_ref(), config);
    if let Some(ref override_model) = model_override {
        engine_info!(
            "[llm-dispatch] request_id={} override model={:?}/{} resolved={:?}/{}",
            request_id,
            override_model.provider(),
            override_model.model_name(),
            model.provider(),
            model.model_name()
        );
    }
    let (system_template, user_template, version) = if let Some(override_template) =
        &template_override
    {
        (
            override_template.system_template.clone(),
            override_template.user_template.clone(),
            override_template.version,
        )
    } else {
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
        (template.system_template, template.user_template, version)
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
    for (key, value) in extra_template_vars.iter() {
        vars.insert(key.clone(), value.clone());
    }
    let rendered = vars.to_map();
    let system_message = match render_template(&system_template, &rendered) {
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
    let user_message = match render_template(&user_template, &rendered) {
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
                let wall_ms = start_at.elapsed().as_millis() as u64;
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

                let metadata = LlmRunMetadata::new(LlmRunMetadataInit {
                    prompt_id,
                    prompt_version: version,
                    resolved_model: model.model_name().to_string(),
                    input_bytes,
                    input_tokens: record.usage.input_tokens,
                    output_tokens: record.usage.output_tokens,
                    cached_input_tokens: record.usage.cached_input_tokens,
                    cost_microdollars: record.cost_microdollars,
                    wall_ms,
                    parse_ok: true,
                    validation_error: None,
                    cache_status: CacheStatus::HitValidated,
                    timestamp_utc: record.timestamp_utc.clone(),
                });
                engine_info!(
                    "[llm-run] request_id={} prompt_id={:?} version={} model={} input_bytes={} input_tokens={} output_tokens={} cached_input_tokens={} cost_microdollars={} wall_ms={} parse_ok=true cache_status=hit_validated",
                    request_id, metadata.prompt_id, metadata.prompt_version, metadata.resolved_model,
                    metadata.input_bytes, metadata.input_tokens, metadata.output_tokens,
                    metadata.cached_input_tokens, metadata.cost_microdollars, metadata.wall_ms
                );
                let result = LlmCompletionResult {
                    output_json,
                    metadata,
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

    let mut attempt = 1u32;
    let run_result = loop {
        match config.provider.complete(&request).await {
            Ok(resp) => {
                if attempt > 1 {
                    engine_info!(
                        "[llm-retry] request_id={} succeeded on attempt={}",
                        request_id,
                        attempt
                    );
                }
                break Ok(resp);
            }
            Err(e) if e.is_retryable() && attempt < MAX_ATTEMPTS => {
                engine_warn!(
                    "[llm-retry] request_id={} attempt={} error={}",
                    request_id,
                    attempt,
                    e
                );
                tokio::time::sleep(RETRY_DELAY).await;
                attempt += 1;
            }
            Err(e) => {
                if attempt > 1 {
                    engine_warn!(
                        "[llm-retry] request_id={} exhausted after {} attempts",
                        request_id,
                        attempt
                    );
                }
                break Err(e);
            }
        }
    };
    let wall_ms = start_at.elapsed().as_millis() as u64;

    if let Err(err) = run_result {
        engine_warn!(
            "[llm-worker] request_id={} provider error={}",
            request_id,
            err
        );
        quota_tracker.lock().unwrap().release_call();
        let failure_metadata = LlmFailureMetadata {
            prompt_id,
            prompt_version: version,
            resolved_model: Some(model.model_name().to_string()),
            input_bytes,
            wall_ms: Some(wall_ms),
            timestamp_utc: timestamp_utc.clone(),
        };
        let _ = event_tx.send(LlmEvent::Completed {
            request_id,
            result: Err(LlmCompletionError::ProviderError(err)),
        });
        drop(failure_metadata); // metadata available if needed in future
        return;
    }

    let response = run_result.unwrap();

    let usage = response.usage();
    let cost = config
        .pricing
        .cost_microdollars(response.model_id().model_name(), &usage);
    if cost == 0 && !config.pricing.is_empty() {
        engine_warn!(
            "[llm-run] WARN missing pricing model={}",
            response.model_id().model_name()
        );
    }

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
            let failure_metadata = LlmFailureMetadata {
                prompt_id,
                prompt_version: version,
                resolved_model: Some(response.model_id().model_name().to_string()),
                input_bytes,
                wall_ms: Some(wall_ms),
                timestamp_utc: timestamp_utc.clone(),
            };
            let _ = event_tx.send(LlmEvent::Completed {
                request_id,
                result: Err(LlmCompletionError::QuotaExhausted {
                    description: failure.to_string(),
                    failure_metadata: Some(failure_metadata),
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
        timestamp_utc: timestamp_utc.clone(),
        rendered_system_message: system_message.clone(),
        rendered_user_message: user_message.clone(),
        raw_response: raw_response.clone(),
        usage,
        validated_output: None,
        validation_error: None,
        cost_microdollars: cost,
        wall_ms,
        cache_status: "miss".to_string(),
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

            if let Err(err) = persist_replay_record(&config.replay_output_dir(), &success_record) {
                engine_error!(
                    "[llm-replay] request_id={} persist failed: {}",
                    request_id,
                    err
                );
                let failure_metadata = LlmFailureMetadata {
                    prompt_id,
                    prompt_version: version,
                    resolved_model: Some(response.model_id().model_name().to_string()),
                    input_bytes,
                    wall_ms: Some(wall_ms),
                    timestamp_utc: timestamp_utc.clone(),
                };
                let _ = event_tx.send(LlmEvent::Completed {
                    request_id,
                    result: Err(LlmCompletionError::PersistenceFailed {
                        detail: err.to_string(),
                        failure_metadata: Some(failure_metadata),
                    }),
                });
                return;
            }

            if let Some(ref cache) = config.replay_cache {
                let mut guard = cache.write().unwrap();
                guard.insert(success_record.clone());
            }

            let metadata = LlmRunMetadata::new(LlmRunMetadataInit {
                prompt_id,
                prompt_version: version,
                resolved_model: response.model_id().model_name().to_string(),
                input_bytes,
                input_tokens: usage.input_tokens,
                output_tokens: usage.output_tokens,
                cached_input_tokens: usage.cached_input_tokens,
                cost_microdollars: cost,
                wall_ms,
                parse_ok: true,
                validation_error: None,
                cache_status: CacheStatus::Miss,
                timestamp_utc: timestamp_utc.clone(),
            });
            engine_info!(
                "[llm-run] request_id={} prompt_id={:?} version={} model={} input_bytes={} input_tokens={} output_tokens={} cached_input_tokens={} cost_microdollars={} wall_ms={} parse_ok=true cache_status=miss",
                request_id, metadata.prompt_id, metadata.prompt_version, metadata.resolved_model,
                metadata.input_bytes, metadata.input_tokens, metadata.output_tokens,
                metadata.cached_input_tokens, metadata.cost_microdollars, metadata.wall_ms
            );

            let result = LlmCompletionResult {
                output_json: validated_output_json,
                metadata,
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

            if let Err(err) = persist_replay_record(&config.replay_output_dir(), &failure_record) {
                engine_error!(
                    "[llm-replay] request_id={} persist failed: {}",
                    request_id,
                    err
                );
                let failure_metadata = LlmFailureMetadata {
                    prompt_id,
                    prompt_version: version,
                    resolved_model: Some(response.model_id().model_name().to_string()),
                    input_bytes,
                    wall_ms: Some(wall_ms),
                    timestamp_utc: timestamp_utc.clone(),
                };
                let _ = event_tx.send(LlmEvent::Completed {
                    request_id,
                    result: Err(LlmCompletionError::PersistenceFailed {
                        detail: err.to_string(),
                        failure_metadata: Some(failure_metadata),
                    }),
                });
                return;
            }

            let failure_metadata = LlmFailureMetadata {
                prompt_id,
                prompt_version: version,
                resolved_model: Some(response.model_id().model_name().to_string()),
                input_bytes,
                wall_ms: Some(wall_ms),
                timestamp_utc: timestamp_utc.clone(),
            };
            let _ = event_tx.send(LlmEvent::Completed {
                request_id,
                result: Err(LlmCompletionError::ValidationFailed {
                    reason,
                    raw_response,
                    failure_metadata: Some(failure_metadata),
                }),
            });
        }
    }
}

/// Resolve the model for a request.
///
/// Precedence (highest to lowest):
/// 1. `model_override` if `Some` — caller's explicit choice.
/// 2. Per-prompt-id stage model (`triage_model`, `summary_model`, `briefing_model`) if configured.
/// 3. `config.default_model` — unconditional fallback.
fn resolve_model(
    prompt_id: PromptId,
    model_override: Option<&ModelId>,
    config: &LlmConfig,
) -> ModelId {
    if let Some(override_model) = model_override {
        return override_model.clone();
    }
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

/// Returns the set of model names the engine will accept as overrides
/// from local config alone (stage models + pricing registry keys).
pub fn local_dispatchable_model_names(config: &LlmConfig) -> Vec<String> {
    let mut names = std::collections::HashSet::new();
    names.insert(config.default_model.model_name().to_string());
    if let Some(m) = &config.triage_model {
        names.insert(m.model_name().to_string());
    }
    if let Some(m) = &config.summary_model {
        names.insert(m.model_name().to_string());
    }
    if let Some(m) = &config.briefing_model {
        names.insert(m.model_name().to_string());
    }
    for key in config.pricing.model_names() {
        names.insert(key.to_string());
    }
    let mut result: Vec<String> = names.into_iter().collect();
    result.sort();
    result
}

/// Validate a model override before use.
///
/// Checks (in order):
/// 1. Provider must match `config.default_model.provider()` (proxy for the configured provider).
/// 2. Model name must be in the allow-list built from config models and the pricing registry,
///    or in the optional extra_catalog parameter (for remote-discovered models).
///
/// Returns a small validation error on failure; caller maps it into
/// `LlmCompletionError::UnsupportedModel`.
fn validate_model_override(
    override_model: &ModelId,
    config: &LlmConfig,
    extra_catalog: Option<&[ModelId]>,
) -> Result<(), ModelOverrideValidationError> {
    let configured_provider = config.default_model.provider();

    // Check 1: provider must match.
    if override_model.provider() != configured_provider {
        engine_warn!(
            "[llm-dispatch] unsupported model override: wrong provider {:?} (expected {:?}) model={}",
            override_model.provider(),
            configured_provider,
            override_model.model_name()
        );
        return Err(ModelOverrideValidationError::WrongProvider {
            configured_provider,
        });
    }

    // Check 2: model name must be in the allow-list (local or extra catalog).
    let mut allow_list: std::collections::HashSet<&str> = std::collections::HashSet::new();
    allow_list.insert(config.default_model.model_name());
    if let Some(m) = &config.triage_model {
        allow_list.insert(m.model_name());
    }
    if let Some(m) = &config.summary_model {
        allow_list.insert(m.model_name());
    }
    if let Some(m) = &config.briefing_model {
        allow_list.insert(m.model_name());
    }

    let in_local_allow_list = allow_list.contains(override_model.model_name())
        || config.pricing.get(override_model.model_name()).is_some();

    let in_extra_catalog = extra_catalog
        .map(|catalog| {
            catalog
                .iter()
                .any(|m| m.model_name() == override_model.model_name())
        })
        .unwrap_or(false);

    if !in_local_allow_list && !in_extra_catalog {
        engine_warn!(
            "[llm-dispatch] unsupported model override: unknown model name={} provider={:?}",
            override_model.model_name(),
            override_model.provider()
        );
        return Err(ModelOverrideValidationError::UnknownModelName);
    }

    Ok(())
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ModelOverrideValidationError {
    WrongProvider { configured_provider: ProviderKind },
    UnknownModelName,
}

impl ModelOverrideValidationError {
    fn into_completion_error(self, model: ModelId) -> LlmCompletionError {
        let reason = match self {
            Self::WrongProvider {
                configured_provider,
            } => format!(
                "provider {:?} does not match configured provider {:?}",
                model.provider(),
                configured_provider
            ),
            Self::UnknownModelName => format!(
                "model name '{}' is not in the known-model allow-list",
                model.model_name()
            ),
        };
        LlmCompletionError::UnsupportedModel { model, reason }
    }
}

fn fetch_prompt_template(
    config: &LlmConfig,
    prompt_id: PromptId,
    prompt_version: Option<PromptVersion>,
) -> Option<(PromptTemplateOwned, PromptVersion)> {
    let registry = config.registry.read().unwrap();
    if let Some(version) = prompt_version {
        if is_draft_version(version) {
            return None;
        }
        return registry
            .get(prompt_id, version)
            .map(|template| (PromptTemplateOwned::from(template), version));
    }

    registry
        .active(prompt_id)
        .map(|template| (PromptTemplateOwned::from(template), template.version))
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
                "top_stories": validated.top_stories.iter().map(|story| {
                    json!({
                        "headline": story.headline,
                        "body": story.body,
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

const LLM_TASK_DRAIN_TIMEOUT: Duration = Duration::from_secs(30);
const LLM_RUNTIME_SHUTDOWN_TIMEOUT: Duration = Duration::from_secs(2);
const MAX_ATTEMPTS: u32 = 2; // 1 initial + 1 retry
const RETRY_DELAY: Duration = Duration::from_secs(2);

#[cfg(test)]
mod tests {
    use super::*;
    use crate::llm::mock_provider::MockLlmProvider;
    use crate::llm::OPENAI_MODEL_GPT_4O_MINI;

    /// Simulate the TemplateVars construction that handle_completion_concurrent does,
    /// then verify that extra_template_vars are NOT folded into {{context}}.
    #[test]
    fn extra_template_vars_not_in_context_block() {
        let context: Vec<(String, String)> = vec![("analyst".to_string(), "finance".to_string())];
        let extra_template_vars: Vec<(String, String)> = vec![(
            "previous_briefings".to_string(),
            "old summary content".to_string(),
        )];

        // Replicate the logic in handle_completion_concurrent
        let mut vars = TemplateVars::new();
        vars.set_document("content", "some article text");
        let context_text = context
            .iter()
            .map(|(k, v)| format!("{}: {}", k, v))
            .collect::<Vec<_>>()
            .join("\n");
        vars.insert("context".to_string(), context_text);
        for (key, value) in context.iter() {
            vars.insert(key.clone(), value.clone());
        }
        for (key, value) in extra_template_vars.iter() {
            vars.insert(key.clone(), value.clone());
        }
        let rendered = vars.to_map();

        // {{context}} must NOT contain the extra var value
        let context_rendered = render_template("{{context}}", &rendered).unwrap();
        assert!(
            !context_rendered.contains("old summary content"),
            "extra_template_vars must not appear inside {{{{context}}}}: got {:?}",
            context_rendered
        );

        // {{previous_briefings}} must render the extra var value
        let pb_rendered = render_template("{{previous_briefings}}", &rendered).unwrap();
        assert_eq!(pb_rendered, "old summary content");
    }

    #[test]
    fn runtime_shutdown_is_time_bounded_when_blocking_task_is_stuck() {
        engine_logging::initialize_for_tests();

        let runtime = Runtime::new().expect("runtime");
        let (_tx, rx) = mpsc::channel::<()>();
        runtime.spawn_blocking(move || {
            let _ = rx.recv();
        });

        let start = Instant::now();
        shutdown_runtime(runtime);
        let elapsed = start.elapsed();

        assert!(
            elapsed < Duration::from_secs(4),
            "runtime shutdown exceeded expected timeout: {:?}",
            elapsed
        );
    }

    fn test_llm_config() -> LlmConfig {
        let provider = Arc::new(MockLlmProvider::new());
        let registry = Arc::new(RwLock::new(PromptRegistry::with_defaults()));
        #[allow(deprecated)]
        {
            LlmConfig {
                provider,
                default_model: ModelId::new(ProviderKind::OpenAi, OPENAI_MODEL_GPT_4O_MINI),
                triage_model: None,
                summary_model: None,
                briefing_model: None,
                registry,
                quotas: LlmQuotas::default(),
                output_dir: PathBuf::from("."),
                pricing: PricingRegistry::with_defaults(),
                max_input_bytes: 100_000,
                max_input_chars: 0,
                timestamp_utc: Arc::new(|| "2026-01-01T00:00:00Z".to_string()),
                session_id: "test-session".to_string(),
                replay_cache: None,
                max_concurrent_requests: 1,
            }
        }
    }

    #[test]
    fn validate_model_override_rejects_wrong_provider_with_small_error_type() {
        let config = test_llm_config();
        let override_model = ModelId::new(ProviderKind::Anthropic, "claude-3-5-sonnet");

        let err = validate_model_override(&override_model, &config, None).unwrap_err();

        assert_eq!(
            err,
            ModelOverrideValidationError::WrongProvider {
                configured_provider: ProviderKind::OpenAi
            }
        );
    }

    #[test]
    fn validate_model_override_rejects_unknown_model_name_with_small_error_type() {
        let config = test_llm_config();
        let override_model = ModelId::new(ProviderKind::OpenAi, "definitely-unknown-model");

        let err = validate_model_override(&override_model, &config, None).unwrap_err();

        assert_eq!(err, ModelOverrideValidationError::UnknownModelName);
    }
}
