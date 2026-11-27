use crate::archive_observation::observe_archive_target;
use crate::persistence::{load_checkpoint_record, save_portfolio_record};
use crate::{load_briefing_checkpoint, save_briefing_checkpoint};
use harvester_core::morning_archive::*;
use serde::Deserialize;
use std::{
    fs,
    path::{Path, PathBuf},
};
use tempfile::TempDir;

const RAW: &str =
    include_str!("../../harvester_engine/tests/fixtures/archive_export/schema2_raw.md");
const TRUNCATED: &str = include_str!(
    "../../harvester_engine/tests/fixtures/archive_export/schema2_truncated_fallback.md"
);
const START: &str = "2026-09-10T01:00:00.123456Z";
const END: &str = "2026-10-03T10:00:00Z";
const LATER: &str = "2026-10-03T11:00:00Z";
const OLD_COLUMNS: &str = "doc | line | fetched_utc | priority | signal_key | title";

fn record(start: &str) -> PortfolioRecord {
    PortfolioRecord {
        confirmed_until_utc: Some(start.parse().unwrap()),
        last_archive_until_utc: Some(END.parse().unwrap()),
        ..Default::default()
    }
}

fn prepare_archive(bytes: &str, until: Option<&str>) -> (TempDir, PathBuf) {
    let dir = TempDir::new().unwrap();
    let target = dir.path().join("archive.md");
    fs::write(&target, bytes).unwrap();
    let sidecar = serde_json::json!({"until_utc":until,"doc_count":3,"commit":"abc"});
    fs::write(dir.path().join("archive.md.processed"), sidecar.to_string()).unwrap();
    (dir, target)
}

fn windowed(old_start: &str) -> String {
    legacy_raw().replace(
        OLD_COLUMNS,
        &format!("window_since_utc: {old_start}\nwindow_until_utc: {END}\n{OLD_COLUMNS}"),
    )
}

fn legacy_raw() -> String {
    RAW.lines()
        .filter(|line| !line.starts_with("window_"))
        .collect::<Vec<_>>()
        .join("\n")
}

fn resolve(target: &Path, start: &str) -> ResolvedWindow {
    let key = target_key(target);
    resolve_window(
        &BriefingCheckpoint {
            since_utc: Some(start.into()),
            portfolio: [(key.clone(), record(start))].into(),
        },
        &key,
        &observe_archive_target(target),
        LATER.parse().unwrap(),
    )
    .unwrap()
}

#[test]
fn actual_windowed_archive_replaces_when_durable_record_confirms_it() {
    let (_dir, target) = prepare_archive(&windowed(START), None);
    assert!(matches!(
        resolve(&target, END).disposition,
        PreviousArchiveDisposition::Replace { .. }
    ));
}

#[test]
fn actual_windowed_archive_kept_and_superseded_when_start_is_equal() {
    let (_dir, target) = prepare_archive(&windowed(START), None);
    let resolved = resolve(&target, START);
    assert_eq!(resolved.disposition, PreviousArchiveDisposition::KeepAside);
    assert!(resolved.previous_superseded);
}

#[test]
fn actual_kept_archive_is_not_superseded_when_its_start_precedes_new_window() {
    let (_dir, target) = prepare_archive(&windowed(START), None);
    let resolved = resolve(&target, "2026-09-12T01:00:00Z");
    assert_eq!(resolved.disposition, PreviousArchiveDisposition::KeepAside);
    assert!(!resolved.previous_superseded);
}

#[test]
fn legacy_schema2_replaces_with_valid_sidecar_including_exact_bootstrap() {
    let (_dir, target) = prepare_archive(&legacy_raw(), Some(END));
    let key = target_key(&target);
    let new = BriefingCheckpoint {
        since_utc: Some(END.into()),
        ..Default::default()
    };
    let resolved = resolve_window(
        &new,
        &key,
        &observe_archive_target(&target),
        LATER.parse().unwrap(),
    )
    .unwrap();
    assert_eq!(
        resolved.since_utc,
        END.parse::<chrono::DateTime<chrono::Utc>>().unwrap()
    );
    assert!(matches!(
        resolved.disposition,
        PreviousArchiveDisposition::Replace { .. }
    ));
}

#[test]
fn legacy_schema2_is_kept_with_null_sidecar_even_if_doc_count_matches() {
    let (_dir, target) = prepare_archive(&legacy_raw(), None);
    assert_eq!(
        resolve(&target, END).disposition,
        PreviousArchiveDisposition::KeepAside
    );
}

#[test]
fn actual_legacy_schema1_replaces_with_valid_sidecar() {
    let legacy = "===== DOC START =====\nurl: https://example.com/old\nfetched_utc: 2026-09-11T07:00:00Z\n\narticle\n===== DOC END =====\n";
    let (_dir, target) = prepare_archive(legacy, Some(END));
    assert!(matches!(
        resolve(&target, START).disposition,
        PreviousArchiveDisposition::Replace { .. }
    ));
}

#[test]
fn actual_truncated_schema2_is_kept_even_with_a_valid_sidecar() {
    let (_dir, target) = prepare_archive(
        TRUNCATED.split("===== INDEX END =====").next().unwrap(),
        Some(END),
    );
    assert_eq!(
        resolve(&target, END).disposition,
        PreviousArchiveDisposition::KeepAside
    );
}

#[test]
fn schema2_row_offsets_and_trailing_index_are_validated() {
    let windowed = windowed(START);
    let malformed = windowed.replace("1 | 1 |", "1 | 2 |");
    assert_ne!(windowed, malformed);
    let (_dir, target) = prepare_archive(&malformed, Some(END));
    assert_eq!(
        resolve(&target, END).disposition,
        PreviousArchiveDisposition::KeepAside
    );
}

#[test]
fn actual_sidecar_read_distinguishes_missing_unreadable_null_and_bad_json() {
    let (dir, target) = prepare_archive(RAW, None);
    let path = dir.path().join("archive.md.processed");
    assert_eq!(
        observe_archive_target(&target).sidecar,
        SidecarObservation::Parsed(None)
    );
    fs::write(
        &path,
        r#"{"until_utc":"invalid","doc_count":3,"commit":"abc"}"#,
    )
    .unwrap();
    assert_eq!(
        observe_archive_target(&target).sidecar,
        SidecarObservation::Unparseable
    );
    fs::write(&path, "{").unwrap();
    assert_eq!(
        observe_archive_target(&target).sidecar,
        SidecarObservation::Unparseable
    );
    fs::remove_file(&path).unwrap();
    assert_eq!(
        observe_archive_target(&target).sidecar,
        SidecarObservation::Missing
    );
    fs::create_dir(&path).unwrap();
    assert_eq!(
        observe_archive_target(&target).sidecar,
        SidecarObservation::Unreadable
    );
}

#[test]
fn observes_only_the_index_and_document_headers_for_reconciliation() {
    let forged = include_str!(
        "../../harvester_engine/tests/fixtures/archive_export/schema2_boundary_forgery.md"
    );
    let (_dir, target) = prepare_archive(forged, None);
    assert!(matches!(
        observe_archive_target(&target).archive,
        ObservedArchive::Present {
            index: ArchiveIndex::Schema2 { .. },
            ..
        }
    ));
}

#[test]
fn set_set_now_and_clear_keep_every_portfolio_record() {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join(".briefing_checkpoint.ron");
    let mut a = record(START);
    a.pending_until_utc = Some(LATER.parse().unwrap());
    let b = record(END);
    save_portfolio_record(&path, "a", &a, CheckpointWrite::Unchanged).unwrap();
    save_portfolio_record(&path, "b", &b, CheckpointWrite::Unchanged).unwrap();
    for since in [Some(START), Some(LATER), None] {
        save_briefing_checkpoint(&path, since).unwrap();
        assert!(path.is_file());
        assert_eq!(load_briefing_checkpoint(&path).as_deref(), since);
        let checkpoint = load_checkpoint_record(&path).unwrap();
        assert_eq!(
            checkpoint.portfolio,
            [("a".into(), a.clone()), ("b".into(), b.clone())].into()
        );
    }
}

#[test]
fn old_checkpoint_without_map_loads_with_an_empty_map() {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join(".briefing_checkpoint.ron");
    fs::write(&path, format!("(since_utc: Some(\"{START}\"))")).unwrap();
    let checkpoint = load_checkpoint_record(&path).unwrap();
    assert_eq!(checkpoint.since_utc.as_deref(), Some(START));
    assert!(checkpoint.portfolio.is_empty());
}

#[test]
fn legacy_strikes_load_without_losing_checkpoint_fields_and_are_not_written() {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join(".briefing_checkpoint.ron");
    fs::write(
        &path,
        format!(
            r#"(since_utc: Some("{START}"), portfolio: {{
                "a": (confirmed_until_utc: Some("{START}"), last_archive_until_utc: Some("{END}"),
                      pending_until_utc: Some("{LATER}"), strikes: {{"https://example.com/retry": (count: 2, last_until: "{END}")}}),
                "b": (confirmed_until_utc: Some("{END}"), last_archive_until_utc: None, pending_until_utc: None),
            }})"#
        ),
    )
    .unwrap();
    let checkpoint = load_checkpoint_record(&path).unwrap();
    assert_eq!(checkpoint.since_utc.as_deref(), Some(START));
    assert_eq!(checkpoint.portfolio.len(), 2);
    assert_eq!(
        checkpoint.portfolio["a"].confirmed_until_utc,
        Some(START.parse().unwrap())
    );
    assert_eq!(
        checkpoint.portfolio["a"].last_archive_until_utc,
        Some(END.parse().unwrap())
    );
    assert_eq!(
        checkpoint.portfolio["a"].pending_until_utc,
        Some(LATER.parse().unwrap())
    );
    assert_eq!(
        checkpoint.portfolio["b"].confirmed_until_utc,
        Some(END.parse().unwrap())
    );

    save_briefing_checkpoint(&path, Some(START)).unwrap();
    assert!(!fs::read_to_string(&path).unwrap().contains("strikes"));
    let rewritten = load_checkpoint_record(&path).unwrap();
    assert_eq!(rewritten, checkpoint);
}

#[test]
fn confirmed_windows_prune_persisted_omissions_across_many_mornings() {
    use chrono::{DateTime, TimeDelta, Utc};

    let dir = TempDir::new().unwrap();
    let path = dir.path().join(".briefing_checkpoint.ron");
    let key = "target";
    let mut since: DateTime<Utc> = START.parse().unwrap();
    let mut record = PortfolioRecord {
        confirmed_until_utc: Some(since),
        ..Default::default()
    };
    record.left_out.insert("undated".into(), None);
    record
        .left_out
        .insert("boundary".into(), Some(since + TimeDelta::days(1)));

    for morning in 0..32 {
        let until = since + TimeDelta::days(1);
        let url = format!("https://example.com/morning-{morning}");
        record.pending_until_utc = Some(until);
        record
            .pending_left_out
            .insert(url.clone(), Some(since + TimeDelta::hours(1)));
        save_portfolio_record(&path, key, &record, CheckpointWrite::Unchanged).unwrap();
        let checkpoint = load_checkpoint_record(&path).unwrap();
        assert!(checkpoint.portfolio[key].left_out.len() <= 2);
        assert_eq!(checkpoint.portfolio[key].pending_left_out.len(), 1);

        // Recover publication intent and advance with the corresponding consumer mark.
        let observed = TargetObservation {
            sidecar: SidecarObservation::Parsed(Some(until)),
            archive: ObservedArchive::Present {
                stamp: None,
                index: ArchiveIndex::Schema2 {
                    window_since_utc: Some(since),
                    window_until_utc: Some(until),
                    fetched_from: None,
                    fetched_to: None,
                },
            },
        };
        let resolved = resolve_window(&checkpoint, key, &observed, until).unwrap();
        assert_eq!(resolved.since_utc, until);
        assert_eq!(resolved.sidecar_status, SidecarStatus::Advanced);
        save_portfolio_record(&path, key, &resolved.record, CheckpointWrite::Unchanged).unwrap();
        record = load_checkpoint_record(&path)
            .unwrap()
            .portfolio
            .remove(key)
            .unwrap();
        assert!(record.pending_left_out.is_empty());
        assert!(!record.left_out.contains_key(&url));
        assert_eq!(record.left_out.get("undated"), Some(&None));
        assert_eq!(record.left_out.len(), if morning == 0 { 2 } else { 1 });
        assert_eq!(
            record.left_out.get("boundary"),
            (morning == 0).then_some(&Some(until))
        );
        since = until;
    }
}

#[test]
fn checkpoint_with_map_remains_readable_by_old_struct_shape() {
    #[derive(Deserialize)]
    struct OldShape {
        since_utc: Option<String>,
    }
    let dir = TempDir::new().unwrap();
    let path = dir.path().join(".briefing_checkpoint.ron");
    save_portfolio_record(
        &path,
        "a",
        &record(START),
        CheckpointWrite::Set(END.parse().unwrap()),
    )
    .unwrap();
    let old: OldShape = ron::from_str(&fs::read_to_string(&path).unwrap()).unwrap();
    assert_eq!(old.since_utc.as_deref(), Some(END));
}

#[test]
fn desktop_write_sets_checkpoint_and_preserves_unreadable_record_map_aside() {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join(".briefing_checkpoint.ron");
    let bad = "(since_utc: None, portfolio: { \"a\": (";
    fs::write(&path, bad).unwrap();
    save_briefing_checkpoint(&path, Some(END)).unwrap();
    assert_eq!(load_briefing_checkpoint(&path).as_deref(), Some(END));
    let aside = unreadable_checkpoints(dir.path());
    assert_eq!(aside.len(), 1);
    assert_eq!(fs::read(&aside[0]).unwrap(), bad.as_bytes());
}

fn unreadable_checkpoints(dir: &Path) -> Vec<PathBuf> {
    fs::read_dir(dir)
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .filter(|path| {
            let name = path.file_name().unwrap().to_string_lossy();
            name.starts_with(".briefing_checkpoint.unreadable-") && name.ends_with(".ron")
        })
        .collect()
}

fn undecodable_map() -> String {
    format!("(\n    since_utc: Some(\"{START}\"),\n    portfolio: {{\"a\": (confirmed_until_utc: Some(\"not-a-date\"))}},\n)")
}

#[test]
fn checkpoint_load_recovers_since_and_logs_the_parse_position_without_renaming() {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join(".briefing_checkpoint.ron");
    let bad = undecodable_map();
    fs::write(&path, &bad).unwrap();
    let (checkpoint, logs) = crate::test_logs::capture(|| load_checkpoint_record(&path));
    let checkpoint = checkpoint.unwrap();
    assert_eq!(checkpoint.since_utc.as_deref(), Some(START));
    assert!(checkpoint.portfolio.is_empty());
    assert_eq!(load_briefing_checkpoint(&path).as_deref(), Some(START));
    assert_eq!(fs::read_to_string(&path).unwrap(), bad);
    assert!(unreadable_checkpoints(dir.path()).is_empty());
    let parse_error = ron::from_str::<BriefingCheckpoint>(&bad).unwrap_err();
    assert!(
        logs.iter()
            .any(|line| line.contains(&parse_error.span.to_string())
                && line.contains(START)
                && line.contains(&*path.to_string_lossy())),
        "{logs:?}"
    );
}

#[test]
fn record_write_with_an_undecodable_map_keeps_the_recovered_since() {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join(".briefing_checkpoint.ron");
    let bad = undecodable_map();
    fs::write(&path, &bad).unwrap();
    let new_record = record(END);
    let (saved, logs) = crate::test_logs::capture(|| {
        save_portfolio_record(&path, "new-target", &new_record, CheckpointWrite::Unchanged)
    });
    saved.unwrap();
    let checkpoint = load_checkpoint_record(&path).unwrap();
    assert_eq!(checkpoint.since_utc.as_deref(), Some(START));
    assert_eq!(checkpoint.portfolio["new-target"], new_record);
    let aside = unreadable_checkpoints(dir.path());
    assert_eq!(aside.len(), 1);
    assert_eq!(fs::read_to_string(&aside[0]).unwrap(), bad);
    assert!(
        logs.iter()
            .any(|line| line.contains(&*path.to_string_lossy())
                && line.contains(&*aside[0].to_string_lossy())
                && line.contains(START)),
        "{logs:?}"
    );
}

#[test]
fn checkpoint_recovery_preserves_existing_aside_names_on_collision() {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join(".briefing_checkpoint.ron");
    let mut existing = Vec::new();
    let now = chrono::Utc::now();
    for seconds in -2..=5 {
        let stamp = (now + chrono::TimeDelta::seconds(seconds)).format("%Y%m%dT%H%M%SZ");
        let aside = dir
            .path()
            .join(format!(".briefing_checkpoint.unreadable-{stamp}.ron"));
        fs::write(&aside, "previous aside").unwrap();
        existing.push(aside);
    }
    fs::write(&path, b"invalid RON").unwrap();
    save_briefing_checkpoint(&path, Some(END)).unwrap();
    for aside in &existing {
        assert_eq!(fs::read_to_string(aside).unwrap(), "previous aside");
    }
    let saved: Vec<_> = unreadable_checkpoints(dir.path())
        .into_iter()
        .filter(|aside| !existing.contains(aside))
        .collect();
    assert_eq!(saved.len(), 1);
    assert!(saved[0]
        .file_name()
        .unwrap()
        .to_string_lossy()
        .ends_with("-2.ron"));
    assert_eq!(fs::read(&saved[0]).unwrap(), b"invalid RON");
}
