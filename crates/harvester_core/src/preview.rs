//! Summary formatting and article title fallback.
//!
//! Pure formatters for the summary-only reading pane and article labels.

use crate::briefing::ArticleSummaryResult;

/// Format a completed article summary for display in the summary reading pane.
pub fn format_summary_for_preview(summary: &ArticleSummaryResult) -> String {
    use std::fmt::Write;
    let mut out = String::new();
    let _ = writeln!(out, "{}", summary.summary);
    if !summary.key_points.is_empty() {
        out.push('\n');
        let _ = writeln!(out, "## Key Points");
        out.push('\n');
        for point in &summary.key_points {
            let _ = writeln!(out, "  - {}", point);
        }
    }
    out
}

pub fn best_effort_article_title(source_title: Option<&str>, url: &str) -> Option<String> {
    let explicit = source_title
        .map(str::trim)
        .filter(|title| !title.is_empty())
        .map(ToOwned::to_owned);
    if explicit.is_some() {
        return explicit;
    }

    let trimmed = url.trim();
    let without_scheme = trimmed
        .find("://")
        .map(|pos| &trimmed[pos + 3..])
        .unwrap_or(trimmed);
    let without_query = without_scheme
        .split(['?', '#'])
        .next()
        .unwrap_or(without_scheme)
        .trim_end_matches('/');
    let slug = without_query
        .rsplit('/')
        .next()
        .unwrap_or(without_query)
        .trim();
    if slug.is_empty() || !slug.contains('-') {
        return None;
    }

    Some(title_case_label(&slug.replace(['-', '_'], " ")))
}

fn title_case_label(value: &str) -> String {
    let mut out = Vec::new();
    for word in value.split(['-', '_', ' ']).filter(|word| !word.is_empty()) {
        let mut chars = word.chars();
        let Some(first) = chars.next() else {
            continue;
        };
        let rest: String = chars.collect();
        out.push(format!(
            "{}{}",
            first.to_uppercase(),
            rest.to_ascii_lowercase()
        ));
    }
    if out.is_empty() {
        value.trim().to_string()
    } else {
        out.join(" ")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn best_effort_article_title_falls_back_to_humanized_slug() {
        let title = best_effort_article_title(
            None,
            "https://epochai.substack.com/p/hyperscaler-capex-has-quadrupled?x=1",
        );
        assert_eq!(title.as_deref(), Some("Hyperscaler Capex Has Quadrupled"));
    }
}
