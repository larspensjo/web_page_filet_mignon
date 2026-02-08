use harvester_engine::llm::{validate_triage, TriagePriority, ValidationError};

#[test]
fn valid_triage_json_parses() {
    let json = r#"{
        "category": "news",
        "priority": 2,
        "tags": ["rust", "ai"],
        "rationale": "High volume candidate"
    }"#;
    let result = validate_triage(json).unwrap();
    assert_eq!(result.category, "news");
    assert_eq!(result.priority.value(), 2);
    assert_eq!(result.tags, vec!["rust", "ai"]);
}

#[test]
fn missing_fields_are_rejected() {
    let json = r#"{"category": "news", "tags": [], "rationale": "missing priority"}"#;
    assert_eq!(
        validate_triage(json).unwrap_err(),
        ValidationError::MissingField("priority")
    );
}

#[test]
fn out_of_range_priority_rejected() {
    let json = r#"{
        "category": "news",
        "priority": 10,
        "tags": [],
        "rationale": "priority out of bounds"
    }"#;
    assert_eq!(
        validate_triage(json).unwrap_err(),
        ValidationError::ValueOutOfRange("priority")
    );
}

#[test]
fn too_many_tags_rejected() {
    let tags = (0..13)
        .map(|i| format!(r#""tag{}""#, i))
        .collect::<Vec<_>>()
        .join(",");
    let json = format!(
        r#"{{"category": "news", "priority": 3, "tags": [{}], "rationale": "too many tags"}}"#,
        tags
    );
    assert_eq!(
        validate_triage(&json).unwrap_err(),
        ValidationError::ValueOutOfRange("tags")
    );
}

#[test]
fn oversized_fields_are_rejected() {
    let category = "x".repeat(125);
    let json = format!(
        r#"{{"category": "{}", "priority": 3, "tags": [], "rationale": "ok"}}"#,
        category
    );
    assert_eq!(
        validate_triage(&json).unwrap_err(),
        ValidationError::FieldTooLong("category")
    );
}

#[test]
fn non_json_is_rejected() {
    assert!(matches!(
        validate_triage("not a json"),
        Err(ValidationError::InvalidJson(_))
    ));
}

#[test]
fn triage_priority_range_is_enforced() {
    assert!(TriagePriority::new(3).is_some());
    assert!(TriagePriority::new(0).is_none());
    assert!(TriagePriority::new(6).is_none());
}
