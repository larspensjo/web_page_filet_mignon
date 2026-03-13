use std::sync::{Arc, RwLock};

use serde_json::json;
use tempfile::tempdir;

use harvester_engine::llm::provider::LlmProvider;
use harvester_engine::llm::{
    content_hash, BlockingMockProvider, FinishReason, LlmCommand, LlmCompletionCommand,
    LlmCompletionError, LlmConfig, LlmError, LlmEvent, LlmHandle, LlmQuotas, LlmResponse,
    MockLlmProvider, ModelId, PricingRegistry, PromptId, PromptRegistry, ProviderKind,
    ReplayProvider, ReplayRecord, TokenUsage,
};

fn make_config(
    provider_trait: Arc<dyn LlmProvider>,
    registry: Arc<RwLock<PromptRegistry>>,
    dir: &tempfile::TempDir,
) -> LlmConfig {
    LlmConfig {
        provider: provider_trait,
        default_model: ModelId::new(ProviderKind::OpenAi, "mock"),
        triage_model: None,
        summary_model: None,
        briefing_model: None,
        registry: Arc::clone(&registry),
        quotas: LlmQuotas::default(),
        output_dir: dir.path().to_path_buf(),
        pricing: PricingRegistry::new(),
        max_input_bytes: 10_000,
        #[allow(deprecated)]
        max_input_chars: 0,
        timestamp_utc: Arc::new(|| "2026-02-08T00:00:00Z".to_string()),
        session_id: "test-session".to_string(),
        replay_cache: None,
        max_concurrent_requests: 1,
    }
}

fn prompt_registry_arc() -> Arc<RwLock<PromptRegistry>> {
    Arc::new(RwLock::new(PromptRegistry::with_defaults()))
}

#[test]
fn llm_handle_dispatches_completion_event() {
    let provider = Arc::new(MockLlmProvider::new());
    let provider_trait: Arc<dyn LlmProvider> = provider.clone();
    provider.queue_json_success(
        r#"{"category":"news","priority":3,"tags":["alpha","beta"],"rationale":"ok"}"#,
    );

    let registry = prompt_registry_arc();
    let dir = tempdir().unwrap();

    let config = make_config(provider_trait, registry, &dir);

    let handle = LlmHandle::new(config);
    handle
        .send(LlmCommand::Complete(Box::new(LlmCompletionCommand {
            request_id: 7,
            prompt_id: PromptId::ArticleTriage,
            prompt_version: Some(1),
            model_override: None,
            input_content: "document text".to_string(),
            context: vec![("key".to_string(), "value".to_string())],
            template_override: None,
            extra_template_vars: vec![],
        })))
        .expect("LLM command should dispatch");

    let event = handle
        .event_receiver()
        .lock()
        .expect("lock receiver")
        .recv()
        .expect("should receive event");

    match event {
        LlmEvent::Completed { request_id, result } => {
            assert_eq!(request_id, 7);
            if let Ok(completion) = result {
                assert_eq!(completion.metadata.prompt_id, PromptId::ArticleTriage);
                assert_eq!(completion.metadata.prompt_version, 1);
                assert_eq!(completion.metadata.input_tokens, 0);
                assert_eq!(completion.metadata.output_tokens, 0);
            } else {
                panic!("LLM completion failed unexpectedly");
            }
        }
    }
}

#[test]
fn llm_handle_skips_provider_when_cache_hit() {
    let provider = Arc::new(MockLlmProvider::new());
    let provider_trait: Arc<dyn LlmProvider> = provider.clone();
    let registry = prompt_registry_arc();
    let dir = tempdir().unwrap();

    let input_content = "cached document";
    let mut replay_provider = ReplayProvider::new();
    replay_provider.insert(ReplayRecord {
        request_id: "cached-session".to_string(),
        input_content_hash: content_hash(input_content),
        prompt_id: PromptId::ArticleSummary,
        prompt_version: 2,
        model_id: "openai::mock".to_string(),
        timestamp_utc: "2026-02-08T00:00:00Z".to_string(),
        rendered_system_message: "".to_string(),
        rendered_user_message: "".to_string(),
        raw_response: r#"{"title":"cached"}"#.to_string(),
        usage: TokenUsage::new(1, 2),
        validated_output: Some(json!({"title": "cached"})),
        validation_error: None,
        cost_microdollars: 0,
        wall_ms: 0,
        cache_status: "miss".to_string(),
    });
    let replay_cache = Arc::new(RwLock::new(replay_provider));

    let mut config = make_config(provider_trait, registry, &dir);
    config.replay_cache = Some(replay_cache.clone());

    let handle = LlmHandle::new(config);
    handle
        .send(LlmCommand::Complete(Box::new(LlmCompletionCommand {
            request_id: 7,
            prompt_id: PromptId::ArticleSummary,
            prompt_version: Some(2),
            model_override: None,
            input_content: input_content.to_string(),
            context: Vec::new(),
            template_override: None,
            extra_template_vars: vec![],
        })))
        .expect("LLM command should dispatch");

    let event = handle
        .event_receiver()
        .lock()
        .expect("lock receiver")
        .recv()
        .expect("should receive event");

    match event {
        LlmEvent::Completed { request_id, result } => {
            assert_eq!(request_id, 7);
            if let Ok(completion) = result {
                assert_eq!(completion.metadata.prompt_id, PromptId::ArticleSummary);
                assert_eq!(completion.metadata.prompt_version, 2);
                assert_eq!(completion.output_json, r#"{"title":"cached"}"#.to_string());
            } else {
                panic!("cached completion failed");
            }
        }
    }

    assert_eq!(provider.recorded_requests().len(), 0);
}

#[test]
fn llm_handle_inserts_cache_after_successful_response() {
    let provider = Arc::new(MockLlmProvider::new());
    provider.queue_json_success(r#"{"title":"fresh","summary":"short","key_points":["one"]}"#);
    let provider_trait: Arc<dyn LlmProvider> = provider.clone();
    let registry = prompt_registry_arc();
    let dir = tempdir().unwrap();
    let replay_cache = Arc::new(RwLock::new(ReplayProvider::new()));

    let mut config = make_config(provider_trait, registry.clone(), &dir);
    config.replay_cache = Some(Arc::clone(&replay_cache));

    let handle = LlmHandle::new(config);
    let input_content = "fresh document";

    handle
        .send(LlmCommand::Complete(Box::new(LlmCompletionCommand {
            request_id: 7,
            prompt_id: PromptId::ArticleSummary,
            prompt_version: None, // use active version (V4)
            model_override: None,
            input_content: input_content.to_string(),
            context: Vec::new(),
            template_override: None,
            extra_template_vars: vec![],
        })))
        .expect("LLM command should dispatch");

    let first_event = handle
        .event_receiver()
        .lock()
        .expect("lock receiver")
        .recv()
        .expect("should receive first event");

    match first_event {
        LlmEvent::Completed { result, .. } => {
            assert!(result.is_ok());
        }
    }

    assert_eq!(provider.recorded_requests().len(), 1);

    handle
        .send(LlmCommand::Complete(Box::new(LlmCompletionCommand {
            request_id: 8,
            prompt_id: PromptId::ArticleSummary,
            prompt_version: None, // use active version (V4); should be a cache hit
            model_override: None,
            input_content: input_content.to_string(),
            context: Vec::new(),
            template_override: None,
            extra_template_vars: vec![],
        })))
        .expect("LLM command should dispatch");

    let second_event = handle
        .event_receiver()
        .lock()
        .expect("lock receiver")
        .recv()
        .expect("should receive second event");

    match second_event {
        LlmEvent::Completed { result, .. } => {
            assert!(result.is_ok());
        }
    }

    assert_eq!(provider.recorded_requests().len(), 1);

    let version = {
        let guard = registry.read().unwrap();
        guard
            .active(PromptId::ArticleSummary)
            .expect("summary prompt missing")
            .version
    };
    let guard = replay_cache.read().unwrap();
    assert!(guard
        .lookup(
            &content_hash(input_content),
            PromptId::ArticleSummary,
            version
        )
        .is_some());
}

/// Verify that the LLM worker never allows more than `max_concurrent_requests`
/// simultaneous provider calls, even when more requests are queued.
#[test]
fn concurrent_requests_never_exceed_cap() {
    let cap = 2usize;
    let total_requests = 5usize;

    let triage_json = r#"{"category":"news","priority":3,"tags":["t"],"rationale":"ok"}"#;
    let provider = Arc::new(BlockingMockProvider::new(triage_json));
    let provider_trait: Arc<dyn LlmProvider> = provider.clone();
    let registry = prompt_registry_arc();
    let dir = tempdir().unwrap();

    let mut config = make_config(provider_trait, registry, &dir);
    config.max_concurrent_requests = cap;

    let handle = LlmHandle::new(config);

    // Send all requests.
    for i in 0..total_requests {
        handle
            .send(LlmCommand::Complete(Box::new(LlmCompletionCommand {
                request_id: i as u64 + 1,
                prompt_id: PromptId::ArticleTriage,
                prompt_version: Some(1),
                model_override: None,
                input_content: format!("document {i}"),
                context: Vec::new(),
                template_override: None,
                extra_template_vars: vec![],
            })))
            .expect("send should succeed");
    }

    // Give the worker a moment to fill the semaphore slots.
    std::thread::sleep(std::time::Duration::from_millis(50));

    // Peak in-flight should not exceed cap.
    let peak = provider.peak_in_flight();
    assert!(peak <= cap, "peak in-flight={peak} exceeded cap={cap}");

    // Release all blocked requests so the worker can finish.
    provider.release(total_requests);

    // Drain all completion events.
    let rx = handle.event_receiver();
    let rx = rx.lock().unwrap();
    for _ in 0..total_requests {
        rx.recv_timeout(std::time::Duration::from_secs(5))
            .expect("should receive completion event");
    }

    // Verify cap was never exceeded during the full run.
    assert!(
        provider.peak_in_flight() <= cap,
        "final peak={} exceeded cap={}",
        provider.peak_in_flight(),
        cap
    );
}

// -----------------------------------------------------------------------
// Step 3 — Model override tests
// -----------------------------------------------------------------------

fn recv_event(handle: &LlmHandle) -> LlmEvent {
    handle
        .event_receiver()
        .lock()
        .expect("lock receiver")
        .recv_timeout(std::time::Duration::from_secs(5))
        .expect("should receive event within 5s")
}

#[test]
fn override_model_wins_over_stage_and_default() {
    let provider = Arc::new(MockLlmProvider::new());
    let provider_trait: Arc<dyn LlmProvider> = provider.clone();
    provider
        .queue_json_success(r#"{"category":"news","priority":3,"tags":["a"],"rationale":"ok"}"#);

    let registry = prompt_registry_arc();
    let dir = tempdir().unwrap();
    let mut config = make_config(provider_trait, registry, &dir);
    // Stage model set to something different
    config.triage_model = Some(ModelId::new(ProviderKind::OpenAi, "stage-model"));
    let override_model = ModelId::new(ProviderKind::OpenAi, "mock");

    let handle = LlmHandle::new(config);
    handle
        .send(LlmCommand::Complete(Box::new(LlmCompletionCommand {
            request_id: 1,
            prompt_id: PromptId::ArticleTriage,
            prompt_version: Some(1),
            model_override: Some(override_model.clone()),
            input_content: "document".to_string(),
            context: vec![],
            template_override: None,
            extra_template_vars: vec![],
        })))
        .unwrap();

    recv_event(&handle);
    // The override model should have been sent to the provider (not the stage model).
    let requests = provider.recorded_requests();
    assert_eq!(requests.len(), 1);
    assert_eq!(
        requests[0].model(),
        &override_model,
        "override model should be sent to provider"
    );
}

#[test]
fn stage_model_wins_over_default_when_override_is_none() {
    let provider = Arc::new(MockLlmProvider::new());
    let provider_trait: Arc<dyn LlmProvider> = provider.clone();
    provider
        .queue_json_success(r#"{"category":"news","priority":3,"tags":["a"],"rationale":"ok"}"#);

    let registry = prompt_registry_arc();
    let dir = tempdir().unwrap();
    let mut config = make_config(provider_trait, registry, &dir);
    config.triage_model = Some(ModelId::new(ProviderKind::OpenAi, "stage-specific"));
    // Pricing for "stage-specific" so allow-list is satisfied; but here we send None override
    config.pricing.insert(
        "stage-specific",
        harvester_engine::llm::ModelPricing::zero(),
    );

    let handle = LlmHandle::new(config);
    handle
        .send(LlmCommand::Complete(Box::new(LlmCompletionCommand {
            request_id: 1,
            prompt_id: PromptId::ArticleTriage,
            prompt_version: Some(1),
            model_override: None,
            input_content: "document".to_string(),
            context: vec![],
            template_override: None,
            extra_template_vars: vec![],
        })))
        .unwrap();

    recv_event(&handle);
    // The stage model should have been sent to the provider (not the default).
    let requests = provider.recorded_requests();
    assert_eq!(requests.len(), 1);
    assert_eq!(
        requests[0].model().model_name(),
        "stage-specific",
        "stage model should win when override is None"
    );
}

#[test]
fn unsupported_model_wrong_provider_fires_before_provider_call() {
    let provider = Arc::new(MockLlmProvider::new());
    let provider_trait: Arc<dyn LlmProvider> = provider.clone();
    let registry = prompt_registry_arc();
    let dir = tempdir().unwrap();
    let config = make_config(provider_trait, registry, &dir);
    // default_model is OpenAi, but we try to send Anthropic
    let bad_model = ModelId::new(ProviderKind::Anthropic, "claude-opus");

    let handle = LlmHandle::new(config);
    handle
        .send(LlmCommand::Complete(Box::new(LlmCompletionCommand {
            request_id: 1,
            prompt_id: PromptId::ArticleTriage,
            prompt_version: Some(1),
            model_override: Some(bad_model.clone()),
            input_content: "document".to_string(),
            context: vec![],
            template_override: None,
            extra_template_vars: vec![],
        })))
        .unwrap();

    match recv_event(&handle) {
        LlmEvent::Completed { result, .. } => {
            assert!(
                matches!(result, Err(LlmCompletionError::UnsupportedModel { .. })),
                "wrong provider should yield UnsupportedModel"
            );
        }
    }
    assert_eq!(
        provider.recorded_requests().len(),
        0,
        "provider must not be called for wrong-provider override"
    );
}

#[test]
fn unsupported_model_unknown_name_fires_before_provider_call() {
    let provider = Arc::new(MockLlmProvider::new());
    let provider_trait: Arc<dyn LlmProvider> = provider.clone();
    let registry = prompt_registry_arc();
    let dir = tempdir().unwrap();
    let config = make_config(provider_trait, registry, &dir);
    // Right provider but unknown name
    let bad_model = ModelId::new(ProviderKind::OpenAi, "totally-unknown-model-xyz");

    let handle = LlmHandle::new(config);
    handle
        .send(LlmCommand::Complete(Box::new(LlmCompletionCommand {
            request_id: 1,
            prompt_id: PromptId::ArticleTriage,
            prompt_version: Some(1),
            model_override: Some(bad_model.clone()),
            input_content: "document".to_string(),
            context: vec![],
            template_override: None,
            extra_template_vars: vec![],
        })))
        .unwrap();

    match recv_event(&handle) {
        LlmEvent::Completed { result, .. } => {
            assert!(
                matches!(result, Err(LlmCompletionError::UnsupportedModel { .. })),
                "unknown name should yield UnsupportedModel"
            );
        }
    }
    assert_eq!(
        provider.recorded_requests().len(),
        0,
        "provider must not be called for unknown-name override"
    );
}

#[test]
fn valid_override_cache_miss_records_override_in_metadata() {
    let provider = Arc::new(MockLlmProvider::new());
    let provider_trait: Arc<dyn LlmProvider> = provider.clone();
    provider
        .queue_json_success(r#"{"category":"news","priority":3,"tags":["a"],"rationale":"ok"}"#);

    let registry = prompt_registry_arc();
    let dir = tempdir().unwrap();
    let config = make_config(provider_trait, registry, &dir);
    // "mock" is the default model name, so it's in the allow-list
    let override_model = ModelId::new(ProviderKind::OpenAi, "mock");

    let handle = LlmHandle::new(config);
    handle
        .send(LlmCommand::Complete(Box::new(LlmCompletionCommand {
            request_id: 1,
            prompt_id: PromptId::ArticleTriage,
            prompt_version: Some(1),
            model_override: Some(override_model),
            input_content: "document".to_string(),
            context: vec![],
            template_override: None,
            extra_template_vars: vec![],
        })))
        .unwrap();

    match recv_event(&handle) {
        LlmEvent::Completed { result, .. } => {
            let metadata = result.expect("should succeed").metadata;
            assert_eq!(metadata.resolved_model, "mock");
        }
    }
}

#[test]
fn valid_override_cache_hit_records_override_in_metadata() {
    let provider = Arc::new(MockLlmProvider::new());
    let provider_trait: Arc<dyn LlmProvider> = provider.clone();
    let registry = prompt_registry_arc();
    let dir = tempdir().unwrap();

    let input_content = "cached content for override test";
    let mut replay_provider = ReplayProvider::new();
    replay_provider.insert(ReplayRecord {
        request_id: "cached-session".to_string(),
        input_content_hash: content_hash(input_content),
        prompt_id: PromptId::ArticleTriage,
        prompt_version: 1,
        model_id: "openai::mock".to_string(),
        timestamp_utc: "2026-02-08T00:00:00Z".to_string(),
        rendered_system_message: "".to_string(),
        rendered_user_message: "".to_string(),
        raw_response: r#"{"category":"news","priority":3,"tags":["a"],"rationale":"ok"}"#
            .to_string(),
        usage: TokenUsage::new(1, 2),
        validated_output: Some(
            json!({"category":"news","priority":3,"tags":["a"],"rationale":"ok"}),
        ),
        validation_error: None,
        cost_microdollars: 0,
        wall_ms: 0,
        cache_status: "miss".to_string(),
    });
    let replay_cache = Arc::new(RwLock::new(replay_provider));
    let mut config = make_config(provider_trait, registry, &dir);
    config.replay_cache = Some(replay_cache);

    let override_model = ModelId::new(ProviderKind::OpenAi, "mock");
    let handle = LlmHandle::new(config);
    handle
        .send(LlmCommand::Complete(Box::new(LlmCompletionCommand {
            request_id: 1,
            prompt_id: PromptId::ArticleTriage,
            prompt_version: Some(1),
            model_override: Some(override_model),
            input_content: input_content.to_string(),
            context: vec![],
            template_override: None,
            extra_template_vars: vec![],
        })))
        .unwrap();

    match recv_event(&handle) {
        LlmEvent::Completed { result, .. } => {
            let metadata = result.expect("cache hit should succeed").metadata;
            assert_eq!(metadata.resolved_model, "mock");
        }
    }
    assert_eq!(
        provider.recorded_requests().len(),
        0,
        "cache hit must skip provider"
    );
}

// -----------------------------------------------------------------------
// Step 6 — Retry loop tests
// -----------------------------------------------------------------------

fn make_triage_command(request_id: u64) -> LlmCommand {
    LlmCommand::Complete(Box::new(LlmCompletionCommand {
        request_id,
        prompt_id: PromptId::ArticleTriage,
        prompt_version: Some(1),
        model_override: None,
        input_content: "document text".to_string(),
        context: vec![],
        template_override: None,
        extra_template_vars: vec![],
    }))
}

fn valid_triage_response() -> LlmResponse {
    LlmResponse::new(
        r#"{"category":"news","priority":3,"tags":["a"],"rationale":"ok"}"#,
        TokenUsage::new(10, 5),
        ModelId::new(ProviderKind::OpenAi, "mock"),
        FinishReason::Stop,
    )
}

#[test]
fn retry_succeeds_on_transient_timeout() {
    let provider = Arc::new(MockLlmProvider::new());
    let provider_trait: Arc<dyn LlmProvider> = provider.clone();

    // First attempt fails with Timeout, second succeeds.
    provider.queue_response(Err(LlmError::Timeout));
    provider.queue_response(Ok(valid_triage_response()));

    let registry = prompt_registry_arc();
    let dir = tempdir().unwrap();
    let config = make_config(provider_trait, registry, &dir);
    let handle = LlmHandle::new(config);

    handle.send(make_triage_command(1)).unwrap();

    match recv_event(&handle) {
        LlmEvent::Completed { result, .. } => {
            assert!(result.is_ok(), "should succeed after retry");
        }
    }

    assert_eq!(
        provider.recorded_requests().len(),
        2,
        "should have made 2 provider calls (initial + 1 retry)"
    );
}

#[test]
fn retry_exhausted_returns_error() {
    let provider = Arc::new(MockLlmProvider::new());
    let provider_trait: Arc<dyn LlmProvider> = provider.clone();

    // Both attempts fail with Timeout.
    provider.queue_response(Err(LlmError::Timeout));
    provider.queue_response(Err(LlmError::Timeout));

    let registry = prompt_registry_arc();
    let dir = tempdir().unwrap();
    let config = make_config(provider_trait, registry, &dir);
    let handle = LlmHandle::new(config);

    handle.send(make_triage_command(2)).unwrap();

    match recv_event(&handle) {
        LlmEvent::Completed { result, .. } => {
            assert!(
                result.is_err(),
                "should fail after exhausting all attempts"
            );
            assert!(
                matches!(result, Err(LlmCompletionError::ProviderError(LlmError::Timeout))),
                "should propagate the provider error"
            );
        }
    }

    assert_eq!(
        provider.recorded_requests().len(),
        2,
        "should have made 2 provider calls (both failed)"
    );
}

#[test]
fn no_retry_on_non_transient_error() {
    let provider = Arc::new(MockLlmProvider::new());
    let provider_trait: Arc<dyn LlmProvider> = provider.clone();

    // AuthenticationFailed is not retryable.
    provider.queue_response(Err(LlmError::AuthenticationFailed));

    let registry = prompt_registry_arc();
    let dir = tempdir().unwrap();
    let config = make_config(provider_trait, registry, &dir);
    let handle = LlmHandle::new(config);

    handle.send(make_triage_command(3)).unwrap();

    match recv_event(&handle) {
        LlmEvent::Completed { result, .. } => {
            assert!(result.is_err(), "should fail immediately");
            assert!(
                matches!(
                    result,
                    Err(LlmCompletionError::ProviderError(
                        LlmError::AuthenticationFailed
                    ))
                ),
                "should propagate authentication error"
            );
        }
    }

    assert_eq!(
        provider.recorded_requests().len(),
        1,
        "should have made exactly 1 provider call (no retry for non-transient)"
    );
}

#[test]
fn retry_on_http_502() {
    let provider = Arc::new(MockLlmProvider::new());
    let provider_trait: Arc<dyn LlmProvider> = provider.clone();

    // HTTP 502 is retryable; second attempt succeeds.
    provider.queue_response(Err(LlmError::Http {
        status: 502,
        body: "Bad Gateway".to_string(),
    }));
    provider.queue_response(Ok(valid_triage_response()));

    let registry = prompt_registry_arc();
    let dir = tempdir().unwrap();
    let config = make_config(provider_trait, registry, &dir);
    let handle = LlmHandle::new(config);

    handle.send(make_triage_command(4)).unwrap();

    match recv_event(&handle) {
        LlmEvent::Completed { result, .. } => {
            assert!(result.is_ok(), "should succeed after retrying HTTP 502");
        }
    }

    assert_eq!(
        provider.recorded_requests().len(),
        2,
        "should have made 2 provider calls (HTTP 502 retry)"
    );
}
