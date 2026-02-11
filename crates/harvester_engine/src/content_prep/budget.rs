use std::fmt::Write;

use crate::content_prep::truncation::truncate_to_budget;
use crate::content_prep::types::{CleanText, TruncationBoundary};
use crate::llm::prompt::{render_template, PromptTemplate, TemplateVars};

/// Byte budget for content within an LLM prompt.
pub struct ContentBudget {
    total_bytes: usize,
}

impl ContentBudget {
    pub fn new(total_bytes: usize) -> Self {
        Self { total_bytes }
    }

    pub fn available(&self) -> usize {
        self.total_bytes
    }

    /// Split budget equally among `n` items, subtracting per-item separator overhead.
    /// Returns `None` if budget cannot fit `n` items with `min_per_item` and the separators.
    pub fn allocate_equal(
        &self,
        n: usize,
        separator_bytes_per_item: usize,
        min_per_item: usize,
    ) -> Option<Vec<usize>> {
        if n == 0 {
            return Some(Vec::new());
        }

        let total_separator = separator_bytes_per_item.saturating_mul(n.saturating_sub(1));
        if total_separator >= self.total_bytes {
            return None;
        }

        let available = self.total_bytes - total_separator;
        let base = available / n;
        if base < min_per_item {
            return None;
        }

        let mut allocations = vec![base; n];
        let mut remainder = available - base * n;
        let mut idx = 0;
        while remainder > 0 {
            allocations[idx] += 1;
            remainder -= 1;
            idx = (idx + 1) % n;
        }

        Some(allocations)
    }
}

/// Nonce wrapping overhead: constant 49 bytes per document slot.
/// `<document-{12hex}>\n` = 24 bytes, `\n</document-{12hex}>` = 25 bytes.
pub const NONCE_OVERHEAD_BYTES: usize = 49;

/// Compute exact overhead bytes for a prompt template.
/// Renders the template with all context vars but empty document content,
/// measures the byte length, and adds `NONCE_OVERHEAD_BYTES` per document slot.
pub fn compute_prompt_overhead(
    template: &PromptTemplate,
    document_key: &str,
    context_vars: &[(String, String)],
) -> usize {
    let mut vars = TemplateVars::new();
    vars.set_document(document_key, "");

    // Build context string from key-value pairs
    let context_text = if context_vars.is_empty() {
        String::new()
    } else {
        context_vars
            .iter()
            .map(|(k, v)| format!("{}: {}", k, v))
            .collect::<Vec<_>>()
            .join("\n")
    };
    vars.insert("context".to_string(), context_text);

    for (key, value) in context_vars {
        vars.insert(key.clone(), value.clone());
    }
    let rendered = vars.to_map();
    let system = render_template(template.system_template, &rendered).unwrap_or_else(|e| {
        // If template rendering fails, use conservative estimate
        format!("ERROR: {}", e)
    });
    let user = render_template(template.user_template, &rendered)
        .unwrap_or_else(|e| format!("ERROR: {}", e));
    system.len() + user.len() + NONCE_OVERHEAD_BYTES
}

/// Single article prepared for LLM consumption.
pub struct PreparedInput {
    clean_text: CleanText,
    bounded_text: String,
    budget_bytes: usize,
    was_truncated: bool,
    truncated_at_boundary: Option<TruncationBoundary>,
}

impl PreparedInput {
    pub fn text(&self) -> &str {
        &self.bounded_text
    }

    pub fn clean_text(&self) -> &CleanText {
        &self.clean_text
    }

    pub fn was_truncated(&self) -> bool {
        self.was_truncated
    }

    pub fn truncation_boundary(&self) -> Option<TruncationBoundary> {
        self.truncated_at_boundary
    }

    pub fn budget_bytes(&self) -> usize {
        self.budget_bytes
    }

    pub fn from_clean_text(clean_text: CleanText, budget_bytes: usize) -> Self {
        let (bounded_text, boundary) = truncate_to_budget(clean_text.text(), budget_bytes);
        Self {
            clean_text,
            bounded_text,
            budget_bytes,
            was_truncated: boundary.is_some(),
            truncated_at_boundary: boundary,
        }
    }
}

/// Multiple articles prepared for a briefing prompt.
pub struct PreparedCollection {
    items: Vec<PreparedInput>,
    concatenated: String,
}

impl PreparedCollection {
    pub fn text(&self) -> &str {
        &self.concatenated
    }

    pub fn article_count(&self) -> usize {
        self.items.len()
    }

    /// Each item wrapped: `--- Article 1: {title} ---\n{text}\n`
    pub fn from_inputs(inputs: Vec<PreparedInput>) -> Self {
        let mut concatenated = String::new();
        for (idx, item) in inputs.iter().enumerate() {
            if idx > 0 {
                concatenated.push('\n');
            }
            let title = item
                .clean_text()
                .report()
                .source_title
                .as_deref()
                .unwrap_or("untitled");
            writeln!(&mut concatenated, "--- Article {}: {} ---", idx + 1, title).ok();
            writeln!(&mut concatenated, "{}", item.text()).ok();
        }
        Self {
            items: inputs,
            concatenated,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::content_prep::types::CleanTextReport;
    use crate::llm::prompt::{PromptId, PromptTemplate, TemplateVars};

    struct DummyCleanTextBuilder;

    impl DummyCleanTextBuilder {
        fn build(text: &str) -> CleanText {
            let report = CleanTextReport {
                source_url: "https://example.com".to_string(),
                source_title: Some("Title".to_string()),
                original_bytes: text.len(),
                original_tokens: text.split_whitespace().count() as u32,
                clean_bytes: text.len(),
                clean_tokens: text.split_whitespace().count() as u32,
                boilerplate_lines_removed: 0,
                boilerplate_patterns: Vec::new(),
                content_hash: "hash".to_string(),
            };
            CleanText::new(text.to_string(), report.content_hash.clone(), report)
        }
    }

    #[test]
    fn allocation_accounts_for_separator_and_remainder() {
        let budget = ContentBudget::new(100);
        let allocation = budget.allocate_equal(3, 2, 10).unwrap();
        assert_eq!(allocation.len(), 3);
        let total: usize = allocation.iter().sum::<usize>() + 2 * 2;
        assert_eq!(total, 100);
        assert!(allocation.iter().all(|&slot| slot >= 10));
    }

    #[test]
    fn allocation_respects_min_per_item() {
        let budget = ContentBudget::new(30);
        assert!(budget.allocate_equal(2, 5, 13).is_none());
    }

    #[test]
    fn prepared_input_truncates_to_budget() {
        let clean_text = DummyCleanTextBuilder::build("abcdefg");
        let input = PreparedInput::from_clean_text(clean_text, 5);
        assert!(input.text().len() <= 5);
        assert!(input.was_truncated());
        assert_eq!(
            input.truncation_boundary(),
            Some(TruncationBoundary::Character)
        );
    }

    #[test]
    fn prepared_collection_concatenates_articles() {
        let clean1 = DummyCleanTextBuilder::build("first");
        let clean2 = DummyCleanTextBuilder::build("second");
        let input1 = PreparedInput::from_clean_text(clean1, 10);
        let input2 = PreparedInput::from_clean_text(clean2, 10);
        let collection = PreparedCollection::from_inputs(vec![input1, input2]);
        assert!(collection.text().contains("--- Article 1"));
        assert_eq!(collection.article_count(), 2);
    }

    #[test]
    fn compute_prompt_overhead_matches_rendered_sum() {
        let template = PromptTemplate {
            id: PromptId::ArticleSummary,
            version: 1,
            system_template: "System {{topic}} {{content}}",
            user_template: "User {{content}}",
            description: "desc",
            expected_format: "json",
        };

        let context_vars = vec![("topic".to_string(), "rust".to_string())];
        let overhead = compute_prompt_overhead(&template, "content", &context_vars);

        let mut vars = TemplateVars::new();
        vars.set_document("content", "");
        for (key, value) in &context_vars {
            vars.insert(key.clone(), value.clone());
        }
        let rendered = vars.to_map();
        let system = render_template(template.system_template, &rendered).unwrap();
        let user = render_template(template.user_template, &rendered).unwrap();
        assert_eq!(overhead, system.len() + user.len() + NONCE_OVERHEAD_BYTES);
    }
}
