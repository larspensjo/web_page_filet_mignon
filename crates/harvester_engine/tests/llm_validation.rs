use harvester_engine::llm::{
    validate_briefing, validate_summary, validate_triage, TriagePriority, ValidationError,
};

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
        ValidationError::FieldTooLong {
            field: "category",
            max_chars: 120,
            actual_chars: 125,
        }
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

#[test]
fn summary_limit_error_reports_actual_and_max() {
    let summary = "s".repeat(1201);
    let json = format!(
        r#"{{"title":"T","summary":"{}","key_points":["k1"]}}"#,
        summary
    );
    assert_eq!(
        validate_summary(&json).unwrap_err(),
        ValidationError::FieldTooLong {
            field: "summary",
            max_chars: 1200,
            actual_chars: 1201,
        }
    );
}

#[test]
fn summary_key_points_over_limit_are_truncated() {
    let long_point = "k".repeat(300);
    let json = format!(
        r#"{{"title":"T","summary":"S","key_points":["{}"]}}"#,
        long_point
    );

    let validated = validate_summary(&json).unwrap();
    assert_eq!(validated.key_points.len(), 1);
    assert_eq!(validated.key_points[0].chars().count(), 256);
}

#[test]
fn executive_summary_over_limit_is_truncated_with_notice() {
    let executive_summary = "e".repeat(3200);
    let json = format!(
        r#"{{
            "executive_summary":"{}",
            "top_stories":[{{"headline":"Story","body":"Description"}}],
            "article_count":1
        }}"#,
        executive_summary
    );

    let validated = validate_briefing(&json).unwrap();
    let chars = validated.executive_summary.chars().count();
    assert_eq!(chars, 3000);
    assert!(
        validated
            .executive_summary
            .contains("[Truncated response: removed"),
        "missing truncation notice in executive summary"
    );
}

#[test]
fn briefing_story_body_is_truncated_to_150_words() {
    let body = (1..=175)
        .map(|idx| format!("word{idx}"))
        .collect::<Vec<_>>()
        .join(" ");
    let json = format!(
        r#"{{
            "executive_summary":"Exec",
            "top_stories":[{{"headline":"Story","body":"{}"}}],
            "article_count":1
        }}"#,
        body
    );

    let validated = validate_briefing(&json).unwrap();
    assert_eq!(
        validated.top_stories[0].body.split_whitespace().count(),
        150
    );
    assert!(validated.top_stories[0].body.ends_with("..."));
}

#[test]
fn legacy_theme_briefing_is_mapped_to_story_schema() {
    let json = r#"{
        "executive_summary":"Exec",
        "themes":[{"name":"Theme","description":"Description"}],
        "article_count":1
    }"#;

    let validated = validate_briefing(json).unwrap();
    assert_eq!(validated.top_stories.len(), 1);
    assert_eq!(validated.top_stories[0].headline, "Theme");
    assert_eq!(validated.top_stories[0].body, "Description");
}
