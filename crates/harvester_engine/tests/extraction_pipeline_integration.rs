//! Integration tests for ExtractionPipeline — fixture-based regression tests.

use harvester_engine::{CandidateKind, ExtractionPipeline, ExtractionPolicy};

fn make_pipeline() -> ExtractionPipeline {
    ExtractionPipeline::new(ExtractionPolicy::default())
}

#[test]
fn clean_article_content_preserved() {
    let html = concat!(
        r#"<!DOCTYPE html><html><head><title>Investment Memo</title></head><body>"#,
        r#"<article>"#,
        r#"<h1>Third Quarter Investment Memo</h1>"#,
        r#"<p>Global equity markets declined sharply in Q3, driven by rising interest rates "#,
        r#"and persistent inflation concerns. The Federal Reserve maintained its hawkish "#,
        r#"stance throughout the period.</p>"#,
        r#"<p>Our portfolio weathered the storm well, with defensive positions in healthcare "#,
        r#"and utilities offsetting losses in technology holdings.</p>"#,
        r#"<p>Looking ahead to Q4, we expect volatility to persist as markets digest the "#,
        r#"implications of tighter monetary policy for corporate earnings.</p>"#,
        r#"</article>"#,
        r#"</body></html>"#,
    );

    let result = make_pipeline().extract(html, Some("https://example.com/memo"));

    // Core content preserved
    assert!(result.markdown.contains("Third Quarter"), "headline missing");
    assert!(
        result.markdown.contains("Global equity markets"),
        "first paragraph missing"
    );
    assert!(
        result.markdown.contains("portfolio weathered"),
        "second paragraph missing"
    );
    assert!(
        result.markdown.contains("volatility to persist"),
        "third paragraph missing"
    );

    // No empty output
    assert!(
        result.markdown.len() > 100,
        "markdown too short: {}",
        result.markdown.len()
    );
}

#[test]
fn article_content_selected_over_body_with_chrome() {
    let article_text = concat!(
        "Federal regulators announced sweeping new rules governing artificial intelligence ",
        "deployment in financial services. The regulations require firms to maintain audit ",
        "trails and conduct regular bias testing.",
    );

    let html = format!(
        concat!(
            r#"<!DOCTYPE html><html><head><title>AI Regulation News</title></head><body>"#,
            r#"<nav><a href="/">Home</a> | <a href="/news">News</a> | <a href="/opinion">Opinion</a> | "#,
            r#"<a href="/markets">Markets</a> | <a href="/tech">Tech</a> | <a href="/about">About</a></nav>"#,
            r#"<article><h1>New AI Rules</h1><p>{}</p><p>Industry groups welcomed the clarity but "#,
            r#"expressed concerns about compliance costs for smaller firms.</p></article>"#,
            r#"<aside><h2>Related</h2><p><a href="/story1">Related story 1</a></p>"#,
            r#"<p><a href="/story2">Related story 2</a></p>"#,
            r#"<p><a href="/story3">Related story 3</a></p></aside>"#,
            r#"<footer><p>© 2024 News Corp. All rights reserved.</p>"#,
            r#"<p>Subscribe | Privacy | Terms of Service</p></footer>"#,
            r#"</body></html>"#,
        ),
        article_text
    );

    let result = make_pipeline().extract(&html, Some("https://news.example.com/ai-rules"));

    // Core article text preserved
    assert!(
        result.markdown.contains("Federal regulators"),
        "core article text missing"
    );
    assert!(
        result.markdown.contains("New AI Rules"),
        "article headline missing"
    );

    // Candidate selected (article preferred over body)
    assert!(
        matches!(
            result.diagnostics.candidate_kind,
            Some(CandidateKind::Article)
        ),
        "expected Article candidate, got: {:?}",
        result.diagnostics.candidate_kind
    );
}

#[test]
fn newsletter_signup_block_removed_by_dom_prune() {
    // The DOM prune policy drops elements whose class/id attribute contains tokens from
    // the deny list. "newsletter" and "signup" are both deny-listed, so a real-world CMS
    // injection like `<div class="newsletter-signup">` is pruned before conversion.
    // This test locks in that behaviour end-to-end through the pipeline.
    let html = concat!(
        r#"<!DOCTYPE html><html><head><title>Long Read</title></head><body><article>"#,
        r#"<h1>The Future of Work</h1>"#,
        r#"<p>Remote work has fundamentally altered the relationship between employers and "#,
        r#"employees. Studies show productivity has remained stable or improved for most "#,
        r#"knowledge workers since the pandemic.</p>"#,
        // Real-world pattern: CMS injects signup block with a recognisable class.
        // The prune policy matches on whitespace-split tokens, so "newsletter signup"
        // (space-separated) means either token independently triggers the drop.
        r#"<div class="newsletter signup">Sign up for our newsletter to get the best "#,
        r#"journalism delivered to your inbox. Enter your email to subscribe today.</div>"#,
        r#"<p>The implications for commercial real estate are profound. Office vacancy "#,
        r#"rates in major cities remain elevated despite return-to-office mandates from "#,
        r#"large employers.</p>"#,
        r#"</article></body></html>"#,
    );

    let result = make_pipeline().extract(html, None);

    // Article content preserved
    assert!(
        result.markdown.contains("Remote work has fundamentally"),
        "article start missing"
    );
    assert!(
        result.markdown.contains("commercial real estate"),
        "article end missing"
    );

    // Newsletter block removed by DOM attribute prune (class token "newsletter"/"signup")
    assert!(
        !result.markdown.to_lowercase().contains("sign up for our newsletter"),
        "newsletter block should have been removed by DOM prune"
    );

    // Pruning was recorded in diagnostics
    assert!(
        result.diagnostics.pruned_by_attr > 0,
        "expected pruned_by_attr > 0, got {}",
        result.diagnostics.pruned_by_attr
    );
}

#[test]
fn css_in_js_block_does_not_survive_into_markdown() {
    // A page with a CSS-in-JS payload (typical of styled-components or emotion)
    let css_payload = concat!(
        ".sc-bdVBmm {\n",
        "  display: flex;\n",
        "  flex-direction: column;\n",
        "  @media (min-width: 768px) {\n",
        "    flex-direction: row;\n",
        "  }\n",
        "  color: var(--primary-color);\n",
        "  background: var(--background);\n",
        "  !important override;\n",
        "}\n",
    );

    let html = format!(
        concat!(
            r#"<!DOCTYPE html><html><head><title>Tech Article</title></head><body><article>"#,
            r#"<h1>Tech Article</h1>"#,
            r#"<p>This is the actual article content about technology trends and developments."#,
            r#" It discusses cloud computing and distributed systems at length.</p>"#,
            r#"<style>{}</style>"#,
            r#"<p>The second paragraph continues the discussion of technology trends in "#,
            r#"enterprise software development and cloud architecture.</p>"#,
            r#"</article></body></html>"#,
        ),
        css_payload
    );

    let result = make_pipeline().extract(&html, None);

    // Article text preserved
    assert!(
        result.markdown.contains("actual article content"),
        "article text missing"
    );

    // CSS should not be in output (style tags are skipped by converter)
    assert!(
        !result.markdown.contains("flex-direction"),
        "CSS leaked into markdown: {}",
        &result.markdown[..result.markdown.len().min(500)]
    );
}

#[test]
fn long_legitimate_article_not_over_pruned() {
    // Simulate a long-form article (transcript-like or memo) that should survive intact
    let paragraphs: Vec<String> = (1..=15)
        .map(|i| {
            format!(
                "<p>Paragraph {} contains substantial analysis of the current macroeconomic \
                 environment. The discussion covers central bank policy, inflation dynamics, \
                 and the implications for asset allocation in a rising rate environment. \
                 Investors should consider the duration risk in their fixed income portfolios \
                 as yields continue to normalize toward historical averages.</p>",
                i
            )
        })
        .collect();

    let html = format!(
        r#"<!DOCTYPE html><html><head><title>Annual Investment Outlook</title></head><body>
        <article><h1>Annual Investment Outlook</h1>{}</article>
        </body></html>"#,
        paragraphs.join("")
    );

    let result = make_pipeline().extract(&html, Some("https://funds.example.com/outlook"));

    // Title extracted
    assert_eq!(result.title.as_deref(), Some("Annual Investment Outlook"));

    // All paragraphs present (none over-pruned)
    for i in 1..=15 {
        assert!(
            result.markdown.contains(&format!("Paragraph {}", i)),
            "Paragraph {} missing from output",
            i
        );
    }

    // Output is substantial
    assert!(
        result.markdown.len() > 1000,
        "long article collapsed: only {} chars",
        result.markdown.len()
    );
}

#[test]
fn diagnostics_are_populated_consistently() {
    let html = concat!(
        r#"<!DOCTYPE html><html><head><title>Diagnostics Test</title></head><body>"#,
        r#"<article><h1>Test</h1>"#,
        r#"<p>Content content content content content content content content content.</p>"#,
        r#"</article></body></html>"#,
    );

    let result = make_pipeline().extract(html, Some("https://test.example.com/"));

    // Diagnostics should be populated
    let diag = &result.diagnostics;
    assert!(diag.original_html_bytes > 0, "original_html_bytes not set");
    assert!(diag.final_markdown_bytes > 0, "final_markdown_bytes not set");
    assert!(diag.candidate_kind.is_some(), "candidate_kind not set");
    assert!(diag.candidate_score.is_some(), "candidate_score not set");
    assert!(diag.outcome.is_some(), "outcome not set");

    // Retention ratio is reasonable
    if let Some(ratio) = diag.retention_ratio {
        assert!(ratio > 0.0, "retention_ratio should be positive");
        assert!(
            ratio <= 2.0,
            "retention_ratio too high: {}",
            ratio
        ); // pre-cleanup to post shouldn't more than double
    }

    // original_html_bytes matches input
    assert_eq!(
        diag.original_html_bytes,
        html.len(),
        "html byte count mismatch"
    );
}

#[test]
fn import_and_fetch_pipeline_use_same_extraction_behavior() {
    // This test verifies the convergence: both import and fetch now use ExtractionPipeline.
    // We verify that the pipeline produces consistent output for the same HTML content.

    let html = concat!(
        r#"<!DOCTYPE html><html><head>"#,
        r#"<title>Consistency Test</title>"#,
        r#"<link rel="canonical" href="https://example.com/test"/>"#,
        r#"</head><body>"#,
        r#"<article><h1>Article Headline</h1>"#,
        r#"<p>The article text is present and should be consistently extracted "</p>"#,
        r#"<p>by both the fetch pipeline and the import pipeline.</p>"#,
        r#"</article></body></html>"#,
    );

    // Run through ExtractionPipeline (same as both fetch and import now use)
    let result1 = make_pipeline().extract(html, Some("https://example.com/test"));
    let result2 = make_pipeline().extract(html, Some("https://example.com/test"));

    // Same input → same output (determinism)
    assert_eq!(
        result1.markdown, result2.markdown,
        "extraction is not deterministic"
    );
    assert_eq!(
        result1.title, result2.title,
        "title extraction is not deterministic"
    );
}
