use harvester_engine::llm::{
    content_hash, load_replay_record, persist_replay_record, PromptId, ReplayProvider,
    ReplayRecord, TokenUsage,
};
use harvester_engine::llm::run_metadata::LlmRunMetadata;
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
        wall_ms: 150,
        cache_status: "miss".to_string(),
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

#[test]
fn llm_run_metadata_old_artifact_cached_input_tokens_defaults_to_zero() {
    // JSON without `cached_input_tokens` (pre-Step-5 artifact format).
    let json = r#"{
        "prompt_id": "ArticleTriage",
        "prompt_version": 1,
        "resolved_model": "gpt-4o-mini",
        "input_bytes": 100,
        "input_tokens": 10,
        "output_tokens": 20,
        "cost_microdollars": 300,
        "wall_ms": 50,
        "parse_ok": true,
        "validation_error": null,
        "cache_status": "miss",
        "timestamp_utc": "2026-01-01T00:00:00Z"
    }"#;
    let metadata: LlmRunMetadata = serde_json::from_str(json).expect("should deserialize");
    assert_eq!(metadata.cached_input_tokens, 0);
}

#[test]
fn replay_record_old_artifact_deserializes_with_defaults() {
    // JSON without wall_ms and cache_status (pre-Step-2 artifact format).
    let json = r#"{
        "request_id": "old-session",
        "input_content_hash": "abc123",
        "prompt_id": "ArticleTriage",
        "prompt_version": 1,
        "model_id": "openai::gpt-4o-mini",
        "timestamp_utc": "2025-01-01T00:00:00Z",
        "rendered_system_message": "",
        "rendered_user_message": "",
        "raw_response": "{}",
        "usage": {"input_tokens": 10, "output_tokens": 5},
        "validated_output": null,
        "validation_error": null,
        "cost_microdollars": 42
    }"#;
    let record: ReplayRecord = serde_json::from_str(json).expect("should deserialize");
    assert_eq!(record.wall_ms, 0);
    assert_eq!(record.cache_status, "miss");
}

#[test]
fn replay_record_roundtrip_new_fields() {
    let dir = tempdir().unwrap();
    let mut record = mock_record("session--4");
    record.wall_ms = 250;
    record.cache_status = "hit_validated".to_string();
    let path = persist_replay_record(dir.path(), &record).unwrap();
    let loaded = load_replay_record(&path).unwrap();
    assert_eq!(loaded.wall_ms, 250);
    assert_eq!(loaded.cache_status, "hit_validated");
}
