//! Read-only benchmark parsing and deterministic audit reports.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt::Write;

use harvester_engine::archive_url_key;
use serde::Deserialize;

use super::{group_signal_keys, SignalKeyGroups};
use crate::signal_candidate::canonical_signal_key;

const HEADER: &str = "doc | line | fetched_utc | priority | signal_key | title";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ArchiveIndexRow {
    pub doc: usize,
    pub line: usize,
    pub fetched_utc: String,
    pub priority: String,
    pub signal_key: String,
    pub title: String,
}

/// Parses a standalone index fixture or the authoritative trailing archive index.
/// `str::lines` accepts both LF and CRLF checkouts; titles may contain pipes.
pub fn parse_archive_index(text: &str) -> Result<Vec<ArchiveIndexRow>, String> {
    let all_lines: Vec<_> = text.lines().collect();
    let start = all_lines
        .iter()
        .rposition(|line| *line == "===== ARCHIVE INDEX =====")
        .map_or(0, |index| index + 1);
    let mut lines = all_lines[start..].iter().copied();
    if !lines.by_ref().any(|line| line == HEADER) {
        return Err("archive index column header not found".into());
    }
    let mut rows = Vec::new();
    let mut docs = BTreeSet::new();
    for line in lines {
        if line == "===== INDEX END =====" {
            break;
        }
        if line.is_empty() {
            continue;
        }
        let columns: Vec<_> = line.splitn(6, " | ").collect();
        if columns.len() != 6 {
            return Err(format!("invalid index row: {line}"));
        }
        let doc = columns[0]
            .parse::<usize>()
            .map_err(|e| format!("invalid doc number in {line}: {e}"))?;
        let line_number = columns[1]
            .parse::<usize>()
            .map_err(|e| format!("invalid line number in {line}: {e}"))?;
        if doc == 0 || line_number == 0 || !docs.insert(doc) {
            return Err(format!("invalid or duplicate doc/line number: {line}"));
        }
        rows.push(ArchiveIndexRow {
            doc,
            line: line_number,
            fetched_utc: columns[2].into(),
            priority: columns[3].into(),
            signal_key: columns[4].into(),
            title: columns[5].into(),
        });
    }
    Ok(rows)
}

/// Only inspect document headers: body URLs never imply export membership.
pub fn exported_docs(archive: &str) -> Result<BTreeMap<String, usize>, String> {
    let mut docs = BTreeMap::new();
    let mut in_header = false;
    let mut url: Option<String> = None;
    let mut doc = None;
    for line in archive.lines() {
        if line == "===== DOC START =====" {
            in_header = true;
            url = None;
            doc = None;
        } else if in_header && line.is_empty() {
            if let (Some(url), Some(doc)) = (url.take(), doc.take()) {
                docs.insert(archive_url_key(&url), doc);
            }
            in_header = false;
        } else if in_header {
            if let Some(value) = line.strip_prefix("url: ") {
                url = Some(value.to_string());
            }
            if let Some(value) = line.strip_prefix("doc: ") {
                doc = Some(
                    value
                        .parse::<usize>()
                        .map_err(|err| format!("invalid archive doc: {err}"))?,
                );
            }
        }
    }
    Ok(docs)
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FamilyTarget {
    OneGroup,
    Report,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FamilyExpectation {
    pub name: String,
    pub docs: Vec<usize>,
    pub target: FamilyTarget,
    #[serde(default)]
    pub floor: Vec<usize>,
}

/// Every pair from different sets must remain separate. Members within a set
/// may merge (for example, a loan family versus a power-purchase agreement).
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SeparateExpectation {
    pub name: String,
    pub sets: Vec<Vec<usize>>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SyntheticExpectation {
    pub name: String,
    pub keys: Vec<String>,
    #[serde(default)]
    pub veto: bool,
}

#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ReportExpectations {
    pub families: Vec<FamilyExpectation>,
    pub separate: Vec<SeparateExpectation>,
    pub synthetic: Vec<SyntheticExpectation>,
    pub synthetic_merge: Vec<SyntheticExpectation>,
}

fn distinct_groups<'a>(
    keys: impl IntoIterator<Item = &'a str>,
    groups: &'a SignalKeyGroups,
) -> Result<usize, String> {
    keys.into_iter()
        .map(|key| {
            groups
                .group_id(key)
                .ok_or_else(|| format!("key missing from groups: {key}"))
        })
        .collect::<Result<BTreeSet<_>, _>>()
        .map(|set| set.len())
}

/// Targets are printed for review and are not enforced here. Separation guards
/// are enforced by the golden test independently of the snapshot.
pub fn build_grouping_report(
    rows: &[ArchiveIndexRow],
    groups: &SignalKeyGroups,
    expectations: &ReportExpectations,
) -> Result<String, String> {
    let by_doc: BTreeMap<_, _> = rows.iter().map(|row| (row.doc, row)).collect();
    let keys_for = |docs: &[usize]| -> Result<Vec<&str>, String> {
        docs.iter()
            .map(|doc| {
                by_doc
                    .get(doc)
                    .map(|row| row.signal_key.as_str())
                    .ok_or_else(|| format!("expectation references missing doc {doc}"))
            })
            .collect()
    };
    let before = rows
        .iter()
        .filter(|row| !row.signal_key.is_empty())
        .map(|row| canonical_signal_key(&row.signal_key))
        .collect::<BTreeSet<_>>()
        .len();
    let mut report = format!(
        "Archive signal-key grouping (selected-index benchmark)\nRows: {}\nGroups before: {before}\nGroups after: {}\n\nFamilies:\n",
        rows.len(), groups.groups().len()
    );
    for family in &expectations.families {
        let keys = keys_for(&family.docs)?;
        let before = keys
            .iter()
            .map(|key| canonical_signal_key(key))
            .collect::<BTreeSet<_>>()
            .len();
        let after = distinct_groups(keys.iter().copied(), groups)?;
        let target = match family.target {
            FamilyTarget::OneGroup => "one_group",
            FamilyTarget::Report => "report",
        };
        writeln!(
            report,
            "- {}: {before} keys -> {after} groups; {target}; docs {:?}",
            family.name, family.docs
        )
        .unwrap();
        if !family.floor.is_empty() {
            let floor_keys = keys_for(&family.floor)?;
            writeln!(
                report,
                "  floor {:?}: {} groups; one_group",
                family.floor,
                distinct_groups(floor_keys, groups)?
            )
            .unwrap();
        }
    }
    writeln!(
        report,
        "\nSeparation guards: {} real cases / {} synthetic cases ({} marker vetoes)",
        expectations.separate.len(),
        expectations.synthetic.len(),
        expectations
            .synthetic
            .iter()
            .filter(|case| case.veto)
            .count()
    )
    .unwrap();
    report.push_str("\nSynthetic merge targets:\n");
    for case in &expectations.synthetic_merge {
        let groups = group_signal_keys(&case.keys);
        writeln!(
            report,
            "- {}: {} keys -> {} groups; {:?}",
            case.name,
            case.keys.len(),
            groups.groups().len(),
            case.keys
        )
        .unwrap();
    }
    report.push_str("\nMulti-member groups:\n");
    let mut members: BTreeMap<&str, Vec<&ArchiveIndexRow>> = BTreeMap::new();
    for row in rows.iter().filter(|row| !row.signal_key.is_empty()) {
        let group = groups
            .group_id(&row.signal_key)
            .ok_or_else(|| format!("doc {} missing from groups", row.doc))?;
        members.entry(group).or_default().push(row);
    }
    let mut count = 0;
    for (group, mut rows) in members {
        if rows.len() < 2 {
            continue;
        }
        count += 1;
        rows.sort_by_key(|row| row.doc);
        writeln!(report, "- {group}").unwrap();
        for row in rows {
            writeln!(
                report,
                "  doc {} | {} | {}",
                row.doc, row.signal_key, row.title
            )
            .unwrap();
        }
    }
    if count == 0 {
        report.push_str("(none)\n");
    }
    Ok(report)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn membership_uses_only_document_headers_and_canonical_urls() {
        let text = "===== DOC START =====\r\nurl: https://example.com/a#part\r\ndoc: 1\r\n\r\nurl: https://example.com/body\r\ndoc: 99\r\n===== DOC END =====\r\n";
        let docs = exported_docs(text).unwrap();
        assert_eq!(docs.len(), 1);
        assert_eq!(
            docs.get(&archive_url_key("https://example.com/a")),
            Some(&1)
        );
    }
}
