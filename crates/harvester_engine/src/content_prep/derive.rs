use std::sync::Arc;

use engine_logging::engine_info;

use crate::{
    content_prep::{
        boilerplate::{filter_boilerplate, BoilerplatePolicy, BoilerplateResult},
        normalize::{normalize_markdown, NormalizationPolicy},
        types::{CleanText, CleanTextReport},
    },
    frontmatter::strip_frontmatter,
    llm::replay::content_hash,
    token::TokenCounter,
};

#[derive(Debug, Clone)]
pub struct ContentPrepConfig {
    pub normalization: NormalizationPolicy,
    pub boilerplate: BoilerplatePolicy,
    pub token_counter: Arc<dyn TokenCounter>,
}

pub fn derive_clean_text(
    markdown: &str,
    source_url: &str,
    source_title: Option<&str>,
    config: &ContentPrepConfig,
) -> CleanText {
    let stripped = strip_frontmatter(markdown);
    let normalized = normalize_markdown(stripped, &config.normalization);
    let BoilerplateResult {
        filtered_text,
        removed_line_count,
        detected_patterns,
    } = filter_boilerplate(&normalized, &config.boilerplate);

    let original_bytes = markdown.len();
    let clean_bytes = filtered_text.len();
    let original_tokens = config.token_counter.count(markdown);
    let clean_tokens = config.token_counter.count(&filtered_text);
    let hash = content_hash(&filtered_text);

    let report = CleanTextReport {
        source_url: source_url.to_string(),
        source_title: source_title.map(str::to_string),
        original_bytes,
        original_tokens,
        clean_bytes,
        clean_tokens,
        boilerplate_lines_removed: removed_line_count,
        boilerplate_patterns: detected_patterns,
        content_hash: hash.clone(),
    };

    engine_info!(
        "[content-prep] url={} original_bytes={} clean_bytes={} hash={}",
        source_url,
        original_bytes,
        clean_bytes,
        &hash[..8],
    );

    CleanText::new(filtered_text, hash, report)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::token::{TokenCounter, WhitespaceTokenCounter};
    use engine_logging;
    use std::sync::Arc;

    const SOURCE_URL: &str = "https://example.com";
    const SOURCE_TITLE: &str = "Test Article";

    fn init_logging() {
        let _ = engine_logging::initialize_for_tests();
    }

    fn test_config(counter: Arc<dyn TokenCounter>) -> ContentPrepConfig {
        ContentPrepConfig {
            normalization: NormalizationPolicy::default(),
            boilerplate: BoilerplatePolicy::default(),
            token_counter: counter,
        }
    }

    #[derive(Debug)]
    struct ConstantTokenCounter;

    impl TokenCounter for ConstantTokenCounter {
        fn count(&self, _text: &str) -> u32 {
            1
        }
    }

    #[test]
    fn strips_frontmatter_and_normalizes() {
        init_logging();
        let config = ContentPrepConfig {
            normalization: NormalizationPolicy::default(),
            boilerplate: BoilerplatePolicy::default(),
            token_counter: Arc::new(WhitespaceTokenCounter::default()),
        };
        let markdown = "---\nmeta: value\n---\n\nLine 1\n\n<!-- comment -->Line 2\n";
        let clean = derive_clean_text(markdown, SOURCE_URL, Some(SOURCE_TITLE), &config);
        assert!(clean.text().contains("Line 1"));
        assert!(clean.text().contains("Line 2"));
        assert_eq!(clean.report().source_title.as_deref(), Some(SOURCE_TITLE));
        assert!(!clean.text().contains("meta: value"));
        assert!(!clean.text().contains("<!--"));
    }

    #[test]
    fn deterministic_report_and_hash() {
        init_logging();
        let config = test_config(Arc::new(WhitespaceTokenCounter::default()));
        let markdown = "Stable content\n\nSome more text.";
        let first = derive_clean_text(markdown, SOURCE_URL, None, &config);
        let second = derive_clean_text(markdown, SOURCE_URL, None, &config);
        assert_eq!(first.text(), second.text());
        assert_eq!(first.content_hash(), second.content_hash());
        assert_eq!(first.report(), second.report());
    }

    #[test]
    fn hash_matches_replay_content_hash() {
        init_logging();
        let config = test_config(Arc::new(WhitespaceTokenCounter::default()));
        let markdown = "Paragraph.\nAnother line.";
        let clean = derive_clean_text(markdown, SOURCE_URL, None, &config);
        assert_eq!(
            clean.content_hash(),
            crate::llm::replay::content_hash(clean.text())
        );
    }

    #[test]
    fn token_counter_swap_does_not_change_hash() {
        init_logging();
        let markdown = "Shared content for different counters.";
        let config_whitespace = test_config(Arc::new(WhitespaceTokenCounter::default()));
        let config_constant = test_config(Arc::new(ConstantTokenCounter));

        let clean1 = derive_clean_text(markdown, SOURCE_URL, None, &config_whitespace);
        let clean2 = derive_clean_text(markdown, SOURCE_URL, None, &config_constant);

        assert_eq!(clean1.content_hash(), clean2.content_hash());
        assert_ne!(
            clean1.report().original_tokens,
            clean2.report().original_tokens
        );
    }
}
