use std::{
    collections::HashMap,
    path::PathBuf,
    sync::{mpsc, Arc, Mutex},
    thread,
    time::Instant,
};

use engine_logging::{engine_error, engine_info, engine_warn};
use tokio::runtime::Runtime;

use crate::llm::{
    pricing::PricingRegistry,
    prompt::{PromptId, PromptRegistry, PromptTemplate, PromptVersion, TemplateVars},
    quota::{LlmQuotaTracker, LlmQuotas},
    replay::{content_hash, persist_replay_record, ReplayRecord},
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
    let runtime = Runtime::new().expect("failed to build tokio runtime for LLM worker");
    let mut quota_tracker = LlmQuotaTracker::new(config.quotas.clone());

    for cmd in cmd_rx {
        match cmd {
            LlmCommand::Stop => break,
            LlmCommand::Complete {
                request_id,
                prompt_id,
                prompt_version,
                input_content,
                context,
            } => {
                handle_completion(
                    request_id,
                    prompt_id,
                    prompt_version,
                    input_content,
                    context,
                    &config,
                    &runtime,
                    &mut quota_tracker,
                    &event_tx,
                );
            }
        }
    }
}

fn handle_completion(
    request_id: u64,
    prompt_id: PromptId,
    prompt_version: Option<PromptVersion>,
    input_content: String,
    context: Vec<(String, String)>,
    config: &LlmConfig,
    runtime: &Runtime,
    quota_tracker: &mut LlmQuotaTracker,
    event_tx: &mpsc::Sender<LlmEvent>,
) {
    if input_content.len() > config.max_input_chars {
        let err = LlmCompletionError::InputTooLarge {
            size: input_content.len(),
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
        input_content.len()
    );

    let model = resolve_model(prompt_id, config);
    let (template, version) = match fetch_prompt_template(config, prompt_id, prompt_version) {
        Some(pair) => pair,
        None => {
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
    for (key, value) in context.iter() {
        vars.insert(key.clone(), value.clone());
    }
    let rendered = vars.to_map();
    let system_message = render_template(template.system_template, &rendered);
    let user_message = render_template(template.user_template, &rendered);

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
    let run_result = runtime.block_on(config.provider.complete(&request));
    let duration_ms = start_at.elapsed().as_millis();

    if let Err(err) = run_result {
        engine_warn!(
            "[llm-worker] request_id={} provider error={}",
            request_id,
            err
        );
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

    if let Err(failure) =
        quota_tracker.check_call(usage.input_tokens as u64, usage.output_tokens as u64, cost)
    {
        let _ = event_tx.send(LlmEvent::Completed {
            request_id,
            result: Err(LlmCompletionError::QuotaExhausted {
                description: failure.to_string(),
            }),
        });
        return;
    }

    quota_tracker.record_call(usage.input_tokens as u64, usage.output_tokens as u64, cost);

    let totals = quota_tracker.totals();
    engine_info!(
        "[llm-quota] request_id={} calls={} input={} output={} cost={}",
        request_id,
        totals.calls,
        totals.input_tokens,
        totals.output_tokens,
        totals.cost_microdollars
    );

    let raw_response = response.content().to_string();
    let validation_result = validate_response(prompt_id, &raw_response);

    let record = ReplayRecord {
        request_id: format!("{}-{request_id}", config.session_id),
        input_content_hash: content_hash(&input_content),
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

    if validation_result.is_ok() {
        let validated_output = match serde_json::from_str::<serde_json::Value>(&raw_response) {
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

        let record = ReplayRecord {
            validated_output,
            ..record
        };

        if let Err(err) = persist_replay_record(&config.replay_output_dir(), &record) {
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

        let result = LlmCompletionResult {
            output_json: raw_response,
            usage,
            model_id: response.model_id().clone(),
            prompt_id,
            prompt_version: version,
        };

        let _ = event_tx.send(LlmEvent::Completed {
            request_id,
            result: Ok(result),
        });
        return;
    }

    let reason = validation_result.err().unwrap().to_string();
    engine_warn!(
        "[llm-validation] request_id={} prompt_id={:?} failure={}",
        request_id,
        prompt_id,
        reason
    );

    let record = ReplayRecord {
        validation_error: Some(reason.clone()),
        ..record
    };

    if let Err(err) = persist_replay_record(&config.replay_output_dir(), &record) {
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

fn fetch_prompt_template<'a>(
    config: &'a LlmConfig,
    prompt_id: PromptId,
    prompt_version: Option<PromptVersion>,
) -> Option<(&'a PromptTemplate, PromptVersion)> {
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

fn render_template(template: &str, vars: &HashMap<String, String>) -> String {
    let mut text = template.to_string();
    for (key, value) in vars {
        let placeholder = format!("{{{{{}}}}}", key);
        text = text.replace(&placeholder, value);
    }
    text
}

fn validate_response(prompt_id: PromptId, content: &str) -> Result<(), ValidationError> {
    match prompt_id {
        PromptId::ArticleTriage => {
            let _ = validate_triage(content)?;
            Ok(())
        }
        PromptId::ArticleSummary => {
            let _ = validate_summary(content)?;
            Ok(())
        }
        PromptId::AggregateBriefing => {
            let _ = validate_briefing(content)?;
            Ok(())
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
