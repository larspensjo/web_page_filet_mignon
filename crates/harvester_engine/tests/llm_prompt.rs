use harvester_engine::llm::prompt::{content_nonce, TemplateVars};
use harvester_engine::llm::{PromptId, PromptRegistry};

#[test]
fn registry_with_defaults_has_restart_scope() {
    let registry = PromptRegistry::with_defaults();
    assert_eq!(registry.active(PromptId::ArticleTriage).unwrap().version, 2);
    assert_eq!(registry.versions(PromptId::ArticleSummary).len(), 2);
    assert_eq!(registry.versions(PromptId::AggregateBriefing).len(), 2);
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
    let rendered = template.user_template.replace("{{content}}", &rendered_content);
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
