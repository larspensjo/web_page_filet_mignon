//! Preview content formatting and selection logic.
//!
//! Centralizes the logic for determining what content to show in the preview pane
//! and how to format it. Follows the Unidirectional Data Flow Architecture:
//! formatters are pure functions with no side effects.

use crate::briefing::ArticleSummaryResult;
use crate::pre_triage_filter::{ArticleFilterEntry, AutoVerdict, FilterReason};
use crate::triage::ArticleTriageResult;

/// Indicates the source/type of content displayed in the preview pane.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PreviewContentKind {
    /// Full summary from the briefing stage.
    Summary,
    /// Triage assessment (priority, category, rationale).
    Triage,
    /// Pre-triage exclusion reasons.
    Exclusion,
    /// Generic fallback when no analysis exists.
    Fallback,
}

/// Format a completed article summary for display in the preview pane.
pub fn format_summary_for_preview(summary: &ArticleSummaryResult) -> String {
    use std::fmt::Write;
    let mut out = String::new();
    let _ = writeln!(out, "# {}", summary.title);
    out.push('\n');
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

/// Format a triage assessment as human-readable markdown.
pub fn format_triage_for_preview(title: Option<&str>, result: &ArticleTriageResult) -> String {
    use std::fmt::Write;
    let mut out = String::new();
    let heading = title
        .map(str::trim)
        .filter(|title| !title.is_empty())
        .unwrap_or("Triage assessment");
    let _ = writeln!(out, "# {heading}");
    out.push('\n');
    let _ = writeln!(
        out,
        "{} · Priority P{}",
        title_case_label(&result.category),
        result.priority
    );
    out.push('\n');
    let chips: Vec<String> = result
        .tags
        .iter()
        .filter_map(|tag| sanitize_tag_for_chip(tag))
        .map(|tag| format!("`{tag}`"))
        .collect();
    if !chips.is_empty() {
        // Each chip is an inline code span so the RTF renderer shows them
        // as shaded chips in the preview pane. See sanitize_tag_for_chip
        // for why tag text must be normalized before wrapping in backticks.
        let _ = writeln!(out, "Tags: {}", chips.join(" "));
        out.push('\n');
    }
    let _ = writeln!(out, "## Why It Matters");
    out.push('\n');
    let _ = writeln!(out, "{}", result.rationale.trim());
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

/// Format pre-triage exclusion reasons for display.
pub fn format_exclusion_for_preview(entry: &ArticleFilterEntry) -> String {
    use std::fmt::Write;
    let mut out = String::new();
    let _ = writeln!(out, "# Not Included");
    out.push('\n');

    let decision_label = match (entry.auto_verdict, entry.manual_decision) {
        (_, Some(crate::pre_triage_filter::ManualDecision::Exclude)) => "Manually excluded",
        (AutoVerdict::HardExclude, None) => "Auto-excluded",
        (AutoVerdict::Review, None) => "Needs review",
        _ => "Excluded",
    };
    let _ = writeln!(out, "**Decision:** {}", decision_label);
    out.push('\n');

    if !entry.reasons.is_empty() {
        let _ = writeln!(out, "**Reasons:**");
        for reason in &entry.reasons {
            let _ = writeln!(out, "- {}", filter_reason_display(reason));
        }
        out.push('\n');
    }

    let _ = writeln!(
        out,
        "*Tip: Override in the pre-triage review panel to include this article.*"
    );
    out
}

/// Format the fallback message when no analysis is available.
pub fn format_fallback_preview() -> String {
    let mut out = String::new();
    out.push_str("# No Analysis Available Yet\n\n");
    out.push_str("This article has not been triaged or summarized.\n");
    out.push_str("Run Triage to assess article relevance, then Briefing to generate summaries.\n");
    out
}

/// Convert a FilterReason to a human-readable display string.
pub fn filter_reason_display(reason: &FilterReason) -> &'static str {
    match reason {
        FilterReason::BlockedHost => "Blocked host",
        FilterReason::VerySmallContent => "Very small content",
        FilterReason::PaywallShellTitle => "Paywall shell title",
        FilterReason::SmallMediumContent => "Small/medium content",
        FilterReason::BoilerplateDensity => "High boilerplate density",
        FilterReason::HighLinkDensity => "High link density",
        FilterReason::StubPhraseLowContent => "Stub phrase with low content",
    }
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

/// Prepare an LLM-produced tag string for rendering as a markdown inline
/// code chip. Strips backticks (which would close the span) and CR/LF
/// (which would break the paragraph), and trims surrounding whitespace.
/// Returns `None` if nothing remains.
fn sanitize_tag_for_chip(raw: &str) -> Option<String> {
    let cleaned: String = raw
        .chars()
        .map(|ch| match ch {
            '`' => '\u{2032}', // U+2032 PRIME — visually similar, inert in markdown
            '\n' | '\r' => ' ',
            other => other,
        })
        .collect();
    let trimmed = cleaned.trim();
    if trimmed.is_empty() {
        None
    } else {
        Some(trimmed.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn triage_formatter_produces_stable_markdown() {
        let result = ArticleTriageResult {
            category: "Security".to_string(),
            priority: 7,
            tags: vec!["vulnerability".to_string(), "zero-day".to_string()],
            rationale: "The article discusses a newly discovered vulnerability...".to_string(),
            input_tokens: 100,
            output_tokens: 50,
        };
        let formatted = format_triage_for_preview(Some("New vulnerability disclosed"), &result);
        assert!(formatted.contains("# New vulnerability disclosed"));
        assert!(formatted.contains("Security · Priority P7"));
        // Each tag is wrapped in a backtick span so the RTF preview renders
        // them as shaded chips (see markdown_to_rtf::convert_markdown_to_rtf).
        assert!(formatted.contains("Tags: `vulnerability` `zero-day`"));
        assert!(formatted.contains("## Why It Matters"));
        assert!(!formatted.contains("## Signals"));
        assert!(formatted.contains("newly discovered vulnerability"));
    }

    #[test]
    fn triage_formatter_no_json_leakage() {
        let result = ArticleTriageResult {
            category: "Security".to_string(),
            priority: 7,
            tags: vec![],
            rationale: "Test rationale".to_string(),
            input_tokens: 100,
            output_tokens: 50,
        };
        let formatted = format_triage_for_preview(None, &result);
        // Should not contain JSON-like braces from struct formatting
        assert!(!formatted.contains('{'));
        assert!(!formatted.contains('}'));
        assert!(formatted.contains("# Triage assessment"));
    }

    #[test]
    fn best_effort_article_title_falls_back_to_humanized_slug() {
        let title = best_effort_article_title(
            None,
            "https://epochai.substack.com/p/hyperscaler-capex-has-quadrupled?x=1",
        );
        assert_eq!(title.as_deref(), Some("Hyperscaler Capex Has Quadrupled"));
    }

    #[test]
    fn exclusion_formatter_includes_decision_source() {
        use crate::pre_triage_filter::{ArticleFilterKey, ManualDecision};

        let entry_auto = ArticleFilterEntry {
            key: ArticleFilterKey {
                url: "https://example.com".to_string(),
                content_hash: 12345,
            },
            source_title: Some("Test".to_string()),
            auto_verdict: AutoVerdict::HardExclude,
            reasons: vec![FilterReason::BlockedHost],
            manual_decision: None,
        };
        let formatted = format_exclusion_for_preview(&entry_auto);
        assert!(formatted.contains("Auto-excluded"));
        assert!(formatted.contains("- Blocked host"));

        let entry_manual = ArticleFilterEntry {
            key: ArticleFilterKey {
                url: "https://example.com".to_string(),
                content_hash: 12345,
            },
            source_title: Some("Test".to_string()),
            auto_verdict: AutoVerdict::Review,
            reasons: vec![],
            manual_decision: Some(ManualDecision::Exclude),
        };
        let formatted = format_exclusion_for_preview(&entry_manual);
        assert!(formatted.contains("Manually excluded"));
    }

    #[test]
    fn fallback_formatter_provides_guidance() {
        let formatted = format_fallback_preview();
        assert!(formatted.contains("No Analysis Available Yet"));
        assert!(formatted.contains("Triage"));
        assert!(formatted.contains("Briefing"));
    }

    #[test]
    fn triage_formatter_sanitizes_backticks_in_tag_text() {
        // A tag containing a literal backtick must not be allowed to close the
        // inline code span. Verify the output contains no adjacent backticks
        // (which would indicate a broken span) and that the sanitized form is
        // present.
        let result = ArticleTriageResult {
            category: "Security".to_string(),
            priority: 3,
            tags: vec!["back`tick".to_string(), "clean".to_string()],
            rationale: "test".to_string(),
            input_tokens: 0,
            output_tokens: 0,
        };
        let formatted = format_triage_for_preview(Some("t"), &result);
        // No "``" — that would mean a code span got closed and reopened.
        assert!(
            !formatted.contains("``"),
            "formatter must not emit adjacent backticks; got: {formatted}"
        );
        // Sanitized form: backtick replaced with U+2032 PRIME.
        assert!(formatted.contains("`back\u{2032}tick`"));
        assert!(formatted.contains("`clean`"));
    }

    #[test]
    fn triage_formatter_sanitizes_newlines_in_tag_text() {
        let result = ArticleTriageResult {
            category: "Security".to_string(),
            priority: 3,
            tags: vec!["line1\nline2".to_string(), "tab\rreturn".to_string()],
            rationale: "test".to_string(),
            input_tokens: 0,
            output_tokens: 0,
        };
        let formatted = format_triage_for_preview(Some("t"), &result);
        // The Tags: line stays on a single line; no stray CR/LF leaks into a chip.
        let tags_line = formatted
            .lines()
            .find(|line| line.starts_with("Tags:"))
            .expect("Tags: line present");
        assert!(!tags_line.contains('\r'));
        assert!(tags_line.contains("`line1 line2`"));
        assert!(tags_line.contains("`tab return`"));
    }

    #[test]
    fn triage_formatter_drops_empty_tags_after_sanitization() {
        let result = ArticleTriageResult {
            category: "Security".to_string(),
            priority: 3,
            tags: vec!["  ".to_string(), "keep".to_string(), "\n\n".to_string()],
            rationale: "test".to_string(),
            input_tokens: 0,
            output_tokens: 0,
        };
        let formatted = format_triage_for_preview(Some("t"), &result);
        let tags_line = formatted
            .lines()
            .find(|line| line.starts_with("Tags:"))
            .expect("Tags: line present");
        // Only the non-empty sanitized tag survives.
        assert_eq!(tags_line.trim(), "Tags: `keep`");
    }
}
