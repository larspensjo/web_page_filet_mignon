use harvester_engine::{
    compute_prompt_overhead, derive_clean_text,
    llm::{prompt::TemplateVars, prompts::SUMMARY_PROMPT, replay::content_hash},
    BoilerplatePolicy, ContentBudget, ContentPrepConfig, NormalizationPolicy, PreparedCollection,
    PreparedInput, WhitespaceTokenCounter,
};
use std::{collections::HashMap, fs, path::Path, sync::Arc};

fn init_logging() {
    engine_logging::initialize_for_tests();
}

fn default_config() -> ContentPrepConfig {
    ContentPrepConfig {
        normalization: NormalizationPolicy::default(),
        boilerplate: BoilerplatePolicy::default(),
        token_counter: Arc::new(WhitespaceTokenCounter),
    }
}

/// Exercise the production `content_prep` pipeline on the downloaded markdown corpus.
/// Prints provenance, hashing, truncation, and budget data so we can tune policies.
/// Kept `#[ignore]` to avoid running during regular test suites.
#[test]
#[ignore]
fn real_corpus_dry_run() {
    init_logging();
    let config = default_config();
    let budget_bytes = 4_096;
    let workspace_root = Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(|p| p.parent())
        .expect("workspace root missing");
    let corpus_dir = workspace_root.join("output");
    let entries = fs::read_dir(corpus_dir).expect("output directory missing");
    let mut processed = 0;

    for entry in entries.filter_map(Result::ok) {
        let path = entry.path();
        if path.extension().and_then(|ext| ext.to_str()) != Some("md") {
            continue;
        }

        let markdown = fs::read_to_string(&path).unwrap_or_else(|err| {
            panic!("failed to read {}: {err}", path.display());
        });

        let clean = derive_clean_text(&markdown, &path.to_string_lossy(), None, &config);
        let prepared = PreparedInput::from_clean_text(clean.clone(), budget_bytes);
        assert!(prepared.text().len() <= budget_bytes);

        println!(
            "[dry-run] {} -> original={} clean={} truncated={} boundary={:?} boilerplate={} hash={}",
            path.file_name().and_then(|s| s.to_str()).unwrap_or("<unknown>"),
            markdown.len(),
            clean.text().len(),
            prepared.was_truncated(),
            prepared.truncation_boundary(),
            clean.report().boilerplate_lines_removed,
            &clean.content_hash()[..8]
        );

        processed += 1;
    }

    assert!(processed > 0, "no markdown files present in output/");
}

fn render_like_worker(template: &str, vars: &HashMap<String, String>) -> String {
    let mut text = template.to_string();
    for (key, value) in vars {
        let placeholder = format!("{{{{{}}}}}", key);
        text = text.replace(&placeholder, value);
    }
    text
}

#[test]
fn pipeline_cleans_normalizes_and_hashes() {
    init_logging();
    let config = default_config();
    let body_lines = (0..30)
        .map(|i| format!("Article body line {}", i + 1))
        .collect::<Vec<_>>()
        .join(
            "
",
        );
    let markdown = format!(
        "---
meta: value
---

[home](#)
[about](#)
[contact](#)

Accept cookies to continue reading.

Article begins here.
{body}
",
        body = body_lines
    );

    let clean = derive_clean_text(
        &markdown,
        "https://example.org/article",
        Some("Article"),
        &config,
    );
    assert!(!clean.text().contains("meta:"));
    assert!(clean.report().boilerplate_lines_removed > 0);
    assert!(
        !clean.text().contains("Accept cookies to continue reading."),
        "expected boilerplate content to be removed"
    );

    let prepared = PreparedInput::from_clean_text(clean.clone(), 512);
    assert!(prepared.text().len() <= 512);
    assert_eq!(prepared.clean_text().content_hash(), clean.content_hash());
    assert_eq!(clean.content_hash(), content_hash(clean.text()));
}

#[test]
fn prepared_collection_respects_allocated_budgets() {
    init_logging();
    let config = default_config();
    let budgets = ContentBudget::new(240)
        .allocate_equal(2, 2, 50)
        .expect("budget should fit");

    let mut prepared_inputs = Vec::new();
    for (idx, budget) in budgets.iter().enumerate() {
        let markdown = format!(
            "---\nmeta: {}\n---\n\nArticle {} body content with enough words to require truncation.",
            idx, idx + 1
        );
        let clean = derive_clean_text(&markdown, "https://example.org", None, &config);
        let prepared = PreparedInput::from_clean_text(clean, *budget);
        assert!(prepared.text().len() <= *budget);
        prepared_inputs.push(prepared);
    }

    let collection = PreparedCollection::from_inputs(prepared_inputs);
    assert_eq!(collection.article_count(), 2);
    assert!(collection.text().contains("Article 1 body content"));
    assert!(collection.text().contains("Article 2 body content"));
}

#[test]
fn content_hash_is_deterministic_for_equivalent_cleaned_text() {
    init_logging();
    let config = default_config();
    let clean_a = derive_clean_text(
        "Lock-in text to guard the normalization pipeline.",
        "https://integrations",
        Some("Lock"),
        &config,
    );
    let clean_b = derive_clean_text(
        "  Lock-in text to guard the normalization pipeline.  ",
        "https://integrations",
        Some("Lock"),
        &config,
    );

    assert_eq!(clean_a.text(), clean_b.text());
    assert_eq!(clean_a.content_hash(), clean_b.content_hash());
    assert_eq!(clean_a.content_hash(), content_hash(clean_a.text()));
}

#[test]
fn byte_budget_invariant_with_unicode() {
    init_logging();
    let config = default_config();
    let ascii_clean =
        derive_clean_text("Simple ASCII content only.", "https://ascii", None, &config);
    let unicode_clean = derive_clean_text(
        "Emoji ❤️ section with text.",
        "https://unicode",
        None,
        &config,
    );

    let ascii_prep = PreparedInput::from_clean_text(ascii_clean, 20);
    let unicode_prep = PreparedInput::from_clean_text(unicode_clean, 24);

    assert!(ascii_prep.text().len() <= 20);
    assert!(unicode_prep.text().len() <= 24);
    assert!(unicode_prep.truncation_boundary().is_some());
}

#[test]
fn prompt_overhead_matches_rendered_prompt_length() {
    init_logging();
    let mut vars = TemplateVars::new();
    let document = "Brief article content.";
    vars.set_document("content", document);
    vars.insert("context".to_string(), "".to_string());
    let overhead = compute_prompt_overhead(&SUMMARY_PROMPT, "content", &[]);
    let rendered = vars.to_map();
    let system = render_like_worker(SUMMARY_PROMPT.system_template, &rendered);
    let user = render_like_worker(SUMMARY_PROMPT.user_template, &rendered);
    let total_rendered = system.len() + user.len();
    let doc_bytes = rendered.get("content").unwrap().len();
    assert_eq!(
        overhead + doc_bytes,
        total_rendered + 2 * harvester_engine::NONCE_OVERHEAD_BYTES
    );
}
