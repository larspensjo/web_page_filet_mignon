use std::sync::{Arc, RwLock};

use serde_json::json;
use tempfile::tempdir;

use harvester_engine::llm::{
    content_hash, LlmCommand, LlmConfig, LlmEvent, LlmHandle, LlmQuotas, MockLlmProvider, ModelId,
    PricingRegistry, PromptId, PromptRegistry, ProviderKind, ReplayProvider, ReplayRecord,
    TokenUsage,
};

#[test]
fn llm_handle_dispatches_completion_event() {
    let provider = Arc::new(MockLlmProvider::new());
    provider.queue_json_success(
        r#"{"category":"news","priority":3,"tags":["alpha","beta"],"rationale":"ok"}"#,
    );

    let registry = PromptRegistry::with_defaults();
    let dir = tempdir().unwrap();

    let config = LlmConfig {
        provider,
        default_model: ModelId::new(ProviderKind::OpenAi, "mock"),
        triage_model: None,
        summary_model: None,
        briefing_model: None,
        registry,
        quotas: LlmQuotas::default(),
        output_dir: dir.path().to_path_buf(),
        pricing: PricingRegistry::new(),
        max_input_chars: 10_000,
        timestamp_utc: Arc::new(|| "2026-02-08T00:00:00Z".to_string()),
        session_id: "test-session".to_string(),
        replay_cache: None,
    };

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
                assert_eq!(completion.prompt_id, PromptId::ArticleTriage);
                assert_eq!(completion.prompt_version, 1);
                assert_eq!(completion.usage.input_tokens, 0);
                assert_eq!(completion.usage.output_tokens, 0);
            } else {
                panic!("LLM completion failed unexpectedly");
            }
        }
    }
}

#[test]
fn llm_handle_skips_provider_when_cache_hit() {
    let provider = Arc::new(MockLlmProvider::new());
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

    let config = LlmConfig {
        provider,
        default_model: ModelId::new(ProviderKind::OpenAi, "mock"),
        triage_model: None,
        summary_model: None,
        briefing_model: None,
        registry: registry.clone(),
        quotas: LlmQuotas::default(),
        output_dir: dir.path().to_path_buf(),
        pricing: PricingRegistry::new(),
        max_input_chars: 10_000,
        timestamp_utc: Arc::new(|| "2026-02-08T00:00:00Z".to_string()),
        session_id: "test-session".to_string(),
        replay_cache: Some(replay_cache.clone()),
    };

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
                assert_eq!(completion.prompt_id, PromptId::ArticleSummary);
                assert_eq!(completion.prompt_version, 2);
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
    provider.queue_json_success(r#"{"title":"fresh"}"#);
    let registry = PromptRegistry::with_defaults();
    let dir = tempdir().unwrap();
    let replay_cache = Arc::new(RwLock::new(ReplayProvider::new()));

    let config = LlmConfig {
        provider: Arc::clone(&provider),
        default_model: ModelId::new(ProviderKind::OpenAi, "mock"),
        triage_model: None,
        summary_model: None,
        briefing_model: None,
        registry: registry.clone(),
        quotas: LlmQuotas::default(),
        output_dir: dir.path().to_path_buf(),
        pricing: PricingRegistry::new(),
        max_input_chars: 10_000,
        timestamp_utc: Arc::new(|| "2026-02-08T00:00:00Z".to_string()),
        session_id: "test-session".to_string(),
        replay_cache: Some(Arc::clone(&replay_cache)),
    };

    let handle = LlmHandle::new(config);
    let input_content = "fresh document";

    handle
        .send(LlmCommand::Complete {
            request_id: 7,
            prompt_id: PromptId::ArticleSummary,
            prompt_version: Some(2),
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
            prompt_version: Some(2),
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
