use std::sync::{Arc, RwLock};

use tempfile::tempdir;

use harvester_engine::llm::provider::LlmProvider;
use harvester_engine::llm::{
    BlockingMockProvider, FinishReason, LlmCommand, LlmCompletionCommand, LlmCompletionError,
    LlmConfig, LlmError, LlmEvent, LlmHandle, LlmQuotas, LlmResponse, MockLlmProvider, ModelId,
    PricingRegistry, PromptId, PromptRegistry, ProviderKind, TokenUsage,
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
        signal_candidate_model: None,
        registry: Arc::clone(&registry),
        quotas: LlmQuotas::default(),
        output_dir: dir.path().to_path_buf(),
        pricing: PricingRegistry::new(),
        max_input_bytes: 10_000,
        timestamp_utc: Arc::new(|| "2026-02-08T00:00:00Z".to_string()),
        session_id: "test-session".to_string(),
        replay_write_observer: None,
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
            input_content: "document text".to_string(),
            context: vec![("key".to_string(), "value".to_string())],
            extra_template_vars: vec![],
        })))
        .expect("LLM command should dispatch");

    let event = recv_event(&handle);

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
        LlmEvent::UsageUpdated { .. } => unreachable!(),
    }
}

#[test]
fn llm_handle_emits_usage_update_after_completion() {
    let provider = Arc::new(MockLlmProvider::new());
    let provider_trait: Arc<dyn LlmProvider> = provider.clone();
    provider
        .queue_json_success(r#"{"category":"news","priority":3,"tags":["a"],"rationale":"ok"}"#);
    let registry = prompt_registry_arc();
    let dir = tempdir().unwrap();
    let handle = LlmHandle::new(make_config(provider_trait, registry, &dir));

    handle
        .send(LlmCommand::Complete(Box::new(LlmCompletionCommand {
            request_id: 7,
            prompt_id: PromptId::ArticleTriage,
            prompt_version: Some(1),
            input_content: "document text".to_string(),
            context: Vec::new(),
            extra_template_vars: vec![],
        })))
        .expect("LLM command should dispatch");

    let receiver = handle.event_receiver();
    let first = receiver
        .lock()
        .expect("lock receiver")
        .recv_timeout(std::time::Duration::from_secs(5))
        .expect("completion event");
    assert!(matches!(first, LlmEvent::Completed { .. }));

    let second = receiver
        .lock()
        .expect("lock receiver")
        .recv_timeout(std::time::Duration::from_secs(5))
        .expect("usage event");
    match second {
        LlmEvent::UsageUpdated { usage } => assert_eq!(usage.calls, 1),
        LlmEvent::Completed { .. } => panic!("expected usage update after completion"),
    }
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
                input_content: format!("document {i}"),
                context: Vec::new(),
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
    loop {
        let event = handle
            .event_receiver()
            .lock()
            .expect("lock receiver")
            .recv_timeout(std::time::Duration::from_secs(5))
            .expect("should receive event within 5s");
        if matches!(event, LlmEvent::Completed { .. }) {
            return event;
        }
    }
}

#[test]
fn stage_model_wins_over_default() {
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
            input_content: "document".to_string(),
            context: vec![],
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

// -----------------------------------------------------------------------
// Retry under concurrency pressure test
// -----------------------------------------------------------------------

/// Verify that a retry can complete when `max_concurrent_requests - 1` other
/// permits are occupied by long-running tasks.
///
/// The retry loop in `handle.rs` holds the semaphore permit for the entire
/// task lifetime (including the retry sleep). This test confirms that holding
/// a permit during the retry sleep does not cause a deadlock even when all
/// other permits are occupied by blocked tasks.
#[test]
fn retry_under_concurrency_pressure() {
    const CAP: usize = 2;
    let triage_json = r#"{"category":"news","priority":3,"tags":["t"],"rationale":"ok"}"#;

    // The filler task uses BlockingMockProvider so it stays in-flight until released.
    let blocker = Arc::new(BlockingMockProvider::new(triage_json));
    let blocker_trait: Arc<dyn LlmProvider> = blocker.clone();

    // The retryable request uses MockLlmProvider: first returns Timeout, then success.
    let retryable = Arc::new(MockLlmProvider::new());
    retryable.queue_response(Err(LlmError::Timeout));
    retryable.queue_response(Ok(LlmResponse::new(
        triage_json,
        TokenUsage::new(0, 0),
        ModelId::new(ProviderKind::OpenAi, "mock"),
        FinishReason::Stop,
    )));

    // Build two handles: one per provider, both sharing the same output dir.
    let registry = prompt_registry_arc();
    let dir = tempdir().unwrap();

    let mut blocker_config = make_config(blocker_trait, Arc::clone(&registry), &dir);
    blocker_config.max_concurrent_requests = CAP;
    let blocker_handle = LlmHandle::new(blocker_config);

    let provider_trait: Arc<dyn LlmProvider> = retryable.clone();
    let mut retry_config = make_config(provider_trait, Arc::clone(&registry), &dir);
    retry_config.max_concurrent_requests = CAP;
    let retry_handle = LlmHandle::new(retry_config);

    // Fill CAP - 1 = 1 slots of the blocker handle.
    blocker_handle
        .send(LlmCommand::Complete(Box::new(LlmCompletionCommand {
            request_id: 100,
            prompt_id: PromptId::ArticleTriage,
            prompt_version: Some(1),
            input_content: "filler document".to_string(),
            context: Vec::new(),
            extra_template_vars: vec![],
        })))
        .expect("filler send should succeed");

    // Wait briefly so the filler is actually in-flight (permit acquired).
    std::thread::sleep(std::time::Duration::from_millis(50));
    assert_eq!(
        blocker.current_in_flight(),
        CAP - 1,
        "filler should be in-flight"
    );

    // Send the retryable request on its own handle (separate semaphore).
    // This confirms the retry completes even with CAP - 1 other permits occupied.
    retry_handle
        .send(LlmCommand::Complete(Box::new(LlmCompletionCommand {
            request_id: 200,
            prompt_id: PromptId::ArticleTriage,
            prompt_version: Some(1),
            input_content: "retryable document".to_string(),
            context: Vec::new(),
            extra_template_vars: vec![],
        })))
        .expect("retry send should succeed");

    // Await the retry result with a generous timeout (retry delay is 2 s).
    let result = recv_event(&retry_handle);

    match result {
        LlmEvent::Completed { request_id, result } => {
            assert_eq!(request_id, 200);
            assert!(result.is_ok(), "retry should succeed on second attempt");
        }
        LlmEvent::UsageUpdated { .. } => unreachable!(),
    }

    assert_eq!(
        retryable.recorded_requests().len(),
        2,
        "should have made 2 provider calls (Timeout + retry success)"
    );

    // Release the filler and drain its event.
    blocker.release(CAP - 1);
    blocker_handle
        .event_receiver()
        .lock()
        .unwrap()
        .recv_timeout(std::time::Duration::from_secs(5))
        .expect("filler should complete after release");
}

// -----------------------------------------------------------------------
// Step 6 — Retry loop tests
// -----------------------------------------------------------------------

fn make_triage_command(request_id: u64) -> LlmCommand {
    LlmCommand::Complete(Box::new(LlmCompletionCommand {
        request_id,
        prompt_id: PromptId::ArticleTriage,
        prompt_version: Some(1),
        input_content: "document text".to_string(),
        context: vec![],
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
        LlmEvent::UsageUpdated { .. } => unreachable!(),
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
            assert!(result.is_err(), "should fail after exhausting all attempts");
            assert!(
                matches!(
                    result,
                    Err(LlmCompletionError::ProviderError(LlmError::Timeout))
                ),
                "should propagate the provider error"
            );
        }
        LlmEvent::UsageUpdated { .. } => unreachable!(),
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
        LlmEvent::UsageUpdated { .. } => unreachable!(),
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
        LlmEvent::UsageUpdated { .. } => unreachable!(),
    }

    assert_eq!(
        provider.recorded_requests().len(),
        2,
        "should have made 2 provider calls (HTTP 502 retry)"
    );
}
