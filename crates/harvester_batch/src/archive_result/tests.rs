use super::*;
use serde_json::Value;

#[test]
fn result_line_is_ordered_ascii_json_with_exact_cost_and_utc_precision() {
    let line = ArchiveResult {
        status: Status::Exported,
        articles: 8,
        path: Some(PathBuf::from("c:/utgåva/📰.md")),
        previous_kept: Some(PathBuf::from("c:/utgåva/old.md")),
        previous_superseded: true,
        cost_microdollars: Some(412_305),
        since_utc: Some("2025-09-17T06:10:31.123456Z".parse().unwrap()),
        until_utc: Some("2025-09-18T06:12:04Z".parse().unwrap()),
        failed_sources: 1,
        excused_articles: 2,
        ..ArchiveResult::new(Status::Exported, None)
    }
    .line(Some("0123456789abcdef0123456789ABCDEF01234567"));
    assert!(line.is_ascii());
    assert!(!line.contains('\n'));
    assert_eq!(
        line,
        concat!(
            r#"HARVESTER_RESULT {"v":1,"status":"exported","articles":8,"path":"c:/utg\u00e5va/\ud83d\udcf0.md","#,
            r#""previous_kept":"c:/utg\u00e5va/old.md","previous_superseded":true,"cost_usd":0.412305,"#,
            r#""cost_basis":"openai_price_table","#,
            r#""archive_schema":2,"since_utc":"2025-09-17T06:10:31.123456Z","until_utc":"2025-09-18T06:12:04Z","#,
            r#""failed_sources":1,"backlog_articles":0,"excused_articles":2,"lock_holder":null,"reason":null,"#,
            r#""commit":"0123456789abcdef0123456789ABCDEF01234567"}"#,
        )
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
            r#"HARVESTER_RESULT {"v":1,"status":"usage_error","articles":0,"path":null,"#,
            r#""previous_kept":null,"previous_superseded":false,"cost_usd":null,"#,
            r#""cost_basis":"openai_price_table","#,
            r#""archive_schema":2,"since_utc":null,"until_utc":null,"failed_sources":0,"backlog_articles":0,"#,
            r#""excused_articles":0,"lock_holder":null,"reason":"invalid_arguments","commit":null}"#,
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
