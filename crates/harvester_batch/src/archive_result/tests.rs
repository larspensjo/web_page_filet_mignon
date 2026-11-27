use super::*;
use serde_json::Value;

const FIXTURE_COMMIT: &str = "0123456789abcdef0123456789ABCDEF01234567";

fn exported_left_out_line() -> String {
    ArchiveResult {
        status: Status::Exported,
        articles: 8,
        path: Some(PathBuf::from("c:/utgåva/📰.md")),
        previous_kept: Some(PathBuf::from("c:/utgåva/old.md")),
        previous_superseded: true,
        cost_microdollars: Some(412_305),
        since_utc: Some("2025-09-17T06:10:31.123456Z".parse().unwrap()),
        until_utc: Some("2025-09-18T06:12:04Z".parse().unwrap()),
        failed_sources: 1,
        left_out: vec![
            LeftOutArticle {
                url: "https://example.com/a".into(),
                title: Some("Utgåva 📰".into()),
                reason: "failed:summary+scoring".into(),
            },
            LeftOutArticle {
                url: "https://example.com/b".into(),
                title: None,
                reason: "not_finished".into(),
            },
        ],
        ..ArchiveResult::new(Status::Exported, None)
    }
    .line(Some(FIXTURE_COMMIT))
}

fn quota_halted_line() -> String {
    ArchiveResult {
        since_utc: Some("2025-09-17T06:10:31.123456Z".parse().unwrap()),
        cost_microdollars: Some(12_400),
        ..ArchiveResult::new(
            Status::CycleFailed,
            Some(harvester_core::morning_archive::BatchArchiveReason::QuotaHalted.as_str()),
        )
    }
    .line(Some(FIXTURE_COMMIT))
}

fn nothing_new_left_out_line() -> String {
    ArchiveResult {
        since_utc: Some("2025-09-17T06:10:31.123456Z".parse().unwrap()),
        cost_microdollars: Some(23_800),
        left_out: vec![LeftOutArticle {
            url: "https://example.com/a".into(),
            title: Some("An unfinished article".into()),
            reason: "failed:scoring".into(),
        }],
        ..ArchiveResult::new(Status::NothingNew, None)
    }
    .line(Some(FIXTURE_COMMIT))
}

#[test]
fn result_line_matches_quota_halted_and_nothing_new_fixtures() {
    assert_eq!(
        format!("{}\n", quota_halted_line()).as_bytes(),
        include_bytes!("../../tests/fixtures/result_line/quota_halted.txt")
    );
    assert_eq!(
        format!("{}\n", nothing_new_left_out_line()).as_bytes(),
        include_bytes!("../../tests/fixtures/result_line/nothing_new_left_out.txt")
    );
}

#[test]
fn result_line_is_ordered_ascii_json_with_exact_cost_and_utc_precision() {
    let line = exported_left_out_line();
    assert!(line.is_ascii());
    assert!(!line.contains('\n'));
    assert_eq!(
        format!("{line}\n").as_bytes(),
        include_bytes!("../../tests/fixtures/result_line/exported_left_out.txt")
    );
    let value: Value =
        serde_json::from_str(line.strip_prefix("HARVESTER_RESULT ").unwrap()).unwrap();
    assert_eq!(value["path"], "c:/utgåva/📰.md");
}

#[test]
fn result_line_includes_nulls_and_zero_counts_for_early_usage_failure() {
    let line = ArchiveResult::new(Status::UsageError, Some("invalid_arguments")).line(None);
    assert_eq!(
        line,
        concat!(
            r#"HARVESTER_RESULT {"v":2,"status":"usage_error","articles":0,"path":null,"#,
            r#""previous_kept":null,"previous_superseded":false,"cost_usd":null,"#,
            r#""cost_basis":"openai_price_table","#,
            r#""archive_schema":2,"since_utc":null,"until_utc":null,"failed_sources":0,"left_out_articles":0,"#,
            r#""left_out":[],"lock_holder":null,"reason":"invalid_arguments","commit":null}"#,
        )
    );
}

#[test]
fn result_line_omits_invalid_commit_values() {
    let result = ArchiveResult::new(Status::UsageError, None);
    for invalid in [
        "",
        &"a".repeat(39),
        &"a".repeat(41),
        &"g".repeat(40),
        &"é".repeat(20),
    ] {
        assert!(result.line(Some(invalid)).ends_with(r#","commit":null}"#));
    }
}

#[test]
fn unsuccessful_statuses_cannot_serialize_omissions() {
    for status in [
        Status::CycleFailed,
        Status::ExportFailed,
        Status::Locked,
        Status::UsageError,
    ] {
        let result = ArchiveResult {
            left_out: vec![LeftOutArticle {
                url: "https://example.com/unfinished".into(),
                title: None,
                reason: "not_finished".into(),
            }],
            ..ArchiveResult::new(status, None)
        };
        let line = result.line(None);
        let json: Value =
            serde_json::from_str(line.strip_prefix("HARVESTER_RESULT ").unwrap()).unwrap();
        assert_eq!(json["left_out"], serde_json::json!([]), "{status:?}");
        assert_eq!(json["left_out_articles"], 0, "{status:?}");
    }
}
