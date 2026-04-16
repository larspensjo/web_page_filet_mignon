use std::fs;
use std::path::Path;

use harvester_engine::llm::{PromptId, PromptRegistry};
use harvester_engine::{
    build_markdown_document, compute_prompt_overhead, load_and_prepare_articles,
    load_and_prepare_articles_filtered, load_and_prepare_articles_filtered_with_progress,
    WhitespaceTokenCounter,
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
    let (articles, collection) =
        load_and_prepare_articles(tmp.path(), 10_000, &registry, None).unwrap();
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
        "body text sentinel",
    );

    let (articles, collection) =
        load_and_prepare_articles(tmp.path(), 10_000, &registry, None).unwrap();

    assert_eq!(articles.len(), 1);
    assert_eq!(articles[0].url, "https://example.com/1");
    assert!(collection.contains("body text sentinel"));
}

#[test]
fn non_md_files_are_skipped() {
    let registry = prompt_registry_with_defaults();
    let tmp = tempdir().unwrap();
    fs::write(tmp.path().join("note.txt"), "irrelevant").unwrap();

    let (articles, _) = load_and_prepare_articles(tmp.path(), 10_000, &registry, None).unwrap();
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

    let (articles, _) = load_and_prepare_articles(tmp.path(), 10_000, &registry, None).unwrap();
    assert!(articles.is_empty());
}

#[test]
fn files_without_frontmatter_are_skipped() {
    let registry = prompt_registry_with_defaults();
    let tmp = tempdir().unwrap();
    fs::write(tmp.path().join("orphan.md"), "just a body").unwrap();

    let (articles, _) = load_and_prepare_articles(tmp.path(), 10_000, &registry, None).unwrap();
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

    let (articles, _) = load_and_prepare_articles(tmp.path(), 10_000, &registry, None).unwrap();
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

    let (articles, _) = load_and_prepare_articles(tmp.path(), max_input, &registry, None).unwrap();
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

    let (_, collection) =
        load_and_prepare_articles(tmp.path(), max_input, &registry, None).unwrap();
    let collection_budget = max_input - briefing_overhead;
    assert!(collection.len() <= collection_budget);
}

#[test]
fn collection_limits_articles_when_budget_tight() {
    let mut registry = prompt_registry_with_defaults();
    // Pin prompt versions so this budget-shaping test does not depend on evolving defaults.
    registry.set_active(PromptId::ArticleSummary, 1);
    registry.set_active(PromptId::AggregateBriefing, 1);
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
    const TOTAL_ARTICLES: usize = 20;
    for i in 0..TOTAL_ARTICLES {
        write_markdown_file(
            tmp.path(),
            &format!("article_{i:02}.md"),
            &format!("https://example.com/{i:02}"),
            Some(&format!("Budget {i:02}")),
            &format!("body article {i:02}"),
        );
    }

    let (articles, collection) =
        load_and_prepare_articles(tmp.path(), max_input, &registry, None).unwrap();
    assert_eq!(articles.len(), TOTAL_ARTICLES);
    let selected_articles = (0..TOTAL_ARTICLES)
        .filter(|i| collection.contains(&format!("body article {i:02}")))
        .count();
    assert!(
        selected_articles >= 1,
        "collection should include at least one article"
    );
    assert!(
        selected_articles < TOTAL_ARTICLES,
        "collection should drop articles when budget is tight"
    );
    for i in 0..selected_articles {
        assert!(
            collection.contains(&format!("body article {i:02}")),
            "collection should keep the selected prefix"
        );
    }
    for i in selected_articles..TOTAL_ARTICLES {
        assert!(
            !collection.contains(&format!("body article {i:02}")),
            "collection should drop articles after the selected prefix"
        );
    }
}

#[test]
fn filtered_loader_includes_only_selected_urls() {
    let registry = prompt_registry_with_defaults();
    let tmp = tempdir().unwrap();
    write_markdown_file(
        tmp.path(),
        "a.md",
        "https://example.com/a",
        Some("A"),
        "body a",
    );
    write_markdown_file(
        tmp.path(),
        "b.md",
        "https://example.com/b",
        Some("B"),
        "body b",
    );

    let selected = vec!["https://example.com/b".to_string()];
    let (articles, _) =
        load_and_prepare_articles_filtered(tmp.path(), 10_000, &registry, &selected, None).unwrap();

    assert_eq!(articles.len(), 1);
    assert_eq!(articles[0].url, "https://example.com/b");
}

#[test]
fn filtered_loader_preserves_caller_order() {
    let registry = prompt_registry_with_defaults();
    let tmp = tempdir().unwrap();
    write_markdown_file(
        tmp.path(),
        "a.md",
        "https://example.com/a",
        Some("A"),
        "body a",
    );
    write_markdown_file(
        tmp.path(),
        "b.md",
        "https://example.com/b",
        Some("B"),
        "body b",
    );

    let selected = vec![
        "https://example.com/b".to_string(),
        "https://example.com/a".to_string(),
    ];
    let (articles, _) =
        load_and_prepare_articles_filtered(tmp.path(), 10_000, &registry, &selected, None).unwrap();

    assert_eq!(articles.len(), 2);
    assert_eq!(articles[0].url, "https://example.com/b");
    assert_eq!(articles[1].url, "https://example.com/a");
}

#[test]
fn filtered_loader_missing_selected_url_is_skipped() {
    let registry = prompt_registry_with_defaults();
    let tmp = tempdir().unwrap();
    write_markdown_file(
        tmp.path(),
        "a.md",
        "https://example.com/a",
        Some("A"),
        "body a",
    );

    let selected = vec![
        "https://example.com/missing".to_string(),
        "https://example.com/a".to_string(),
    ];
    let (articles, _) =
        load_and_prepare_articles_filtered(tmp.path(), 10_000, &registry, &selected, None).unwrap();

    assert_eq!(articles.len(), 1);
    assert_eq!(articles[0].url, "https://example.com/a");
}

#[test]
fn filtered_loader_budget_trimming_drops_tail_only() {
    let mut registry = prompt_registry_with_defaults();
    // Pin prompt versions so this budget-shaping test does not depend on evolving defaults.
    registry.set_active(PromptId::ArticleSummary, 1);
    registry.set_active(PromptId::AggregateBriefing, 1);
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
    let mut selected = Vec::new();
    for idx in 0..20 {
        let url = format!("https://example.com/{idx:02}");
        write_markdown_file(
            tmp.path(),
            &format!("article_{idx:02}.md"),
            &url,
            Some(&format!("Title {idx}")),
            &format!("body article {idx:02}"),
        );
        selected.push(url);
    }
    let (articles, collection) =
        load_and_prepare_articles_filtered(tmp.path(), max_input, &registry, &selected, None)
            .unwrap();

    assert_eq!(articles.len(), selected.len());
    let selected_articles = (0..selected.len())
        .filter(|idx| collection.contains(&format!("body article {idx:02}")))
        .count();
    assert!(selected_articles >= 1);
    assert!(selected_articles < selected.len());
    for idx in 0..selected_articles {
        assert!(
            collection.contains(&format!("body article {idx:02}")),
            "collection should keep the selected prefix"
        );
    }
    for idx in selected_articles..selected.len() {
        assert!(
            !collection.contains(&format!("body article {idx:02}")),
            "collection should drop the tail after budget trimming"
        );
    }
}

#[test]
fn filtered_loader_empty_selection_returns_empty_result() {
    let registry = prompt_registry_with_defaults();
    let tmp = tempdir().unwrap();
    write_markdown_file(
        tmp.path(),
        "a.md",
        "https://example.com/a",
        Some("A"),
        "body a",
    );

    let (articles, collection) =
        load_and_prepare_articles_filtered(tmp.path(), 10_000, &registry, &[], None).unwrap();

    assert!(articles.is_empty());
    assert!(collection.is_empty());
}

#[test]
fn filtered_loader_matches_www_and_eu_host_variants() {
    let registry = prompt_registry_with_defaults();
    let tmp = tempdir().unwrap();
    let article_url = "https://eu.detroitnews.com/story/business/2026/02/14/example/";
    write_markdown_file(
        tmp.path(),
        "detroit.md",
        article_url,
        Some("Detroit"),
        "body",
    );

    let selected =
        vec!["https://www.detroitnews.com/story/business/2026/02/14/example/".to_string()];
    let (articles, _) =
        load_and_prepare_articles_filtered(tmp.path(), 10_000, &registry, &selected, None).unwrap();

    assert_eq!(articles.len(), 1);
    assert_eq!(articles[0].url, article_url);
}

#[test]
fn filtered_loader_matches_normalized_url_shape() {
    let registry = prompt_registry_with_defaults();
    let tmp = tempdir().unwrap();
    write_markdown_file(
        tmp.path(),
        "a.md",
        "https://example.com/news/item",
        Some("Item"),
        "body",
    );

    let selected = vec!["HTTPS://EXAMPLE.COM:443/news/item/".to_string()];
    let (articles, _) =
        load_and_prepare_articles_filtered(tmp.path(), 10_000, &registry, &selected, None).unwrap();

    assert_eq!(articles.len(), 1);
    assert_eq!(articles[0].url, "https://example.com/news/item");
}

#[test]
fn filtered_loader_matches_mobile_and_query_variants() {
    let registry = prompt_registry_with_defaults();
    let tmp = tempdir().unwrap();
    let article_url = "https://www.economictimes.com/ai/story";
    write_markdown_file(tmp.path(), "economics.md", article_url, Some("ET"), "body");

    let selected = vec!["https://m.economictimes.com/ai/story?from=mdr".to_string()];
    let (articles, _) =
        load_and_prepare_articles_filtered(tmp.path(), 10_000, &registry, &selected, None).unwrap();

    assert_eq!(articles.len(), 1);
    assert_eq!(articles[0].url, article_url);
}

#[test]
fn filtered_loader_matches_http_https_and_edition_variants() {
    let registry = prompt_registry_with_defaults();
    let tmp = tempdir().unwrap();
    let cnn_url = "https://edition.cnn.com/2026/02/24/tech/example";
    write_markdown_file(tmp.path(), "cnn.md", cnn_url, Some("CNN"), "body");

    let selected = vec!["http://www.cnn.com/2026/02/24/tech/example".to_string()];
    let (articles, _) =
        load_and_prepare_articles_filtered(tmp.path(), 10_000, &registry, &selected, None).unwrap();

    assert_eq!(articles.len(), 1);
    assert_eq!(articles[0].url, cnn_url);
}

#[test]
fn filtered_loader_matches_cisco_content_path_alias() {
    let registry = prompt_registry_with_defaults();
    let tmp = tempdir().unwrap();
    let article_url = "https://newsroom.cisco.com/c/r/newsroom/en/us/a/y2026/m02/example.html";
    write_markdown_file(tmp.path(), "cisco.md", article_url, Some("Cisco"), "body");

    let selected = vec![
        "https://newsroom.cisco.com/content/r/newsroom/en/us/a/y2026/m02/example.html".to_string(),
    ];
    let (articles, _) =
        load_and_prepare_articles_filtered(tmp.path(), 10_000, &registry, &selected, None).unwrap();

    assert_eq!(articles.len(), 1);
    assert_eq!(articles[0].url, article_url);
}

#[test]
fn filtered_loader_single_selection_ignores_unrelated_invalid_markdown() {
    let registry = prompt_registry_with_defaults();
    let tmp = tempdir().unwrap();
    write_markdown_file(
        tmp.path(),
        "a.md",
        "https://example.com/a",
        Some("A"),
        "body a",
    );
    fs::write(tmp.path().join("z-invalid.md"), [0xFF, 0xFE, 0xFD]).unwrap();

    let selected = vec!["https://example.com/a".to_string()];
    let (articles, _) =
        load_and_prepare_articles_filtered(tmp.path(), 10_000, &registry, &selected, None).unwrap();

    assert_eq!(articles.len(), 1);
    assert_eq!(articles[0].url, "https://example.com/a");
}

/// Archive files (multi-doc format, starting with `===== DOC START =====`) live in the same
/// output directory as articles. The scan must skip them without preventing valid articles
/// from loading.
#[test]
fn archive_format_file_in_output_dir_does_not_block_article_scan() {
    let registry = prompt_registry_with_defaults();
    let tmp = tempdir().unwrap();
    write_markdown_file(
        tmp.path(),
        "article.md",
        "https://example.com/article",
        Some("Article"),
        "body text",
    );
    // Simulate an archive file: starts with the multi-doc separator, not a frontmatter block.
    let archive_content = "===== DOC START =====\n---\nurl: \"https://example.com/old\"\ntitle: \"Old\"\n---\n\nold body\n";
    fs::write(tmp.path().join("archive.md"), archive_content).unwrap();

    let (articles, _) = load_and_prepare_articles(tmp.path(), 10_000, &registry, None).unwrap();

    assert_eq!(articles.len(), 1);
    assert_eq!(articles[0].url, "https://example.com/article");
}

/// When `since_utc` is set and all selected URLs belong to articles older than the cutoff,
/// none of them appear in the date-filtered corpus. The function must return an empty result
/// without error — this is expected behaviour, not a data-integrity problem.
#[test]
fn filtered_loader_selected_urls_older_than_since_utc_produce_empty_result() {
    let registry = prompt_registry_with_defaults();
    let tmp = tempdir().unwrap();
    // FETCHED constant is "2026-02-09T00:00:00Z" — well before the cutoff below.
    write_markdown_file(
        tmp.path(),
        "old.md",
        "https://example.com/old",
        Some("Old"),
        "old body",
    );

    let since_utc: chrono::DateTime<chrono::Utc> = "2026-03-01T00:00:00Z"
        .parse::<chrono::DateTime<chrono::Utc>>()
        .unwrap();
    let selected = vec!["https://example.com/old".to_string()];
    let (articles, collection) = load_and_prepare_articles_filtered(
        tmp.path(),
        10_000,
        &registry,
        &selected,
        Some(since_utc),
    )
    .unwrap();

    assert!(articles.is_empty());
    assert!(collection.is_empty());
}

#[test]
fn filtered_loader_with_progress_reports_scan_progress() {
    let registry = prompt_registry_with_defaults();
    let tmp = tempdir().unwrap();
    write_markdown_file(
        tmp.path(),
        "first.md",
        "https://example.com/first",
        Some("First"),
        "first body",
    );
    write_markdown_file(
        tmp.path(),
        "second.md",
        "https://example.com/second",
        Some("Second"),
        "second body",
    );

    let mut progress = Vec::new();
    let selected = vec!["https://example.com/second".to_string()];
    let (articles, collection) = load_and_prepare_articles_filtered_with_progress(
        tmp.path(),
        10_000,
        &registry,
        &selected,
        None,
        |scan| progress.push(scan),
    )
    .unwrap();

    assert_eq!(articles.len(), 1);
    assert_eq!(articles[0].url, "https://example.com/second");
    assert!(collection.contains("second body"));
    assert!(!progress.is_empty());
    assert_eq!(progress[0].files_scanned, 1);
    assert_eq!(progress[0].files_total, 2);
    assert_eq!(progress.last().unwrap().files_scanned, 2);
    assert_eq!(progress.last().unwrap().files_total, 2);
}
