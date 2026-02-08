use harvester_engine::llm::{
    content_hash, load_replay_record, persist_replay_record, PromptId, ReplayProvider,
    ReplayRecord, TokenUsage,
};
use serde_json::json;
use tempfile::tempdir;

fn mock_record(request_id: &str) -> ReplayRecord {
    ReplayRecord {
        request_id: request_id.to_string(),
        input_content_hash: content_hash("evaluation payload"),
        prompt_id: PromptId::ArticleTriage,
        prompt_version: 1,
        model_id: "openai::gpt-4".to_string(),
        timestamp_utc: "2026-02-08T00:00:00Z".to_string(),
        rendered_system_message: "system".to_string(),
        rendered_user_message: "user".to_string(),
        raw_response: "{\"priority\":2}".to_string(),
        usage: TokenUsage::new(10, 20),
        validated_output: Some(json!({"priority": 2})),
        validation_error: None,
        cost_microdollars: 250,
    }
}

#[test]
fn content_hash_is_consistent() {
    assert_eq!(
        "49ebf8b74e35f7c3364b01aecebe16c304c0e46c12fbe9f0549329d8957e2f8a",
        content_hash("hello replay")
    );
}

#[test]
fn persist_and_load_roundtrip() {
    let dir = tempdir().unwrap();
    let record = mock_record("session--1");
    let path = persist_replay_record(dir.path(), &record).unwrap();
    let loaded = load_replay_record(&path).unwrap();
    assert_eq!(record, loaded);
}

#[test]
fn replay_provider_loads_and_finds_record() {
    let dir = tempdir().unwrap();
    let record = mock_record("session--2");
    persist_replay_record(dir.path(), &record).unwrap();
    let provider = ReplayProvider::load_from_dir(dir.path()).unwrap();
    let fetched = provider
        .lookup(
            &record.input_content_hash,
            record.prompt_id,
            record.prompt_version,
        )
        .cloned()
        .unwrap();
    assert_eq!(record, fetched);
}

#[test]
fn persist_appends_suffix_when_collision() {
    let dir = tempdir().unwrap();
    let record = mock_record("session--3");
    let first = persist_replay_record(dir.path(), &record).unwrap();
    let second = persist_replay_record(dir.path(), &record).unwrap();
    assert_ne!(first, second);
    assert!(first.exists());
    assert!(second.exists());
}
