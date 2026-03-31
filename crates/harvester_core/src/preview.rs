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
pub fn format_triage_for_preview(result: &ArticleTriageResult) -> String {
    use std::fmt::Write;
    let mut out = String::new();
    let _ = writeln!(out, "# Triage Assessment");
    out.push('\n');
    let _ = writeln!(out, "**Priority:** {}/10", result.priority);
    let _ = writeln!(out, "**Category:** {}", result.category);
    if !result.tags.is_empty() {
        let _ = write!(out, "**Tags:** {}", result.tags.join(", "));
        out.push('\n');
    }
    out.push('\n');
    let _ = writeln!(out, "## Rationale");
    out.push('\n');
    let _ = writeln!(out, "{}", result.rationale);
    out
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
        let formatted = format_triage_for_preview(&result);
        assert!(formatted.contains("# Triage Assessment"));
        assert!(formatted.contains("**Priority:** 7/10"));
        assert!(formatted.contains("**Category:** Security"));
        assert!(formatted.contains("**Tags:** vulnerability, zero-day"));
        assert!(formatted.contains("## Rationale"));
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
        let formatted = format_triage_for_preview(&result);
        // Should not contain JSON-like braces from struct formatting
        assert!(!formatted.contains('{'));
        assert!(!formatted.contains('}'));
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
}
