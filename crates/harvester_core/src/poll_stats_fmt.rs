use harvester_engine::SourceKind;

use crate::SourcePollStat;

/// Formats a grouped poll-stats summary (RSS / Brave / other source types) as a
/// multi-line string.
///
/// Returns `"No poll data yet."` when `stats` is empty.
/// Source kinds with no stats are omitted.
/// No surrounding banner is included; that is the caller's responsibility.
pub fn format_poll_stats(stats: &[SourcePollStat]) -> String {
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

    sections.join("\n\n")
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
        assert!(formatted.contains("- **google-alerts-rss**: 20 parsed -> 0 dedup-filtered -> 20 emitted"));
        assert!(formatted.contains("- **venturebeat-rss**: 7 parsed -> 1 dedup-filtered -> 6 emitted"));
        assert!(formatted.contains("\n\n## Brave\n"));
        assert!(formatted.contains("- **brave-ai-data-center**: 0 parsed"));
    }
}
