use std::sync::{Arc, RwLock};

use serde_json::json;
use tempfile::tempdir;

use harvester_engine::llm::provider::LlmProvider;
use harvester_engine::llm::{
    content_hash, BlockingMockProvider, LlmCommand, LlmConfig, LlmEvent, LlmHandle, LlmQuotas,
    MockLlmProvider, ModelId, PricingRegistry, PromptId, PromptRegistry, ProviderKind,
    ReplayProvider, ReplayRecord, TokenUsage,
};

fn make_config(
    provider_trait: Arc<dyn LlmProvider>,
    registry: PromptRegistry,
    dir: &tempfile::TempDir,
) -> LlmConfig {
    LlmConfig {
        provider: provider_trait,
        default_model: ModelId::new(ProviderKind::OpenAi, "mock"),
        triage_model: None,
        summary_model: None,
        briefing_model: None,
        registry,
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

#[test]
fn llm_handle_dispatches_completion_event() {
    let provider = Arc::new(MockLlmProvider::new());
    let provider_trait: Arc<dyn LlmProvider> = provider.clone();
    provider.queue_json_success(
        r#"{"category":"news","priority":3,"tags":["alpha","beta"],"rationale":"ok"}"#,
    );

    let registry = PromptRegistry::with_defaults();
    let dir = tempdir().unwrap();

    let config = make_config(provider_trait, registry, &dir);

    let handle = LlmHandle::new(config);
    handle
        .send(LlmCommand::Complete {
            request_id: 7,
            prompt_id: PromptId::ArticleTriage,
            prompt_version: Some(1),
            input_content: "document text".to_string(),
            context: vec![("key".to_string(), "value".to_string())],
        })
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
    let registry = PromptRegistry::with_defaults();
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
    });
    let replay_cache = Arc::new(RwLock::new(replay_provider));

    let mut config = make_config(provider_trait, registry, &dir);
    config.replay_cache = Some(replay_cache.clone());

    let handle = LlmHandle::new(config);
    handle
        .send(LlmCommand::Complete {
            request_id: 7,
            prompt_id: PromptId::ArticleSummary,
            prompt_version: Some(2),
            input_content: input_content.to_string(),
            context: Vec::new(),
        })
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
    let registry = PromptRegistry::with_defaults();
    let dir = tempdir().unwrap();
    let replay_cache = Arc::new(RwLock::new(ReplayProvider::new()));

    let mut config = make_config(provider_trait, registry.clone(), &dir);
    config.replay_cache = Some(Arc::clone(&replay_cache));

    let handle = LlmHandle::new(config);
    let input_content = "fresh document";

    handle
        .send(LlmCommand::Complete {
            request_id: 7,
            prompt_id: PromptId::ArticleSummary,
            prompt_version: Some(3),
            input_content: input_content.to_string(),
            context: Vec::new(),
        })
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
        .send(LlmCommand::Complete {
            request_id: 8,
            prompt_id: PromptId::ArticleSummary,
            prompt_version: Some(3),
            input_content: input_content.to_string(),
            context: Vec::new(),
        })
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

    let version = registry
        .active(PromptId::ArticleSummary)
        .expect("summary prompt missing")
        .version;
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
    let registry = PromptRegistry::with_defaults();
    let dir = tempdir().unwrap();

    let mut config = make_config(provider_trait, registry, &dir);
    config.max_concurrent_requests = cap;

    let handle = LlmHandle::new(config);

    // Send all requests.
    for i in 0..total_requests {
        handle
            .send(LlmCommand::Complete {
                request_id: i as u64 + 1,
                prompt_id: PromptId::ArticleTriage,
                prompt_version: Some(1),
                input_content: format!("document {i}"),
                context: Vec::new(),
            })
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
