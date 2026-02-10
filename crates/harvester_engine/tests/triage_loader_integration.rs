use std::fs;
use std::path::Path;

use harvester_engine::llm::{PromptId, PromptRegistry};
use harvester_engine::{
    build_markdown_document, compute_prompt_overhead, load_and_prepare_articles,
    load_and_prepare_articles_for_triage, WhitespaceTokenCounter,
};
use tempfile::tempdir;

const ENCODING: &str = "utf-8";
const FETCHED: &str = "2026-02-09T00:00:00Z";

fn write_markdown_file(dir: &Path, filename: &str, url: &str, title: Option<&str>, body: &str) {
    let counter = WhitespaceTokenCounter;
    let (_, markdown) = build_markdown_document(url, title, ENCODING, FETCHED, body, &counter);
    fs::write(dir.join(filename), markdown).unwrap();
}

fn prompt_registry_with_defaults() -> PromptRegistry {
    PromptRegistry::with_defaults()
}

#[test]
fn triage_loader_respects_triage_budget() {
    let registry = prompt_registry_with_defaults();
    let triage_template = registry
        .active(PromptId::ArticleTriage)
        .expect("triage prompt missing");
    let triage_overhead = compute_prompt_overhead(triage_template, "content", &[]);
    let triage_budget = 1_000;
    let max_input = triage_overhead + triage_budget;

    let tmp = tempdir().unwrap();
    write_markdown_file(
        tmp.path(),
        "article.md",
        "https://example.com/triage",
        Some("Title"),
        "content",
    );

    let articles = load_and_prepare_articles_for_triage(tmp.path(), max_input, &registry).unwrap();
    assert_eq!(articles.len(), 1);
    let loaded = &articles[0];
    assert!(loaded.prepared_text.len() <= triage_budget);
}

#[test]
fn triage_loader_shared_scanning_matches_briefing() {
    let registry = prompt_registry_with_defaults();
    let tmp = tempdir().unwrap();
    write_markdown_file(
        tmp.path(),
        "good.md",
        "https://example.com/good",
        Some("Good"),
        "body",
    );
    fs::write(tmp.path().join("bad.txt"), "ignore me").unwrap();

    let briefing_articles = load_and_prepare_articles(tmp.path(), 100_000, &registry)
        .unwrap()
        .0;
    let triage_articles =
        load_and_prepare_articles_for_triage(tmp.path(), 100_000, &registry).unwrap();
    let briefing_urls: Vec<_> = briefing_articles
        .iter()
        .map(|article| article.url.clone())
        .collect();
    let triage_urls: Vec<_> = triage_articles
        .iter()
        .map(|article| article.url.clone())
        .collect();
    assert_eq!(briefing_urls, triage_urls);
}

#[test]
fn triage_loader_truncates_at_utf8_boundary() {
    let registry = prompt_registry_with_defaults();
    let triage_template = registry
        .active(PromptId::ArticleTriage)
        .expect("triage prompt missing");
    let triage_overhead = compute_prompt_overhead(triage_template, "content", &[]);
    let max_input = triage_overhead + 5;

    let tmp = tempdir().unwrap();
    write_markdown_file(
        tmp.path(),
        "unicode.md",
        "https://example.com/u",
        None,
        "ééééé",
    );

    let articles = load_and_prepare_articles_for_triage(tmp.path(), max_input, &registry).unwrap();
    assert_eq!(articles.len(), 1);
    let prepared = &articles[0].prepared_text;
    assert!(prepared.is_char_boundary(prepared.len()));
    assert!(prepared.len() <= 5);
}

#[test]
fn triage_loader_mapping_preserves_fields() {
    let registry = prompt_registry_with_defaults();
    let tmp = tempdir().unwrap();
    write_markdown_file(
        tmp.path(),
        "article.md",
        "https://example.com/mapping",
        Some("Mapping"),
        "body text",
    );

    let articles = load_and_prepare_articles_for_triage(tmp.path(), 100_000, &registry).unwrap();
    assert_eq!(articles.len(), 1);
    let article = &articles[0];
    assert_eq!(article.url, "https://example.com/mapping");
    assert_eq!(article.source_title.as_deref(), Some("Mapping"));
    assert!(!article.prepared_text.is_empty());
    assert!(!article.content_hash.is_empty());
}
