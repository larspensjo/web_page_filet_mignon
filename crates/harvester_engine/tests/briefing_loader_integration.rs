use std::fs;
use std::path::Path;

use harvester_engine::llm::{PromptId, PromptRegistry};
use harvester_engine::{
    build_markdown_document, compute_prompt_overhead, load_and_prepare_articles,
    token::WhitespaceTokenCounter,
};
use tempfile::tempdir;

const ENCODING: &str = "utf-8";
const FETCHED: &str = "2026-02-09T00:00:00Z";
const COLLECTION_MIN_BYTES: usize = 64;

fn write_markdown_file(
    dir: &Path,
    filename: &str,
    url: &str,
    title: Option<&str>,
    body: &str,
) -> std::path::PathBuf {
    let counter = WhitespaceTokenCounter;
    let (_, markdown) = build_markdown_document(url, title, ENCODING, FETCHED, body, &counter);
    let path = dir.join(filename);
    fs::write(&path, markdown).unwrap();
    path
}

fn prompt_registry_with_defaults() -> PromptRegistry {
    PromptRegistry::with_defaults()
}

#[test]
fn empty_directory_returns_no_articles() {
    let registry = prompt_registry_with_defaults();
    let tmp = tempdir().unwrap();
    let (articles, collection) = load_and_prepare_articles(tmp.path(), 10_000, &registry).unwrap();
    assert!(articles.is_empty());
    assert!(collection.is_empty());
}

#[test]
fn single_article_is_loaded_and_in_collection() {
    let registry = prompt_registry_with_defaults();
    let tmp = tempdir().unwrap();
    write_markdown_file(
        tmp.path(),
        "article.md",
        "https://example.com/1",
        Some("Title"),
        "body text",
    );

    let (articles, collection) = load_and_prepare_articles(tmp.path(), 10_000, &registry).unwrap();

    assert_eq!(articles.len(), 1);
    assert_eq!(articles[0].url, "https://example.com/1");
    assert!(collection.contains("--- Article 1"));
}

#[test]
fn non_md_files_are_skipped() {
    let registry = prompt_registry_with_defaults();
    let tmp = tempdir().unwrap();
    fs::write(tmp.path().join("note.txt"), "irrelevant").unwrap();

    let (articles, _) = load_and_prepare_articles(tmp.path(), 10_000, &registry).unwrap();
    assert!(articles.is_empty());
}

#[test]
fn linked_directory_is_not_scanned() {
    let registry = prompt_registry_with_defaults();
    let tmp = tempdir().unwrap();
    let linked = tmp.path().join("linked");
    fs::create_dir_all(&linked).unwrap();
    write_markdown_file(
        &linked,
        "article.md",
        "https://example.com/linked",
        None,
        "body",
    );

    let (articles, _) = load_and_prepare_articles(tmp.path(), 10_000, &registry).unwrap();
    assert!(articles.is_empty());
}

#[test]
fn files_without_frontmatter_are_skipped() {
    let registry = prompt_registry_with_defaults();
    let tmp = tempdir().unwrap();
    fs::write(tmp.path().join("orphan.md"), "just a body").unwrap();

    let (articles, _) = load_and_prepare_articles(tmp.path(), 10_000, &registry).unwrap();
    assert!(articles.is_empty());
}

#[test]
fn valid_files_do_not_prevent_others_from_loading() {
    let registry = prompt_registry_with_defaults();
    let tmp = tempdir().unwrap();
    write_markdown_file(
        tmp.path(),
        "good.md",
        "https://example.com/good",
        Some("Good"),
        "content",
    );
    fs::write(tmp.path().join("bad.md"), "no frontmatter").unwrap();

    let (articles, _) = load_and_prepare_articles(tmp.path(), 10_000, &registry).unwrap();
    assert_eq!(articles.len(), 1);
}

#[test]
fn prepared_text_is_within_summary_budget() {
    let registry = prompt_registry_with_defaults();
    let summary_template = registry
        .active(PromptId::ArticleSummary)
        .expect("summary prompt missing");
    let briefing_template = registry
        .active(PromptId::AggregateBriefing)
        .expect("briefing prompt missing");
    let summary_overhead = compute_prompt_overhead(summary_template, "content", &[]);
    let briefing_overhead = compute_prompt_overhead(briefing_template, "collection", &[]);
    let max_input = summary_overhead + briefing_overhead + 5_000;

    let tmp = tempdir().unwrap();
    write_markdown_file(
        tmp.path(),
        "article.md",
        "https://example.com/budget",
        Some("Budget"),
        "body",
    );

    let (articles, _) = load_and_prepare_articles(tmp.path(), max_input, &registry).unwrap();
    let summary_budget = max_input - summary_overhead;
    assert!(articles[0].prepared_text.len() <= summary_budget);
}

#[test]
fn collection_text_respects_collection_budget() {
    let registry = prompt_registry_with_defaults();
    let summary_template = registry
        .active(PromptId::ArticleSummary)
        .expect("summary prompt missing");
    let briefing_template = registry
        .active(PromptId::AggregateBriefing)
        .expect("briefing prompt missing");
    let summary_overhead = compute_prompt_overhead(summary_template, "content", &[]);
    let briefing_overhead = compute_prompt_overhead(briefing_template, "collection", &[]);
    let max_input = summary_overhead + briefing_overhead + 5_000;

    let tmp = tempdir().unwrap();
    write_markdown_file(
        tmp.path(),
        "article.md",
        "https://example.com/collection",
        Some("Collection"),
        "body",
    );

    let (_, collection) = load_and_prepare_articles(tmp.path(), max_input, &registry).unwrap();
    let collection_budget = max_input - briefing_overhead;
    assert!(collection.len() <= collection_budget);
}

#[test]
fn collection_limits_articles_when_budget_tight() {
    let registry = prompt_registry_with_defaults();
    let summary_template = registry
        .active(PromptId::ArticleSummary)
        .expect("summary prompt missing");
    let briefing_template = registry
        .active(PromptId::AggregateBriefing)
        .expect("briefing prompt missing");
    let summary_overhead = compute_prompt_overhead(summary_template, "content", &[]);
    let briefing_overhead = compute_prompt_overhead(briefing_template, "collection", &[]);
    let max_input = summary_overhead + briefing_overhead + COLLECTION_MIN_BYTES;

    let tmp = tempdir().unwrap();
    for i in 0..3 {
        write_markdown_file(
            tmp.path(),
            &format!("article_{}.md", i),
            &format!("https://example.com/{}", i),
            Some("Budget"),
            "body",
        );
    }

    let (articles, collection) =
        load_and_prepare_articles(tmp.path(), max_input, &registry).unwrap();
    assert_eq!(articles.len(), 3);
    assert_eq!(collection.matches("--- Article").count(), 1);
}
