use engine_logging::initialize_for_tests;
use harvester_engine::llm::prompt_context::{
    load_context_file, validate_context_covers_template, ContextLoadError, ContextMeta,
    PromptContextFile,
};
use std::collections::HashMap;
use std::fs;
use std::path::PathBuf;
use tempfile::TempDir;

#[test]
fn load_context_file_parses_valid_toml() {
    initialize_for_tests();
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("test.toml");

    fs::write(
        &path,
        r#"
[meta]
prompt_id = "ArticleTriage"
schema_version = 1
version = 1
updated = "2025-01-01"
description = "Test context"

[variables]
holdings = "AAPL, MSFT"
themes = "AI, Cloud Computing"
"#,
    )
    .unwrap();

    let result = load_context_file(&path).expect("should load valid TOML");
    assert_eq!(result.meta.prompt_id, "ArticleTriage");
    assert_eq!(result.meta.schema_version, 1);
    assert_eq!(result.meta.version, 1);
    assert_eq!(result.variables.get("holdings").unwrap(), "AAPL, MSFT");
    assert_eq!(
        result.variables.get("themes").unwrap(),
        "AI, Cloud Computing"
    );
}

#[test]
fn load_context_file_rejects_missing_file() {
    initialize_for_tests();
    let path = PathBuf::from("/nonexistent/path.toml");
    let result = load_context_file(&path);
    assert!(matches!(result, Err(ContextLoadError::Io { .. })));
}

#[test]
fn load_context_file_rejects_invalid_toml() {
    initialize_for_tests();
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("bad.toml");

    fs::write(&path, "not valid toml [[").unwrap();

    let result = load_context_file(&path);
    assert!(matches!(result, Err(ContextLoadError::Parse { .. })));
}

#[test]
fn load_context_file_rejects_unknown_prompt_id() {
    initialize_for_tests();
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("unknown.toml");

    fs::write(
        &path,
        r#"
[meta]
prompt_id = "UnknownPrompt"
schema_version = 1
version = 1
updated = "2025-01-01"

[variables]
key = "value"
"#,
    )
    .unwrap();

    let result = load_context_file(&path);
    assert!(matches!(
        result,
        Err(ContextLoadError::UnknownPromptId { .. })
    ));
}

#[test]
fn load_context_file_rejects_wrong_schema_version() {
    initialize_for_tests();
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("wrong_schema.toml");

    fs::write(
        &path,
        r#"
[meta]
prompt_id = "ArticleTriage"
schema_version = 999
version = 1
updated = "2025-01-01"

[variables]
key = "value"
"#,
    )
    .unwrap();

    let result = load_context_file(&path);
    assert!(matches!(
        result,
        Err(ContextLoadError::UnknownSchemaVersion { .. })
    ));
}

#[test]
fn validate_detects_missing_context_variables() {
    initialize_for_tests();
    let template = "Hello {{name}}, your age is {{age}}.";
    let mut vars = HashMap::new();
    vars.insert("name".to_string(), "Alice".to_string());
    let context = PromptContextFile {
        meta: ContextMeta {
            prompt_id: "ArticleTriage".to_string(),
            schema_version: 1,
            version: 1,
            updated: "2025-01-01".to_string(),
            description: None,
            changelog: None,
        },
        variables: vars,
    };
    let known_runtime = &["content", "collection"];

    let (missing, _unused) = validate_context_covers_template(template, &context, known_runtime);

    assert_eq!(missing.len(), 1);
    assert!(missing.contains(&"age".to_string()));
}

#[test]
fn validate_ignores_known_runtime_vars() {
    initialize_for_tests();
    let template = "Content: {{content}}, Collection: {{collection}}, Custom: {{custom}}";
    let mut vars = HashMap::new();
    vars.insert("custom".to_string(), "value".to_string());
    let context = PromptContextFile {
        meta: ContextMeta {
            prompt_id: "ArticleTriage".to_string(),
            schema_version: 1,
            version: 1,
            updated: "2025-01-01".to_string(),
            description: None,
            changelog: None,
        },
        variables: vars,
    };
    let known_runtime = &["content", "collection"];

    let (missing, _unused) = validate_context_covers_template(template, &context, known_runtime);

    // content and collection should be ignored
    assert!(missing.is_empty());
}

#[test]
fn validate_detects_unused_context_variables() {
    initialize_for_tests();
    let template = "Hello {{name}}";
    let mut vars = HashMap::new();
    vars.insert("name".to_string(), "Alice".to_string());
    vars.insert("unused_key".to_string(), "value".to_string());
    let context = PromptContextFile {
        meta: ContextMeta {
            prompt_id: "ArticleTriage".to_string(),
            schema_version: 1,
            version: 1,
            updated: "2025-01-01".to_string(),
            description: None,
            changelog: None,
        },
        variables: vars,
    };
    let known_runtime = &["content"];

    let (_missing, unused) = validate_context_covers_template(template, &context, known_runtime);

    assert_eq!(unused.len(), 1);
    assert!(unused.contains(&"unused_key".to_string()));
}

#[test]
fn validate_accepts_fully_covered_template() {
    initialize_for_tests();
    let template = "Name: {{name}}, Content: {{content}}";
    let mut vars = HashMap::new();
    vars.insert("name".to_string(), "Bob".to_string());
    let context = PromptContextFile {
        meta: ContextMeta {
            prompt_id: "ArticleTriage".to_string(),
            schema_version: 1,
            version: 1,
            updated: "2025-01-01".to_string(),
            description: None,
            changelog: None,
        },
        variables: vars,
    };
    let known_runtime = &["content"];

    let (missing, unused) = validate_context_covers_template(template, &context, known_runtime);

    assert!(missing.is_empty());
    assert!(unused.is_empty());
}
