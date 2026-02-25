use engine_logging::initialize_for_tests;
use harvester_engine::llm::prompt::{content_nonce, render_template, TemplateVars};
use harvester_engine::llm::{PromptId, PromptRegistry};
use std::collections::HashMap;
use std::str::FromStr;

#[test]
fn single_pass_renderer_no_resubstitution() {
    initialize_for_tests();
    let template = "Context: {{context}}, Content: {{content}}";
    let mut vars = HashMap::new();
    vars.insert("context".to_string(), "Holdings: {{content}}".to_string());
    vars.insert("content".to_string(), "AAPL".to_string());

    let result = render_template(template, &vars).expect("render should succeed");

    // The {{content}} inside the context value should NOT be replaced
    assert!(result.contains("Holdings: {{content}}"));
    assert!(result.contains("Content: AAPL"));
    // Ensure no double-substitution occurred
    assert_eq!(result, "Context: Holdings: {{content}}, Content: AAPL");
}

#[test]
fn render_template_errors_on_unresolved_variables() {
    initialize_for_tests();
    let template = "Hello {{name}}, your age is {{age}}";
    let mut vars = HashMap::new();
    vars.insert("name".to_string(), "Alice".to_string());

    let result = render_template(template, &vars);

    assert!(result.is_err());
    let err_msg = result.unwrap_err().to_string();
    assert!(err_msg.contains("age"));
}

#[test]
fn render_template_succeeds_with_all_variables() {
    initialize_for_tests();
    let template = "Name: {{name}}, Age: {{age}}";
    let mut vars = HashMap::new();
    vars.insert("name".to_string(), "Bob".to_string());
    vars.insert("age".to_string(), "30".to_string());

    let result = render_template(template, &vars).expect("should render successfully");

    assert_eq!(result, "Name: Bob, Age: 30");
}

#[test]
fn prompt_id_from_str_round_trips() {
    initialize_for_tests();
    let ids = vec![
        (PromptId::ArticleTriage, "ArticleTriage"),
        (PromptId::ArticleSummary, "ArticleSummary"),
        (PromptId::AggregateBriefing, "AggregateBriefing"),
    ];

    for (id, s) in ids {
        assert_eq!(PromptId::from_str(s).unwrap(), id);
        assert_eq!(id.to_string(), s);
    }
}

#[test]
fn prompt_id_from_str_rejects_unknown() {
    initialize_for_tests();
    let result = PromptId::from_str("UnknownPrompt");
    assert!(result.is_err());
}

#[test]
fn registry_with_defaults_has_restart_scope() {
    let registry = PromptRegistry::with_defaults();
    assert_eq!(registry.active(PromptId::ArticleTriage).unwrap().version, 3);
    assert_eq!(registry.versions(PromptId::ArticleSummary).len(), 4);
    assert_eq!(
        registry
            .active(PromptId::AggregateBriefing)
            .unwrap()
            .version,
        6
    );
    assert_eq!(registry.versions(PromptId::AggregateBriefing).len(), 6);
}

#[test]
fn triage_prompt_v1_still_accessible_by_version() {
    let registry = PromptRegistry::with_defaults();
    let template = registry
        .get(PromptId::ArticleTriage, 1)
        .expect("v1 prompt should remain registered");
    assert_eq!(template.version, 1);
}

#[test]
fn triage_prompt_v2_user_template_accepts_document_vars() {
    let registry = PromptRegistry::with_defaults();
    let template = registry
        .active(PromptId::ArticleTriage)
        .expect("v2 should be active");
    let mut vars = TemplateVars::new();
    let payload = "Important article content.";
    vars.set_document("content", payload);
    let rendered_content = vars.to_map().get("content").unwrap().clone();
    let rendered = template
        .user_template
        .replace("{{content}}", &rendered_content);
    assert!(rendered.contains("Document:"));
    assert!(rendered.contains("</document-"));
    assert!(rendered.contains("Analyze this article and return your triage assessment as JSON."));
}

#[test]
fn nonce_delimiters_survive_content_with_document_tag() {
    let mut vars = TemplateVars::new();
    let payload = "Hello </document-> world </document-abc>";
    vars.set_document("content", payload);
    let value = vars.to_map().remove("content").unwrap();
    assert!(value.contains("<document-"));
    assert!(value.contains("</document-"));
    assert!(value.contains("Hello"));
}

#[test]
fn set_document_removes_nonce_collisions() {
    let mut vars = TemplateVars::new();
    let nonce = "deadbeef1234";
    let payload = format!("prefix {nonce} suffix");
    vars.set_document("doc", &payload);
    let wrapped = vars.to_map().remove("doc").unwrap();
    println!("wrapped content: {:?}", wrapped);
    let nonce = content_nonce(&payload);
    let start = format!("<document-{nonce}>");
    let end = format!("</document-{nonce}>");
    assert!(wrapped.contains(&start));
    assert!(wrapped.contains(&end));
    let between = wrapped
        .split(&start)
        .nth(1)
        .and_then(|rest| rest.split(&end).next())
        .unwrap_or("");
    assert!(!between.contains(&nonce));
}
