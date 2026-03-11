use crate::content_extraction::diagnostics::BlockDropCounts;
use crate::content_extraction::policy::MarkdownCleanupPolicy;

/// Clean up a markdown string by dropping blocks that match noise patterns.
/// Returns (cleaned_markdown, per_reason_drop_counts).
pub fn cleanup_markdown(markdown: &str, policy: &MarkdownCleanupPolicy) -> (String, BlockDropCounts) {
    let blocks: Vec<&str> = markdown.split("\n\n").collect();
    let mut kept = Vec::with_capacity(blocks.len());
    let mut counts = BlockDropCounts::default();

    for block in blocks {
        if policy.drop_css_blocks && looks_like_css_block(block) {
            counts.css_blocks += 1;
        } else if policy.drop_script_payloads && looks_like_script_payload(block) {
            counts.script_payloads += 1;
        } else if policy.drop_subscription_blocks && looks_like_subscription_block(block) {
            counts.subscription_blocks += 1;
        } else if policy.drop_recirculation_blocks && looks_like_recirculation_block(block, policy) {
            counts.recirculation_blocks += 1;
        } else {
            kept.push(block);
        }
    }

    (kept.join("\n\n"), counts)
}

fn looks_like_css_block(block: &str) -> bool {
    let lines: Vec<&str> = block.lines().collect();
    if lines.len() < 3 {
        return false;
    }
    let css_markers = ["{", "}", "@media", "var(--", "!important"];
    let matching = lines
        .iter()
        .filter(|line| css_markers.iter().any(|marker| line.contains(marker)))
        .count();
    matching as f64 / lines.len() as f64 > 0.3
}

fn looks_like_script_payload(block: &str) -> bool {
    if block.contains("__NEXT_DATA__")
        || block.contains("window.__")
        || block.contains("hydration")
    {
        return true;
    }
    let trimmed = block.trim();
    // Require 500+ chars and at least 3 key-value pairs to avoid false positives on short JSON-like content
    if trimmed.len() > 500 && trimmed.starts_with('{') && trimmed.matches("\":").count() >= 3 {
        return true;
    }
    false
}

fn looks_like_subscription_block(block: &str) -> bool {
    let lower = block.to_lowercase();
    lower.contains("sign up for")
        || lower.contains("subscribe to")
        || lower.contains("newsletter signup")
        || lower.contains("get our newsletter")
        || lower.contains("enter your email")
}

fn looks_like_recirculation_block(block: &str, _policy: &MarkdownCleanupPolicy) -> bool {
    let lower = block.to_lowercase();
    let is_short = block.len() < 150;

    // Short-only patterns — could appear in legitimate long text (e.g. "read more about this topic")
    let short_only_patterns = ["read more", "more from", "also from"];
    if is_short && short_only_patterns.iter().any(|p| lower.contains(p)) {
        return true;
    }

    // Strong patterns — typically only appear in widget/recirculation blocks, safe regardless of length
    let strong_patterns = [
        "you may like",
        "you may also like",
        "related articles",
        "recommended for you",
    ];
    strong_patterns.iter().any(|p| lower.contains(p))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn all_enabled() -> MarkdownCleanupPolicy {
        MarkdownCleanupPolicy::default()
    }

    #[test]
    fn css_block_is_dropped() {
        let markdown = concat!(
            "Real article text here.\n\n",
            ".header {\n",
            "  color: red;\n",
            "  font-size: 16px;\n",
            "}\n",
            "@media (max-width: 600px) {\n",
            "  .header { display: none; }\n",
            "}\n\n",
            "More article text."
        );
        let (result, counts) = cleanup_markdown(markdown, &all_enabled());
        assert_eq!(counts.css_blocks, 1);
        assert_eq!(counts.total(), 1);
        assert!(result.contains("Real article text here."));
        assert!(result.contains("More article text."));
        assert!(!result.contains("@media"));
    }

    #[test]
    fn js_payload_is_dropped() {
        // Payload must be 500+ chars with 3+ "": sequences to pass the tighter heuristic
        let json_blob = format!(
            "Normal intro.\n\n{}\n\nNormal outro.",
            "{".to_owned()
                + &"\"key\": \"value\", ".repeat(32)
                + "\"end\": true}"
        );
        let (result, counts) = cleanup_markdown(&json_blob, &all_enabled());
        assert_eq!(counts.script_payloads, 1);
        assert_eq!(counts.total(), 1);
        assert!(result.contains("Normal intro."));
        assert!(result.contains("Normal outro."));
    }

    #[test]
    fn next_data_payload_is_dropped() {
        let markdown = "Intro text.\n\n__NEXT_DATA__ = {\"props\": {}}\n\nOutro text.";
        let (result, counts) = cleanup_markdown(markdown, &all_enabled());
        assert_eq!(counts.script_payloads, 1);
        assert_eq!(counts.total(), 1);
        assert!(result.contains("Intro text."));
        assert!(!result.contains("__NEXT_DATA__"));
    }

    #[test]
    fn subscription_block_is_dropped() {
        let markdown = concat!(
            "Great article content.\n\n",
            "Sign up for our daily newsletter to get the latest news.\n\n",
            "More article content."
        );
        let (result, counts) = cleanup_markdown(markdown, &all_enabled());
        assert_eq!(counts.subscription_blocks, 1);
        assert_eq!(counts.total(), 1);
        assert!(result.contains("Great article content."));
        assert!(result.contains("More article content."));
        assert!(!result.contains("Sign up for"));
    }

    #[test]
    fn recirculation_block_is_dropped() {
        let markdown = concat!(
            "Article body text.\n\n",
            "Read more: Top 10 tips for productivity\n\n",
            "Continued article text."
        );
        let (result, counts) = cleanup_markdown(markdown, &all_enabled());
        assert_eq!(counts.recirculation_blocks, 1);
        assert_eq!(counts.total(), 1);
        assert!(result.contains("Article body text."));
        assert!(result.contains("Continued article text."));
        assert!(!result.contains("Read more"));
    }

    #[test]
    fn regular_article_text_is_not_dropped() {
        let markdown = concat!(
            "The investigation revealed several key findings about the policy changes.\n\n",
            "Officials confirmed the new regulations will take effect next quarter.\n\n",
            "Experts say the impact will be felt across multiple industries."
        );
        let (result, counts) = cleanup_markdown(markdown, &all_enabled());
        assert_eq!(counts.total(), 0);
        assert_eq!(result, markdown);
    }
}
