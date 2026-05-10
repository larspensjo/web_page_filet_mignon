use harvester_engine::SourceKind;

use crate::{LlmQuotaSeverity, PollQuotaWarning, SourcePollStat};

/// Formats a grouped poll-stats summary (RSS / Brave / other source types) as a
/// multi-line string.
///
/// Returns `"No poll data yet."` when `stats` is empty.
/// Source kinds with no stats are omitted.
/// No surrounding banner is included; that is the caller's responsibility.
pub fn format_poll_stats(stats: &[SourcePollStat]) -> String {
    format_poll_stats_with_warning(stats, None)
}

pub fn format_poll_stats_with_warning(
    stats: &[SourcePollStat],
    warning: Option<&PollQuotaWarning>,
) -> String {
    if stats.is_empty() {
        return "No poll data yet.".to_string();
    }

    let groups: &[(SourceKind, &str)] = &[
        (SourceKind::Rss, "RSS"),
        (SourceKind::Brave, "Brave"),
        (SourceKind::File, "File"),
        (SourceKind::Curated, "Curated"),
        (SourceKind::Script, "Script"),
    ];

    let mut sections: Vec<String> = Vec::new();
    for (kind, label) in groups {
        let group: Vec<_> = stats.iter().filter(|s| s.kind == *kind).collect();
        if group.is_empty() {
            continue;
        }
        let total_emitted: usize = group.iter().map(|s| s.emitted).sum();
        let total_filtered: usize = group.iter().map(|s| s.dedup_filtered).sum();
        let mut lines = vec![
            format!("## {label}"),
            format!(
                "**{} source{}**. {} emitted, {} dedup-filtered.",
                group.len(),
                if group.len() == 1 { "" } else { "s" },
                total_emitted,
                total_filtered,
            ),
            String::new(),
        ];
        for s in &group {
            if s.parsed == 0 {
                lines.push(format!("- **{}**: 0 parsed", s.source_id));
            } else {
                lines.push(format!(
                    "- **{}**: {} parsed -> {} dedup-filtered -> {} emitted",
                    s.source_id, s.parsed, s.dedup_filtered, s.emitted
                ));
            }
        }
        sections.push(lines.join("\n"));
    }

    let formatted_stats = sections.join("\n\n");
    match warning {
        Some(warning) => format!(
            "{}\n\n{}",
            format_poll_quota_warning(warning),
            formatted_stats
        ),
        None => formatted_stats,
    }
}

fn format_poll_quota_warning(warning: &PollQuotaWarning) -> String {
    let severity = match warning.severity {
        LlmQuotaSeverity::Danger => "danger",
        LlmQuotaSeverity::Warning => "warning",
        LlmQuotaSeverity::Normal | LlmQuotaSeverity::Exhausted | LlmQuotaSeverity::Unavailable => {
            "warning"
        }
    };
    format!(
        "## LLM quota warning\n\nThe latest poll emitted {} URLs. This is an upper-bound estimate and may require up to {} triage LLM calls before summaries or briefing generation.\n\nLLM calls remaining this session: {} / {}.\nSummary and briefing calls use the same internal quota.\nRestart harvester_app to reset the internal session quota, or reduce the batch before running AI workflows.\n\nSeverity: {}.",
        warning.estimated_triage_calls,
        warning.estimated_triage_calls,
        warning.remaining_calls,
        warning.max_calls,
        severity
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use harvester_engine::SourceId;

    fn stat(
        source_id: &str,
        kind: SourceKind,
        parsed: usize,
        dedup_filtered: usize,
        emitted: usize,
    ) -> SourcePollStat {
        SourcePollStat {
            source_id: SourceId::new(source_id).expect("valid"),
            kind,
            parsed,
            dedup_filtered,
            emitted,
        }
    }

    #[test]
    fn empty_stats_returns_empty_state() {
        assert_eq!(format_poll_stats(&[]), "No poll data yet.");
    }

    #[test]
    fn formats_groups_as_markdown_sections_and_bullets() {
        let formatted = format_poll_stats(&[
            stat("google-alerts-rss", SourceKind::Rss, 20, 0, 20),
            stat("venturebeat-rss", SourceKind::Rss, 7, 1, 6),
            stat("brave-orbital-compute", SourceKind::Brave, 5, 0, 5),
            stat("brave-ai-data-center", SourceKind::Brave, 0, 0, 0),
        ]);

        assert!(formatted.contains("## RSS"));
        assert!(formatted.contains("**2 sources**. 26 emitted, 1 dedup-filtered."));
        assert!(formatted
            .contains("- **google-alerts-rss**: 20 parsed -> 0 dedup-filtered -> 20 emitted"));
        assert!(
            formatted.contains("- **venturebeat-rss**: 7 parsed -> 1 dedup-filtered -> 6 emitted")
        );
        assert!(formatted.contains("\n\n## Brave\n"));
        assert!(formatted.contains("- **brave-ai-data-center**: 0 parsed"));
    }

    #[test]
    fn can_prepend_quota_warning() {
        let warning = PollQuotaWarning {
            severity: LlmQuotaSeverity::Danger,
            estimated_triage_calls: 103,
            remaining_calls: 0,
            max_calls: 100,
        };

        let formatted = format_poll_stats_with_warning(
            &[stat("google-alerts-rss", SourceKind::Rss, 103, 0, 103)],
            Some(&warning),
        );

        assert!(formatted.starts_with("## LLM quota warning"));
        assert!(formatted.contains("latest poll emitted 103 URLs"));
        assert!(formatted.contains("LLM calls remaining this session: 0 / 100."));
        assert!(formatted.contains("\n\n## RSS\n"));
    }
}
