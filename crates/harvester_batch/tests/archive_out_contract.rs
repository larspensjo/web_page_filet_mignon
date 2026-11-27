mod archive_out_support;
#[path = "../examples/replay_support/mod.rs"]
#[allow(dead_code)]
mod replay_support;

use archive_out_support::{Capture, ContractFixture, Fixture, CHILD_MARKER, SINCE};
use harvester_batch::{run_main, Args, HostServices};
use harvester_io::{acquire_lock, DESKTOP_LOCK_IDENTITY};
use std::{fs, time::Duration};

#[test]
fn archive_contract_child() {
    if std::env::var_os(CHILD_MARKER).is_none() {
        return;
    }
    let argv: Vec<String> = serde_json::from_str(&std::env::var(CHILD_MARKER).unwrap()).unwrap();
    let args = Args::try_parse_cli(argv).unwrap();
    let services = HostServices::contract_fixture(&args);
    harvester_batch::initialize_logging();
    std::process::exit(run_main(args, services));
}

#[test]
fn fresh_target_is_published_and_the_result_appears_once_at_the_end() {
    let fixture = Fixture::with_article();
    let result = fixture.run("normal").result(0, "exported", None);
    assert_eq!(result["articles"], 1);
    assert_eq!(result["since_utc"], SINCE);
    assert_eq!(result["path"].as_str(), fixture.target.to_str());
    assert!(result["previous_kept"].is_null());
    assert!(result["cost_usd"].as_f64().unwrap() > 0.0);
    assert_eq!(result["archive_schema"], 2);
    assert_eq!(result.get("commit"), Some(&serde_json::Value::Null));
    let until = result["until_utc"].as_str().unwrap();
    assert!(until.ends_with('Z') && !until.contains('.'));
    assert!(fixture
        .archive()
        .contains(&format!("window_until_utc: {until}")));
    assert!(fixture.archive().contains("A canned summary"));
    assert_eq!(fixture.checkpoint_since().as_deref(), Some(until));
    assert!(fixture.record().contains("pending_until_utc: None"));
    assert!(!fixture.sidecar().exists());
}

#[test]
fn real_batch_reports_the_startup_commit_on_argument_errors() {
    const ENV: &str = "HARVESTER_MORNING_COMMIT";
    const COMMIT: &str = "0123456789abcdef0123456789abcdef01234567";
    let fixture = Fixture::empty();
    for (flag, reason) in [
        (Some("--unknown-option"), "invalid_arguments"),
        (None, "invalid_archive_out"),
    ] {
        for (commit, expected) in [
            (None, serde_json::Value::Null),
            (Some("invalid"), serde_json::Value::Null),
            (Some(COMMIT), serde_json::Value::from(COMMIT)),
        ] {
            let mut command = fixture.binary();
            command.args(["--archive-out", "relative.md"]).args(flag);
            if let Some(commit) = commit {
                command.env(ENV, commit);
            } else {
                command.env_remove(ENV);
            }
            let result = command.capture().result(2, "usage_error", Some(reason));
            assert_eq!(result.get("commit"), Some(&expected));
        }
    }
}

#[test]
fn a_processed_target_is_replaced() {
    let fixture = Fixture::with_article();
    let old_until = "2026-09-19T00:00:00Z";
    let old = fixture.seed_target(SINCE, old_until);
    fixture.confirm(old_until);
    let sidecar = fs::read(fixture.sidecar()).unwrap();
    let result = fixture.run("normal").result(0, "exported", None);
    assert_eq!(result["since_utc"], old_until);
    assert!(result["previous_kept"].is_null());
    assert_eq!(result["previous_superseded"], false);
    assert_ne!(fixture.archive(), old);
    assert!(fixture.kept().is_empty());
    assert_eq!(fs::read(fixture.sidecar()).unwrap(), sidecar);
}

#[test]
fn unconfirmed_targets_are_kept_and_supersession_uses_the_window_start() {
    for (old_since, superseded) in [(SINCE, true), ("2026-09-17T00:00:00Z", false)] {
        let fixture = Fixture::with_article();
        let old = fixture.seed_target(old_since, "2026-09-19T00:00:00Z");
        let result = fixture.run("normal").result(0, "exported", None);
        let kept = fixture
            .target
            .with_file_name("archive-unprocessed-20260919T000000Z.md");
        assert_eq!(result["previous_kept"].as_str(), kept.to_str());
        assert_eq!(result["previous_superseded"], superseded);
        assert_eq!(fs::read_to_string(kept).unwrap(), old);
        assert_ne!(fixture.archive(), old);
    }
}

#[test]
fn nothing_new_leaves_files_and_checkpoint_unchanged_but_records_confirmation() {
    let fixture = Fixture::with_article();
    let confirmed = "2026-09-21T00:00:00Z";
    fixture.seed_target(SINCE, confirmed);
    fixture.seed_record(confirmed);
    fixture.confirm(confirmed);
    let old = fs::read(&fixture.target).unwrap();
    let sidecar = fs::read(fixture.sidecar()).unwrap();
    let result = fixture.run("normal").result(10, "nothing_new", None);
    assert_eq!(result["articles"], 0);
    assert_eq!(result["since_utc"], confirmed);
    assert!(result["until_utc"].is_null());
    assert!(result["path"].is_null());
    assert_eq!(fs::read(&fixture.target).unwrap(), old);
    assert_eq!(fs::read(fixture.sidecar()).unwrap(), sidecar);
    assert_eq!(fixture.checkpoint_since().as_deref(), Some(SINCE));
    assert!(fixture
        .record()
        .contains(&format!("confirmed_until_utc: Some(\"{confirmed}\")")));
}

#[test]
fn a_scoring_failure_is_named_in_the_result_without_blocking_the_run() {
    let fixture = Fixture::with_article();
    let result = fixture
        .run("scoring-failure")
        .result(10, "nothing_new", None);
    assert_eq!(result["left_out_articles"], 1);
    assert_eq!(
        result["left_out"],
        serde_json::json!([{
            "url": "https://carry-over.synthetic/article-0",
            "title": "Synthetic carry-over article 0",
            "reason": "failed:scoring"
        }])
    );
    assert!(result["until_utc"].is_null());
    assert!(!fixture.target.exists());
    let retry = fixture.run("normal").result(10, "nothing_new", None);
    assert_eq!(retry["left_out"], serde_json::json!([]));
    assert!(!fixture.target.exists());
}

#[test]
fn morning_logs_each_left_out_url_and_reason() {
    for (mode, code, status, result_reason, url, omission_reason) in [
        (
            "scoring-failure",
            10,
            "nothing_new",
            None,
            "https://carry-over.synthetic/article-0",
            "failed:scoring",
        ),
        (
            "export-failure",
            21,
            "export_failed",
            Some("export_write_failed"),
            "https://carry-over.synthetic/unfinished",
            "not_finished",
        ),
    ] {
        let fixture = Fixture::with_article();
        fixture.run(mode).result(code, status, result_reason);
        let log = fixture.log();
        let omissions: Vec<_> = log
            .lines()
            .filter_map(|line| line.split_once("[morning-archive] left_out "))
            .map(|(_, entry)| entry)
            .collect();
        let expected = format!(
            "target={} url={url} reason={omission_reason}",
            fixture.target.display()
        );
        assert_eq!(omissions, [expected.as_str()], "{mode}");
    }
}

#[test]
fn archive_result_v2_keys_are_in_contract_order() {
    let fixture = Fixture::with_article();
    let run = fixture.run("normal");
    let result = run.result(0, "exported", None);
    assert_eq!(result["v"], 2);
    assert_eq!(result["left_out_articles"], 0);
    assert_eq!(result["left_out"], serde_json::json!([]));
    run.assert_result_key_order(&[
        "v",
        "status",
        "articles",
        "path",
        "previous_kept",
        "previous_superseded",
        "cost_usd",
        "cost_basis",
        "archive_schema",
        "since_utc",
        "until_utc",
        "failed_sources",
        "left_out_articles",
        "left_out",
        "lock_holder",
        "reason",
        "commit",
    ]);
}

#[test]
fn missing_or_unreadable_openai_stops_before_the_poll() {
    for (mode, reason) in [
        ("missing-key", "missing_openai_key"),
        ("empty-key", "missing_openai_key"),
        ("unreadable-key", "credential_unreadable"),
    ] {
        let fixture = Fixture::with_article();
        let result = fixture.run(mode).result(2, "usage_error", Some(reason));
        assert!(result["cost_usd"].is_null());
        assert!(!fixture.target.exists());
        fixture.assert_no_poll();
    }
}

#[test]
fn lock_refusal_reports_the_holder_and_never_polls() {
    let fixture = Fixture::empty();
    let _guard = acquire_lock(&fixture.output, DESKTOP_LOCK_IDENTITY, false).unwrap();
    let result = fixture.run("normal").result(22, "locked", None);
    let holder = result["lock_holder"].as_str().unwrap();
    assert!(holder.contains("the Harvester window"));
    assert!(holder.contains(&std::process::id().to_string()));
    assert!(holder.contains("started:"));
    fixture.assert_no_poll();
}

#[test]
fn graceful_interrupt_before_export_keeps_the_checkpoint() {
    let fixture = Fixture::with_article();
    fixture
        .run("interrupt-before")
        .result(20, "cycle_failed", Some("interrupted"));
    assert!(!fixture.target.exists());
    assert_eq!(fixture.checkpoint_since().as_deref(), Some(SINCE));
    assert!(fixture.state_exists());
}

#[test]
fn interrupt_after_export_request_reports_the_publication_result() {
    for (mode, code, status, reason) in [
        ("interrupt-after", 0, "exported", None),
        (
            "interrupt-export-failure",
            21,
            "export_failed",
            Some("export_write_failed"),
        ),
    ] {
        let fixture = Fixture::with_article();
        fixture.run(mode).result(code, status, reason);
        assert_eq!(fixture.target.is_file(), code == 0);
    }
}

#[test]
fn a_second_interrupt_is_a_crash_and_the_next_run_reconciles_the_intent() {
    let fixture = Fixture::with_article();
    fixture.run("interrupt-twice").crash(130);
    assert!(!fixture.target.exists());
    assert!(fixture.record().contains("pending_until_utc: Some("));
    #[cfg(not(windows))]
    fixture.clear_crash_lock();
    fixture.run("normal").result(0, "exported", None);
    assert!(fixture.record().contains("pending_until_utc: None"));
}

#[test]
fn crash_after_each_publication_step_leaves_complete_names_and_next_run_reconciles() {
    for (step, kept, published) in [
        ("CleanedStaleTemps", false, false),
        ("SyncedNewArchive", false, false),
        ("CheckedTarget", false, false),
        ("SyncedKeptArchive", false, false),
        ("KeptPrevious", true, false),
        ("ReplacedTarget", true, true),
        ("ReportedCompletion", true, true),
    ] {
        let fixture = Fixture::with_article();
        let old = fixture.seed_target(SINCE, "2026-09-19T00:00:00Z");
        fixture.run(&format!("crash-{step}")).crash(91);
        let after_crash = fixture.archive();
        let copies = fixture.kept();
        assert_eq!(after_crash == old, !published, "{step}");
        assert_eq!(copies.len(), usize::from(kept), "{step}");
        if kept {
            assert_eq!(fs::read_to_string(&copies[0]).unwrap(), old, "{step}");
        }
        assert_eq!(fixture.checkpoint_since().as_deref(), Some(SINCE), "{step}");
        assert!(
            fixture.record().contains("pending_until_utc: Some("),
            "{step}"
        );
        #[cfg(not(windows))]
        fixture.clear_crash_lock();
        if published {
            let until = after_crash
                .lines()
                .find_map(|line| line.strip_prefix("window_until_utc: "))
                .unwrap();
            fixture.confirm(until);
            let result = fixture.run("normal").result(10, "nothing_new", None);
            assert_eq!(result["since_utc"], until, "{step}");
            assert_eq!(fixture.checkpoint_since().as_deref(), Some(until), "{step}");
            assert_eq!(fixture.archive(), after_crash, "{step}");
            assert_eq!(fixture.kept(), copies, "{step}");
        } else {
            let result = fixture.run("normal").result(0, "exported", None);
            assert_eq!(result["since_utc"], SINCE, "{step}");
            assert_eq!(fixture.kept().len(), 1, "{step}");
            assert_eq!(
                fs::read_to_string(&fixture.kept()[0]).unwrap(),
                old,
                "{step}"
            );
        }
        assert!(
            fixture.record().contains("pending_until_utc: None"),
            "{step}"
        );
    }
}

#[test]
fn lost_export_ack_checks_the_target_index_and_finishes_under_the_watchdog() {
    for (mode, code, status) in [
        ("drop-success", 0, "exported"),
        ("close-success", 0, "exported"),
        ("drop-failure", 21, "export_failed"),
    ] {
        let fixture = Fixture::with_article();
        if code == 0 {
            fixture.seed_target(SINCE, "2026-09-19T00:00:00Z");
        }
        let run = fixture.run(mode);
        assert!(run.elapsed < Duration::from_secs(25));
        let result = run.result(code, status, Some("export_ack_lost"));
        assert_eq!(fixture.target.is_file(), code == 0);
        if code == 0 {
            assert_eq!(result["articles"], 1);
            assert_eq!(result["previous_kept"].as_str(), fixture.kept()[0].to_str());
            assert_eq!(result["previous_superseded"], true);
            assert_eq!(
                fixture.checkpoint_since().as_deref(),
                result["until_utc"].as_str()
            );
        } else {
            assert_eq!(fixture.checkpoint_since().as_deref(), Some(SINCE));
            assert_eq!(result["articles"], 0);
        }
    }
}

#[test]
fn an_active_publication_step_can_outlast_the_watchdog() {
    let fixture = Fixture::with_article();
    let old = fixture.seed_target(SINCE, "2026-09-19T00:00:00Z");
    let result = fixture.run("slow-publication").result(0, "exported", None);
    let kept = fixture.kept();
    assert_eq!(kept.len(), 1);
    assert_eq!(result["previous_kept"].as_str(), kept[0].to_str());
    assert_eq!(fs::read_to_string(&kept[0]).unwrap(), old);
    assert_ne!(fixture.archive(), old);
}

#[test]
fn recovery_reports_the_kept_archive_when_the_worker_stops_before_its_receipt() {
    let fixture = Fixture::with_article();
    let old = fixture.seed_target(SINCE, "2026-09-19T00:00:00Z");
    let result =
        fixture
            .run("lose-publication-receipt")
            .result(0, "exported", Some("export_ack_lost"));
    let kept = fixture.kept();
    assert_eq!(kept.len(), 1);
    assert_eq!(result["previous_kept"].as_str(), kept[0].to_str());
    assert_eq!(result["previous_superseded"], true);
    assert_eq!(fs::read_to_string(&kept[0]).unwrap(), old);
    assert_eq!(
        fixture.checkpoint_since().as_deref(),
        result["until_utc"].as_str()
    );
}

#[test]
fn publication_intent_precedes_rename_and_the_checkpoint_write_follows_it() {
    let fixture = Fixture::with_article();
    fixture.run("observe-order").result(0, "exported", None);
    let log = fs::read_to_string(fixture.output.join("publication-order")).unwrap();
    assert_eq!(
        log.lines().collect::<Vec<_>>(),
        ["intent", "rename", "checkpoint"]
    );
}

#[test]
fn invalid_cli_paths_and_conflicts_use_the_binary_result_contract() {
    let fixture = Fixture::empty();
    for target in [
        "relative.md".into(),
        fixture.target.with_file_name("missing").join("archive.md"),
        fixture.output.join("archive.md"),
        fixture.output.join("inner/../archive.md"),
    ] {
        fs::create_dir_all(fixture.output.join("inner")).unwrap();
        fixture
            .binary()
            .arg("--archive-out")
            .arg(target)
            .capture()
            .result(2, "usage_error", Some("invalid_archive_out"));
    }
    for flag in [
        "--unknown-option",
        "--show-checkpoint",
        "--clear-checkpoint",
        "--set-checkpoint-now",
    ] {
        fixture
            .binary()
            .arg("--archive-out")
            .arg(&fixture.target)
            .arg(flag)
            .capture()
            .result(2, "usage_error", Some("invalid_arguments"));
    }
    for (flag, value) in [
        ("--set-checkpoint", SINCE),
        ("--import-saved-web-dir", "saved"),
    ] {
        fixture
            .binary()
            .arg("--archive-out")
            .arg(&fixture.target)
            .args([flag, value])
            .capture()
            .result(2, "usage_error", Some("invalid_arguments"));
    }
    fixture.binary().arg("--archive-out").capture().result(
        2,
        "usage_error",
        Some("invalid_arguments"),
    );
    fixture
        .binary()
        .arg(format!("--archive-out={}", fixture.target.display()))
        .arg("--unknown-option")
        .capture()
        .result(2, "usage_error", Some("invalid_arguments"));
}

#[test]
fn help_and_version_keep_clap_output_and_exit_without_a_result_line() {
    let fixture = Fixture::empty();
    for flag in ["--help", "--version"] {
        fixture
            .binary()
            .arg("--archive-out")
            .arg(&fixture.target)
            .arg(flag)
            .capture()
            .crash(0);
    }
}

#[test]
fn startup_reports_window_and_record_errors_before_polling() {
    for (mode, code, status, reason) in [
        ("normal", 2, "usage_error", "no_window_start"),
        (
            "startup-record-failure",
            20,
            "cycle_failed",
            "record_not_saved",
        ),
        ("normal", 20, "cycle_failed", "record_unreadable"),
        ("runtime-failure", 20, "cycle_failed", "startup_stalled"),
        ("startup-watchdog", 20, "cycle_failed", "startup_stalled"),
        (
            "startup-disconnected",
            20,
            "cycle_failed",
            "startup_stalled",
        ),
    ] {
        let fixture = Fixture::empty();
        if reason == "no_window_start" {
            fs::remove_file(&fixture.checkpoint).unwrap();
        } else if reason == "record_unreadable" {
            fs::remove_file(&fixture.checkpoint).unwrap();
            fs::create_dir(&fixture.checkpoint).unwrap();
        }
        let result = fixture.run(mode).result(code, status, Some(reason));
        assert!(result["cost_usd"].is_null());
        assert!(!fixture.target.exists());
        fixture.assert_no_poll();
    }
}

#[test]
fn a_missing_brave_key_is_counted_and_only_total_poll_failure_blocks() {
    for has_success in [true, false] {
        let fixture = Fixture::empty();
        fixture.brave_sources(has_success);
        let run = fixture.run("normal");
        let result = if has_success {
            run.result(10, "nothing_new", None)
        } else {
            run.result(20, "cycle_failed", Some("total_failure"))
        };
        assert_eq!(result["failed_sources"], 1);
        run.assert_printed_failed_source("brave");
        assert!(fixture
            .log()
            .contains("[morning-archive] failed_source id=brave reason="));
        assert!(!fixture.target.exists());
    }
}

#[test]
fn refusing_the_results_store_or_startup_readiness_still_allows_intake() {
    for (mode, reason) in [
        ("normal", "result_store_unavailable"),
        ("failed-contexts", "saved_results_unavailable"),
    ] {
        let fixture = Fixture::empty();
        if mode == "normal" {
            fs::write(fixture.output.join(".summary_cache.ron"), "damaged").unwrap();
        } else {
            fixture.remove_required_context();
        }
        fixture.run(mode).result(20, "cycle_failed", Some(reason));
        assert!(fixture.log().contains("[source-poll] polling requested"));
        assert!(fixture.state_exists());
        assert!(!fixture.target.exists());
    }
}

#[test]
fn cycle_channel_loss_still_flushes_and_persists_before_reporting() {
    let fixture = Fixture::empty();
    fixture
        .run("cycle-disconnected")
        .result(20, "cycle_failed", Some("cycle_stalled"));
    assert!(fixture.state_exists());
    assert!(fixture.output.join("flush-called").exists());
    assert!(!fixture.target.exists());
}

#[test]
fn intent_save_failure_prevents_publication_and_completion_save_failure_keeps_it() {
    for (mode, code, status, reason) in [
        (
            "intent-record-failure",
            21,
            "export_failed",
            "record_not_saved",
        ),
        (
            "completion-record-failure",
            0,
            "exported",
            "checkpoint_not_saved",
        ),
    ] {
        let fixture = Fixture::with_article();
        fixture.run(mode).result(code, status, Some(reason));
        assert_eq!(fixture.target.is_file(), code == 0);
        assert_eq!(fixture.checkpoint_since().as_deref(), Some(SINCE));
    }
}

#[test]
fn worker_failure_is_export_failed_without_advancing_the_checkpoint() {
    let fixture = Fixture::with_article();
    let result =
        fixture
            .run("export-failure")
            .result(21, "export_failed", Some("export_write_failed"));
    assert_eq!(result["left_out_articles"], 0);
    assert_eq!(result["left_out"], serde_json::json!([]));
    assert!(fixture
        .record()
        .contains("https://carry-over.synthetic/unfinished"));
    assert!(!fixture.target.is_file());
    assert_eq!(fixture.checkpoint_since().as_deref(), Some(SINCE));
}

#[test]
fn quota_halted_reports_an_empty_omission_list() {
    let fixture = Fixture::with_article();
    let result = fixture
        .run("quota-halt")
        .result(20, "cycle_failed", Some("quota_halted"));
    assert_eq!(result["left_out_articles"], 0);
    assert_eq!(result["left_out"], serde_json::json!([]));
    assert!(result["until_utc"].is_null());
    assert!(!fixture.target.exists());
    assert_eq!(fixture.checkpoint_since().as_deref(), Some(SINCE));
}

#[test]
fn flush_failure_only_changes_the_reason_for_an_exported_archive() {
    let exported = Fixture::with_article();
    exported
        .run("flush-failure")
        .result(0, "exported", Some("results_not_saved"));
    assert!(exported.target.is_file());
    assert!(exported.state_exists());
    let empty = Fixture::empty();
    empty.run("flush-failure").result(10, "nothing_new", None);
    assert!(empty.state_exists());
    assert!(empty.log().contains("fixture results flush failure"));
}

#[test]
fn flush_failure_preserves_a_specific_export_reason() {
    for (mode, reason) in [
        (
            "completion-record-failure+flush-failure",
            "checkpoint_not_saved",
        ),
        ("drop-success+flush-failure", "export_ack_lost"),
    ] {
        let fixture = Fixture::with_article();
        fixture.run(mode).result(0, "exported", Some(reason));
        assert!(fixture.target.is_file());
        assert!(fixture.log().contains("fixture results flush failure"));
    }
}

#[test]
fn unexpected_panics_remain_crashes_without_a_result_line() {
    Fixture::empty().run("panic").crash(101);
}

#[test]
fn contract_child_refuses_an_unmarked_folder() {
    let fixture = Fixture::empty();
    fs::remove_file(fixture.output.join(".harvester-contract-fixture")).unwrap();
    fixture.run("normal").crash(101);
    assert!(!fixture.state_exists());
}
