use std::collections::{BTreeMap, BTreeSet};

use harvester_core::signal_key_merge::{
    group_signal_keys,
    report::{build_grouping_report, parse_archive_index, ArchiveIndexRow, ReportExpectations},
};

const INDEX: &str = include_str!("fixtures/signal_key_merge/archive-2026-10-07.index.txt");
const EXPECTATIONS: &str = include_str!("fixtures/signal_key_merge/expectations.json");
const REPORT: &str = include_str!("fixtures/signal_key_merge/report-2026-10-07.txt");

fn fixture() -> (Vec<ArchiveIndexRow>, ReportExpectations) {
    let rows = parse_archive_index(INDEX).unwrap();
    assert_eq!(rows.len(), 197);
    assert_eq!(
        rows.iter().map(|row| row.doc).collect::<Vec<_>>(),
        (1..=197).collect::<Vec<_>>()
    );
    (rows, serde_json::from_str(EXPECTATIONS).unwrap())
}

#[test]
fn golden_grouping_report_and_separation_guards() {
    let (rows, expectations) = fixture();
    let by_doc: BTreeMap<_, _> = rows.iter().map(|row| (row.doc, row)).collect();
    let groups = group_signal_keys(rows.iter().map(|row| &row.signal_key));
    for family in &expectations.families {
        assert!(
            family.docs.len() >= 2,
            "empty/trivial family {}",
            family.name
        );
        assert_eq!(
            family.docs.len(),
            family.docs.iter().collect::<BTreeSet<_>>().len(),
            "duplicate docs in {}",
            family.name
        );
        for doc in &family.docs {
            assert!(by_doc.contains_key(doc), "missing family doc {doc}");
        }
        for doc in &family.floor {
            assert!(
                family.docs.contains(doc),
                "floor doc {doc} outside {}",
                family.name
            );
        }
        // one_group and floor subsets are recorded targets, currently inactive.
    }
    for case in &expectations.separate {
        assert!(
            case.sets.len() >= 2,
            "trivial separation case {}",
            case.name
        );
        for (index, left) in case.sets.iter().enumerate() {
            assert!(!left.is_empty(), "empty set in {}", case.name);
            for right in &case.sets[index + 1..] {
                for a in left {
                    for b in right {
                        let a = by_doc.get(a).expect("real case doc exists");
                        let b = by_doc.get(b).expect("real case doc exists");
                        assert_ne!(
                            groups.group_id(&a.signal_key).unwrap(),
                            groups.group_id(&b.signal_key).unwrap(),
                            "{}: docs {} and {} merged",
                            case.name,
                            a.doc,
                            b.doc
                        );
                    }
                }
            }
        }
    }
    // Run key-only guards both alone and in the full real + synthetic population.
    let all = group_signal_keys(
        rows.iter().map(|row| row.signal_key.as_str()).chain(
            expectations
                .synthetic
                .iter()
                .chain(&expectations.synthetic_merge)
                .flat_map(|case| case.keys.iter().map(String::as_str)),
        ),
    );
    for case in &expectations.synthetic {
        assert!(case.keys.len() >= 2, "trivial synthetic case {}", case.name);
        let alone = group_signal_keys(&case.keys);
        for groups in [&alone, &all] {
            for (index, a) in case.keys.iter().enumerate() {
                for b in &case.keys[index + 1..] {
                    assert_ne!(
                        groups.group_id(a).unwrap(),
                        groups.group_id(b).unwrap(),
                        "{}: {a} merged with {b}",
                        case.name
                    );
                }
            }
        }
    }
    // synthetic_merge targets are reported, currently inactive.
    let actual = build_grouping_report(&rows, &groups, &expectations).unwrap();
    assert!(
        actual == REPORT.replace("\r\n", "\n"),
        "grouping report mismatch; actual report follows:\n{actual}"
    );
}

#[test]
fn index_parser_accepts_crlf_and_trailing_indexes_without_splitting_titles() {
    assert_eq!(
        parse_archive_index(INDEX).unwrap(),
        parse_archive_index(&INDEX.replace("\r\n", "\n").replace('\n', "\r\n")).unwrap()
    );
    let text = "ignored body\n===== ARCHIVE INDEX =====\nexport_schema: 2\ndoc | line | fetched_utc | priority | signal_key | title\n1 | 1 | 2026-10-07T00:00:00Z | 4 | a-key | Title | with a pipe\n===== INDEX END =====\n";
    let rows = parse_archive_index(text).unwrap();
    assert_eq!(rows[0].title, "Title | with a pipe");
    assert!(parse_archive_index("no header").is_err());
    assert!(parse_archive_index(&text.replace("1 | 1 |", "bad | 1 |")).is_err());
}

#[test]
fn report_lists_every_member_of_an_existing_alias_group() {
    let text = "doc | line | fetched_utc | priority | signal_key | title\n1 | 1 | date | 4 | openai-frontier-model-access-restriction | First title\n2 | 20 | date | 4 | openai-frontier-model-access-blocked | Second title\n";
    let rows = parse_archive_index(text).unwrap();
    let groups = group_signal_keys(rows.iter().map(|row| &row.signal_key));
    let report = build_grouping_report(&rows, &groups, &ReportExpectations::default()).unwrap();
    assert!(report.contains("Groups before: 1\nGroups after: 1"));
    assert!(report.contains("doc 1 | openai-frontier-model-access-restriction | First title"));
    assert!(report.contains("doc 2 | openai-frontier-model-access-blocked | Second title"));
}
