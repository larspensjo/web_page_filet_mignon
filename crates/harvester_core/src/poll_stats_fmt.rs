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

    let mut lines: Vec<String> = Vec::new();
    for (kind, label) in groups {
        let group: Vec<_> = stats.iter().filter(|s| s.kind == *kind).collect();
        if group.is_empty() {
            continue;
        }
        let total_emitted: usize = group.iter().map(|s| s.emitted).sum();
        let total_filtered: usize = group.iter().map(|s| s.dedup_filtered).sum();
        lines.push(format!(
            "{} ({} source{}): {} emitted, {} dedup-filtered",
            label,
            group.len(),
            if group.len() == 1 { "" } else { "s" },
            total_emitted,
            total_filtered,
        ));
        for s in &group {
            if s.parsed == 0 {
                lines.push(format!("  {}: 0 parsed", s.source_id));
            } else {
                lines.push(format!(
                    "  {}: {} parsed \u{2192} {} dedup-filtered \u{2192} {} emitted",
                    s.source_id, s.parsed, s.dedup_filtered, s.emitted
                ));
            }
        }
    }

    lines.join("\n")
}
