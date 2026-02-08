use std::sync::Arc;

use tempfile::tempdir;

use harvester_engine::llm::{
    LlmCommand, LlmConfig, LlmEvent, LlmHandle, LlmQuotas, MockLlmProvider, ModelId,
    PricingRegistry, PromptId, PromptRegistry, ProviderKind,
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
