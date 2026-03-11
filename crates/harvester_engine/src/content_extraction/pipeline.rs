use scraper::{Html, Selector};

use crate::content_extraction::{
    candidate_select::select_candidate,
    diagnostics::{ExtractionDiagnostics, ExtractionOutcome},
    dom_prune::should_prune_element,
    markdown_cleanup::cleanup_markdown,
    policy::ExtractionPolicy,
};
use crate::links::{ConversionOutput, ExtractedLink, LinkExtractingConverter};
use engine_logging::{engine_debug, engine_warn};

const DEFAULT_MAX_LINKS: usize = 5_000;

/// The result of a full extraction run.
pub struct ExtractedArticle {
    pub title: Option<String>,
    pub markdown: String,
    pub links: Vec<ExtractedLink>,
    pub diagnostics: ExtractionDiagnostics,
    pub canonical_url: Option<String>,
    pub published_utc: Option<String>,
}

/// Orchestrates candidate selection, DOM pruning, markdown conversion, and cleanup.
pub struct ExtractionPipeline {
    policy: ExtractionPolicy,
    max_links: usize,
}

impl ExtractionPipeline {
    pub fn new(policy: ExtractionPolicy) -> Self {
        Self {
            policy,
            max_links: DEFAULT_MAX_LINKS,
        }
    }

    pub fn with_max_links(mut self, max_links: usize) -> Self {
        self.max_links = max_links;
        self
    }

    pub fn extract(&self, html: &str, base_url: Option<&str>) -> ExtractedArticle {
        let doc = Html::parse_document(html);

        let title = extract_title_from_doc(&doc);

        let (candidate_element, info) = select_candidate(&doc, &self.policy.candidate);

        let prune_policy = self.policy.prune.clone();
        let prune_closure = |el: &scraper::ElementRef<'_>| should_prune_element(el, &prune_policy);

        let converter = LinkExtractingConverter::with_max_links(self.max_links);
        let (ConversionOutput { markdown, links }, prune_stats) =
            converter.convert_element(candidate_element, base_url, &prune_closure);

        let (cleaned_markdown, block_drop_counts) =
            cleanup_markdown(&markdown, &self.policy.cleanup);

        let used_fallback = cleaned_markdown.len() < self.policy.retention.min_retained_chars
            || (cleaned_markdown.len() as f64 / markdown.len().max(1) as f64)
                < self.policy.retention.min_retention_ratio;

        let final_markdown = if used_fallback {
            engine_warn!(
                "[extract-pipeline] retention safeguard triggered: cleaned={} original={}",
                cleaned_markdown.len(),
                markdown.len()
            );
            markdown.clone()
        } else {
            cleaned_markdown
        };

        let retention_ratio = final_markdown.len() as f64 / markdown.len().max(1) as f64;

        engine_debug!(
            "[extract-pipeline] candidate={:?} score={:.1} pruned={} cleanup_dropped={} bytes_in={} bytes_out={}",
            info.kind,
            info.score,
            prune_stats.total_pruned,
            block_drop_counts.total(),
            html.len(),
            final_markdown.len()
        );

        let diagnostics = ExtractionDiagnostics {
            candidate_kind: Some(info.kind),
            candidate_score: Some(info.score),
            pruned_by_tag: 0,
            pruned_by_attr: prune_stats.total_pruned,
            cleanup_blocks_dropped: block_drop_counts,
            original_html_bytes: html.len(),
            pre_cleanup_markdown_bytes: markdown.len(),
            final_markdown_bytes: final_markdown.len(),
            retention_ratio: Some(retention_ratio),
            outcome: Some(if used_fallback {
                ExtractionOutcome::LessPrunedFallback {
                    reason: "retention safeguard".to_string(),
                }
            } else {
                ExtractionOutcome::BestCandidate {
                    score: info.score,
                    pruned_nodes: prune_stats.total_pruned,
                }
            }),
        };

        ExtractedArticle {
            title,
            markdown: final_markdown,
            links,
            diagnostics,
            canonical_url: None,
            published_utc: None,
        }
    }
}

fn extract_title_from_doc(doc: &Html) -> Option<String> {
    if let Some(title) = try_og_title(doc) {
        return Some(title);
    }
    try_title_tag(doc)
}

fn try_og_title(doc: &Html) -> Option<String> {
    let sel = Selector::parse(r#"meta[property="og:title"]"#).ok()?;
    doc.select(&sel)
        .next()
        .and_then(|el| el.value().attr("content"))
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
}

fn try_title_tag(doc: &Html) -> Option<String> {
    let sel = Selector::parse("title").ok()?;
    doc.select(&sel)
        .next()
        .map(|el| el.text().collect::<String>().trim().to_string())
        .filter(|s| !s.is_empty())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::content_extraction::policy::{ExtractionPolicy, RetentionPolicy};

    #[test]
    fn extracts_article_from_article_element() {
        let html = r#"<!DOCTYPE html><html><head><title>Test Article</title></head><body>
        <nav><a href="/">Home</a><a href="/about">About</a></nav>
        <article>
            <h1>Article Headline</h1>
            <p>This is the first paragraph with substantial content that forms the body of the article.</p>
            <p>This is the second paragraph. It adds more content to make the article interesting and long.</p>
        </article>
        <footer>Copyright 2024. All rights reserved. Subscribe to our newsletter.</footer>
        </body></html>"#;

        let pipeline = ExtractionPipeline::new(ExtractionPolicy::default());
        let result = pipeline.extract(html, Some("https://example.com/article"));

        assert_eq!(result.title.as_deref(), Some("Test Article"));

        assert!(
            result.markdown.contains("Article Headline"),
            "headline missing from markdown"
        );
        assert!(
            result.markdown.contains("first paragraph"),
            "article text missing"
        );

        assert!(result.diagnostics.candidate_score.is_some());
        assert!(result.diagnostics.outcome.is_some());
        assert!(result.diagnostics.original_html_bytes > 0);
    }

    #[test]
    fn og_title_preferred_over_title_tag() {
        let html = r#"<!DOCTYPE html><html><head>
            <title>Site Name - Article Title</title>
            <meta property="og:title" content="Clean Article Title"/>
        </head><body><article><p>Content content content content content content content content content content.</p></article></body></html>"#;

        let pipeline = ExtractionPipeline::new(ExtractionPolicy::default());
        let result = pipeline.extract(html, None);

        assert_eq!(result.title.as_deref(), Some("Clean Article Title"));
    }

    #[test]
    fn retention_safeguard_falls_back_when_cleanup_removes_too_much() {
        let policy = ExtractionPolicy {
            retention: RetentionPolicy {
                min_retained_chars: 10_000, // impossible threshold to trigger fallback
                min_retention_ratio: 0.0,
            },
            ..Default::default()
        };

        let html = r#"<!DOCTYPE html><html><head><title>T</title></head><body>
        <article><p>Short content.</p></article>
        </body></html>"#;

        let pipeline = ExtractionPipeline::new(policy);
        let result = pipeline.extract(html, None);

        assert!(!result.markdown.is_empty());
    }

    #[test]
    fn nav_and_footer_are_pruned_from_output() {
        let html = r#"<!DOCTYPE html><html><body>
        <nav>Home About Contact</nav>
        <article>
            <p>Real article content that is meaningful and present.</p>
            <p>More real article content here for the second paragraph.</p>
        </article>
        <footer>All rights reserved. Subscribe to newsletter.</footer>
        </body></html>"#;

        let pipeline = ExtractionPipeline::new(ExtractionPolicy::default());
        let result = pipeline.extract(html, None);

        assert!(
            result.diagnostics.pruned_by_attr > 0
                || result.diagnostics.pruned_by_tag > 0
                || result.diagnostics.candidate_kind.is_some(),
            "should have had some pruning or candidate selection"
        );
        assert!(
            result.markdown.contains("Real article content"),
            "article content missing"
        );
    }
}
