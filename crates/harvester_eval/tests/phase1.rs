use std::fs;
use std::path::{Path, PathBuf};
use std::thread;
use std::time::Duration;

use clap::Parser;
use harvester_engine::llm::{content_hash, PromptId, ReplayRecord, TemplateVars, TokenUsage};
use harvester_engine::{derive_clean_text, parse_frontmatter};
use pretty_assertions::assert_eq;
use serde_json::json;
use tempfile::TempDir;

use harvester_eval::article_index::build_or_load_index;
use harvester_eval::cli::{run, Cli};
use harvester_eval::freeze::{freeze, FreezeOptions};
use harvester_eval::identity::{classify_path, verify_source_identity, PathKind};
use harvester_eval::manifest::{load_manifest, manifest_path};
use harvester_eval::prompt_identity::{classify_record, load_prompt_identity, PromptMatch};
use harvester_eval::recovery::{recover_document_text, RecoveryError};
use harvester_eval::report::baseline::{compute, rate_label, write_baseline_report};

fn setup() -> (TempDir, PathBuf, PathBuf, PathBuf) {
    let temp = TempDir::new().unwrap();
    let output = temp.path().join("output");
    let experiment = temp.path().join("experiment");
    let context = temp.path().join("triage.toml");
    fs::create_dir_all(output.join("llm_results")).unwrap();
    fs::write(&context, "[meta]\nprompt_id = 'ArticleTriage'\nschema_version = 1\nversion = 3\nupdated = '2026-01-01'\n[variables]\ntriage_instructions = 'fixture rubric'\n").unwrap();
    (temp, output, experiment, context)
}

fn markdown(title: &str, body: &str) -> String {
    format!("---\nurl: https://example.test/{title}\ntitle: {title}\nfetched_utc: 2026-01-01T00:00:00Z\n---\n\n{body}")
}

fn clean_hash(markdown: &str) -> String {
    let frontmatter = parse_frontmatter(markdown).unwrap();
    derive_clean_text(
        markdown,
        frontmatter.url.as_deref().unwrap().trim(),
        frontmatter.title.as_deref(),
        &harvester_engine::eval_support::default_content_prep_config(),
    )
    .content_hash()
    .to_string()
}

fn record(
    context: &Path,
    request_id: &str,
    text: &str,
    source_hash: String,
    batch: bool,
    timestamp: &str,
    priority: u8,
) -> ReplayRecord {
    let identity = load_prompt_identity(context).unwrap();
    let mut vars = TemplateVars::new();
    vars.set_document("content", text);
    let wrapped = vars.to_map().remove("content").unwrap();
    ReplayRecord {
        request_id: request_id.to_string(),
        input_content_hash: source_hash,
        prompt_id: PromptId::ArticleTriage,
        prompt_version: 4,
        model_id: "gpt-5.4-nano".into(),
        timestamp_utc: timestamp.into(),
        rendered_system_message: identity.system_message,
        rendered_user_message: format!("Document:\n{wrapped}\n\nAnalyze"),
        raw_response: "{}".into(),
        usage: TokenUsage::new(100, 20),
        validated_output: Some(
            json!({"category":"Technology", "priority":priority, "tags":["ai"], "rationale":"fixture"}),
        ),
        validation_error: None,
        cost_microdollars: 10,
        wall_ms: if batch { 0 } else { 15 },
        cache_status: if batch {
            "batch_collected".into()
        } else {
            "miss".into()
        },
    }
}

fn write_record(output: &Path, name: &str, record: &ReplayRecord) {
    fs::write(
        output.join("llm_results").join(name),
        serde_json::to_vec(record).unwrap(),
    )
    .unwrap();
}

fn options(output: PathBuf, experiment: PathBuf, context: PathBuf) -> FreezeOptions {
    FreezeOptions {
        output_dir: output,
        linked_dir: None,
        experiment_dir: experiment,
        limit: 200,
        prompt_version: 4,
        context_file: context,
        dev_share: 0.5,
        split_seed: 7,
        min_priority_5: 16,
        dry_run: false,
    }
}

#[test]
fn freeze_refuses_missing_empty_and_nonmatching_replay_inputs() {
    let (_temp, output, experiment, context) = setup();
    fs::create_dir_all(&experiment).unwrap();
    let existing_manifest = experiment.join("manifest.json");
    fs::write(&existing_manifest, b"existing 800-article freeze").unwrap();
    fs::remove_dir_all(output.join("llm_results")).unwrap();
    let error = freeze(&options(
        output.clone(),
        experiment.clone(),
        context.clone(),
    ))
    .unwrap_err()
    .to_string();
    assert!(error.contains("replay directory is missing"), "{error}");
    assert_eq!(
        fs::read(&existing_manifest).unwrap(),
        b"existing 800-article freeze"
    );

    fs::create_dir_all(output.join("llm_results")).unwrap();
    let error = freeze(&options(
        output.clone(),
        experiment.clone(),
        context.clone(),
    ))
    .unwrap_err()
    .to_string();
    assert!(
        error.contains("replay directory contains no JSON records"),
        "{error}"
    );
    assert_eq!(
        fs::read(&existing_manifest).unwrap(),
        b"existing 800-article freeze"
    );

    let mut wrong_prompt = record(
        &context,
        "wrong-prompt",
        "not a triage record",
        content_hash("not a triage record"),
        false,
        "2026-01-01T00:00:00Z",
        3,
    );
    wrong_prompt.prompt_id = PromptId::ArticleSummary;
    write_record(&output, "wrong-prompt.json", &wrong_prompt);
    let error = freeze(&options(output, experiment.clone(), context))
        .unwrap_err()
        .to_string();
    assert!(error.contains("no replay records"), "{error}");
    assert!(error.contains("match ArticleTriage"), "{error}");
    assert_eq!(
        fs::read(existing_manifest).unwrap(),
        b"existing 800-article freeze"
    );
}

#[test]
fn prompt_identity_matches_context_and_detects_changes() {
    let (_temp, _output, _experiment, context) = setup();
    let mut matching = record(
        &context,
        "one",
        "text",
        content_hash("text"),
        false,
        "2026-01-01T00:00:00Z",
        3,
    );
    let identity = load_prompt_identity(&context).unwrap();
    assert_eq!(classify_record(&matching, 4, &identity), PromptMatch::Match);
    let context_block = identity
        .system_message
        .split_once("BACKGROUND CONTEXT:\n")
        .unwrap()
        .1
        .split_once("\n\nUse the background context")
        .unwrap()
        .0;
    assert_eq!(context_block, "triage_instructions: fixture rubric");
    fs::write(&context, "[meta]\nprompt_id = 'ArticleTriage'\nschema_version = 1\nversion = 3\nupdated = '2026-01-01'\n[variables]\ntriage_instructions = '''fixture rubric\nchanged rubric line'''\n").unwrap();
    let changed = load_prompt_identity(&context).unwrap();
    assert_eq!(
        classify_record(&matching, 4, &changed),
        PromptMatch::PromptMismatch
    );
    matching.prompt_version = 3;
    assert_eq!(
        classify_record(&matching, 4, &identity),
        PromptMatch::WrongPromptOrVersion
    );
}

#[test]
fn recovery_handles_nonce_wrapper_and_malformed_tags() {
    let mut vars = TemplateVars::new();
    vars.set_document("content", "bytes\nunchanged");
    let wrapped = vars.to_map().remove("content").unwrap();
    let recovered = recover_document_text(&wrapped).unwrap();
    assert_eq!(recovered.text, "bytes\nunchanged");
    assert!(recovered.nonce_verified);
    assert_eq!(
        recover_document_text("<document-aaaaaaaaaaaa>\ntext").unwrap_err(),
        RecoveryError::MissingCloseTag
    );
    assert_eq!(
        recover_document_text("<document-aaaaaaaaaaaa>\ntext\n</document-bbbbbbbbbbbb>")
            .unwrap_err(),
        RecoveryError::TagNonceMismatch
    );
    let mut vars = TemplateVars::new();
    vars.set_document("content", "article contains </document-abc> as data");
    let wrapped = vars.to_map().remove("content").unwrap();
    assert_eq!(
        recover_document_text(&wrapped).unwrap().text,
        "article contains </document-abc> as data"
    );
}

#[test]
fn identity_is_path_aware_for_every_sync_batch_truncation_combination() {
    let (_temp, _output, _experiment, context) = setup();
    let full = "long full article";
    let truncated = format!("prefix{}", harvester_engine::TRUNCATION_MARKER);
    for (batch, text, source) in [
        (false, full, content_hash(full)),
        (false, truncated.as_str(), content_hash(&truncated)),
        (true, full, content_hash(full)),
        (true, truncated.as_str(), content_hash(full)),
    ] {
        let record = record(
            &context,
            "fixture",
            text,
            source,
            batch,
            "2026-01-01T00:00:00Z",
            3,
        );
        let recovered = recover_document_text(&record.rendered_user_message).unwrap();
        let verification = verify_source_identity(classify_path(&record), &record, &recovered);
        if batch && text == truncated {
            assert!(!verification.matches_evidence);
            assert!(verification.anomaly.is_none());
        } else {
            assert!(verification.matches_evidence);
            assert!(verification.anomaly.is_none());
        }
    }
    let doctored = record(
        &context,
        "bad",
        full,
        content_hash("other"),
        false,
        "2026-01-01T00:00:00Z",
        3,
    );
    let recovered = recover_document_text(&doctored.rendered_user_message).unwrap();
    assert!(
        verify_source_identity(PathKind::Sync, &doctored, &recovered)
            .anomaly
            .is_some()
    );
}

#[test]
fn freeze_collapses_cross_path_records_and_maps_matrix() {
    let (_temp, output, experiment, context) = setup();
    let full = "prefix evidence followed by remaining article material";
    let truncated = format!("prefix evidence{}", harvester_engine::TRUNCATION_MARKER);
    let markdown = markdown("one", full);
    fs::write(output.join("one.md"), &markdown).unwrap();
    let clean = clean_hash(&markdown);
    write_record(
        &output,
        "01-sync.json",
        &record(
            &context,
            "sync",
            &truncated,
            content_hash(&truncated),
            false,
            "2026-01-01T00:00:00Z",
            3,
        ),
    );
    write_record(
        &output,
        "02-batch.json",
        &record(
            &context,
            "batch",
            &truncated,
            clean,
            true,
            "2026-01-02T00:00:00Z",
            4,
        ),
    );
    fs::write(output.join("llm_results/03-unreadable.json"), "not json").unwrap();
    let result = freeze(&options(output.clone(), experiment.clone(), context)).unwrap();
    assert_eq!(result.manifest.articles.len(), 1);
    let article = &result.manifest.articles[0];
    assert_eq!(article.source_content_hashes.len(), 2);
    assert_eq!(article.mapping_status, "mapped_by_hash");
    assert_eq!(result.manifest.counts.cross_path_duplicates, 1);
    assert_eq!(result.manifest.counts.exact_duplicates, 1);
    assert_eq!(result.manifest.counts.load_failed, 1);
    let rendered = result.preconditions.render();
    assert!(rendered.contains("rejections (of 3 scanned): load_failed=1 (33.3%)"));
    assert!(rendered.contains("exact duplicates: 1; near-duplicate groups:"));
    assert!(rendered.contains("sync/batch:"));
    assert!(rendered.contains("date range:"));
    assert!(rendered.contains("splits: dev="));
    assert!(experiment.join(&article.text_path).is_file());
    assert!(load_manifest(&manifest_path(&experiment)).is_ok());
}

#[test]
fn selection_is_deterministic_uses_parsed_timestamps_and_limits_newest() {
    let (_temp, output, experiment, context) = setup();
    let records = [
        ("r0", "2026-01-01T00:00:00Z"),
        ("r1", "2026-01-02T00:00:00Z"),
        ("r2", "2026-01-03T00:00:00Z"),
        ("r3", "2026-01-04T00:00:00Z"),
        ("r4", "2026-01-05T00:00:00Z"),
        ("r5", "2026-01-06T00:00:00Z"),
        ("r6", "2026-01-07T00:00:00Z"),
        ("r7", "2026-01-08T00:00:00Z"),
        ("offset", "2026-01-11T00:30:00+01:00"),
        ("utc", "2026-01-10T23:45:00Z"),
        ("tie-a", "2026-01-12T00:00:00Z"),
        ("tie-z", "2026-01-12T00:00:00.000Z"),
    ];
    for (index, (request_id, timestamp)) in records.iter().enumerate() {
        let text = format!("selection evidence {index}");
        write_record(
            &output,
            &format!("{index:02}.json"),
            &record(
                &context,
                request_id,
                &text,
                content_hash(&text),
                false,
                timestamp,
                3,
            ),
        );
    }

    let first = freeze(&options(
        output.clone(),
        experiment.join("first"),
        context.clone(),
    ))
    .unwrap()
    .manifest;
    thread::sleep(Duration::from_millis(2));
    let second = freeze(&options(
        output.clone(),
        experiment.join("second"),
        context.clone(),
    ))
    .unwrap()
    .manifest;
    assert_ne!(first.created_utc, second.created_utc);
    assert_eq!(first.manifest_hash, second.manifest_hash);
    let mut first_without_time = first.clone();
    let mut second_without_time = second.clone();
    first_without_time.created_utc.clear();
    second_without_time.created_utc.clear();
    assert_eq!(first_without_time, second_without_time);
    assert_eq!(
        first
            .articles
            .iter()
            .take(4)
            .map(|article| article.baseline.request_id.as_str())
            .collect::<Vec<_>>(),
        vec!["tie-z", "tie-a", "utc", "offset"]
    );

    let mut limited_options = options(output, experiment.join("limited"), context);
    limited_options.limit = 3;
    let limited = freeze(&limited_options).unwrap().manifest;
    assert_eq!(
        limited
            .articles
            .iter()
            .map(|article| article.baseline.request_id.as_str())
            .collect::<Vec<_>>(),
        vec!["tie-z", "tie-a", "utc"]
    );
}

#[test]
fn near_duplicate_group_members_share_a_split() {
    let (_temp, output, experiment, context) = setup();
    for (index, text) in ["Near DUPLICATE, article!", "near duplicate article"]
        .iter()
        .enumerate()
    {
        write_record(
            &output,
            &format!("{index}.json"),
            &record(
                &context,
                &format!("near-{index}"),
                text,
                content_hash(text),
                false,
                &format!("2026-01-0{}T00:00:00Z", index + 1),
                3,
            ),
        );
    }
    let manifest = freeze(&options(output, experiment, context))
        .unwrap()
        .manifest;
    assert_eq!(manifest.articles.len(), 2);
    assert_eq!(
        manifest.articles[0].duplicate_group,
        manifest.articles[1].duplicate_group
    );
    assert_eq!(manifest.articles[0].split, manifest.articles[1].split);
    assert_eq!(manifest.counts.duplicate_groups, 1);
}

#[test]
fn mapping_uses_4k_bucket_reports_ambiguity_and_keeps_unmapped() {
    let (_temp, output, experiment, context) = setup();
    let long_body = format!("{} tail", "a".repeat(5000));
    let long_markdown = markdown("long", &long_body);
    fs::write(output.join("long.md"), &long_markdown).unwrap();
    let prefix = &long_body[..4200];
    let truncated = format!("{prefix}{}", harvester_engine::TRUNCATION_MARKER);
    write_record(
        &output,
        "prefix.json",
        &record(
            &context,
            "prefix",
            &truncated,
            content_hash(&truncated),
            false,
            "2026-01-03T00:00:00Z",
            3,
        ),
    );

    let duplicate_body = "same clean evidence";
    let duplicate_one = markdown("duplicate-one", duplicate_body);
    let duplicate_two = markdown("duplicate-two", duplicate_body);
    fs::write(output.join("duplicate-one.md"), &duplicate_one).unwrap();
    fs::write(output.join("duplicate-two.md"), &duplicate_two).unwrap();
    write_record(
        &output,
        "ambiguous.json",
        &record(
            &context,
            "ambiguous",
            duplicate_body,
            clean_hash(&duplicate_one),
            true,
            "2026-01-02T00:00:00Z",
            3,
        ),
    );
    write_record(
        &output,
        "unmapped.json",
        &record(
            &context,
            "unmapped",
            "no corpus source",
            content_hash("no corpus source"),
            false,
            "2026-01-01T00:00:00Z",
            3,
        ),
    );

    let manifest = freeze(&options(output, experiment, context))
        .unwrap()
        .manifest;
    let prefix_entry = manifest
        .articles
        .iter()
        .find(|article| article.baseline.request_id == "prefix")
        .unwrap();
    assert_eq!(prefix_entry.mapping_status, "mapped_by_prefix");
    let ambiguous = manifest
        .articles
        .iter()
        .find(|article| article.baseline.request_id == "ambiguous")
        .unwrap();
    assert_eq!(ambiguous.mapping_status, "ambiguous");
    assert_eq!(ambiguous.ambiguous_paths.len(), 2);
    assert!(ambiguous
        .ambiguous_paths
        .iter()
        .any(|path| path.ends_with("duplicate-one.md")));
    assert!(ambiguous
        .ambiguous_paths
        .iter()
        .any(|path| path.ends_with("duplicate-two.md")));
    let unmapped = manifest
        .articles
        .iter()
        .find(|article| article.baseline.request_id == "unmapped")
        .unwrap();
    assert_eq!(unmapped.mapping_status, "unmapped");
    assert!(unmapped.article_file.is_none());
}

#[test]
fn article_index_cache_reuses_unchanged_invalidates_length_and_skips_non_utf8() {
    let temp = TempDir::new().unwrap();
    let output = temp.path().join("output");
    let cache = temp.path().join("cache/article-index.json");
    fs::create_dir_all(&output).unwrap();
    let article = output.join("article.md");
    fs::write(&article, markdown("cache", "first body")).unwrap();
    let first = build_or_load_index(&output, None, &cache).unwrap();
    assert_eq!(first.stats.derived, 1);
    assert_eq!(first.stats.reused, 0);
    let second = build_or_load_index(&output, None, &cache).unwrap();
    assert_eq!(second.stats.derived, 0);
    assert_eq!(second.stats.reused, 1);
    fs::write(&article, markdown("cache", "a longer second body")).unwrap();
    let third = build_or_load_index(&output, None, &cache).unwrap();
    assert_eq!(third.stats.derived, 1);
    assert_eq!(third.stats.reused, 0);
    fs::write(output.join("binary.md"), [0xff, 0xfe, 0xfd]).unwrap();
    let fourth = build_or_load_index(&output, None, &cache).unwrap();
    assert_eq!(fourth.stats.skipped, 1);
    assert_eq!(fourth.entries.len(), 1);
}

#[test]
fn cli_dry_run_writes_only_log_and_optional_preconditions() {
    let (_temp, output, experiment, context) = setup();
    let text = "evidence";
    write_record(
        &output,
        "record.json",
        &record(
            &context,
            "one",
            text,
            content_hash(text),
            false,
            "2026-01-01T00:00:00Z",
            3,
        ),
    );
    let first_experiment = experiment.join("without-report");
    run(Cli::parse_from([
        "harvester_eval",
        "--experiment-dir",
        first_experiment.to_str().unwrap(),
        "freeze",
        "--output-dir",
        output.to_str().unwrap(),
        "--context-file",
        context.to_str().unwrap(),
        "--dry-run",
    ]))
    .unwrap();
    let mut first_files = fs::read_dir(&first_experiment)
        .unwrap()
        .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
        .collect::<Vec<_>>();
    first_files.sort();
    assert_eq!(first_files, vec!["harvester_eval.log"]);

    let second_experiment = experiment.join("with-report");
    let report = second_experiment.join("preconditions.json");
    run(Cli::parse_from([
        "harvester_eval",
        "--experiment-dir",
        second_experiment.to_str().unwrap(),
        "freeze",
        "--output-dir",
        output.to_str().unwrap(),
        "--context-file",
        context.to_str().unwrap(),
        "--dry-run",
        "--precondition-report",
        report.to_str().unwrap(),
    ]))
    .unwrap();
    let mut second_files = fs::read_dir(&second_experiment)
        .unwrap()
        .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
        .collect::<Vec<_>>();
    second_files.sort();
    assert_eq!(
        second_files,
        vec!["harvester_eval.log", "preconditions.json"]
    );
}

#[test]
fn frozen_input_integrity_and_baseline_metrics_are_enforced() {
    let (_temp, output, experiment, context) = setup();
    for (id, priority, batch, category, tags, cost, wall_ms) in [
        ("a", 5, false, "Technology", vec!["ai", "chips"], 10, 10),
        ("b", 1, true, "Business", vec!["markets"], 20, 0),
        ("c", 4, false, "Technology", vec![], 30, 20),
        ("d", 2, false, "Science", vec!["ai"], 40, 30),
        (
            "e",
            3,
            false,
            "Business",
            vec!["policy", "ai", "markets"],
            50,
            40,
        ),
        ("f", 1, false, "Technology", vec!["ai"], 60, 0),
    ] {
        let text = format!("evidence {id}");
        let mut replay = record(
            &context,
            id,
            &text,
            content_hash(&text),
            batch,
            &format!("2026-01-0{}T00:00:00Z", if id == "a" { 2 } else { 1 }),
            priority,
        );
        replay.cost_microdollars = cost;
        replay.wall_ms = wall_ms;
        replay.validated_output = Some(json!({
            "category": category,
            "priority": priority,
            "tags": tags,
            "rationale": "fixture"
        }));
        write_record(&output, &format!("{id}.json"), &replay);
    }
    let result = freeze(&options(output, experiment.clone(), context)).unwrap();
    let metrics = compute(&result.manifest);
    assert_eq!(metrics.priority_histogram.get(&5), Some(&1));
    assert_eq!(metrics.category_histogram.get("Technology"), Some(&3));
    assert_eq!(metrics.category_histogram.get("Business"), Some(&2));
    assert_eq!(metrics.tag_histogram.get("ai"), Some(&4));
    assert_eq!(metrics.tag_histogram.get("markets"), Some(&2));
    assert_eq!(metrics.tag_cardinality_histogram.get(&0), Some(&1));
    assert_eq!(metrics.tag_cardinality_histogram.get(&3), Some(&1));
    assert_eq!(metrics.cost_microdollars_total, 210);
    assert_eq!(metrics.cost_microdollars_sync, 190);
    assert_eq!(metrics.cost_microdollars_batch, 20);
    assert_eq!(metrics.cost_microdollars_per_article, 35.0);
    assert_eq!(metrics.cost_microdollars_per_sync_article, 38.0);
    assert_eq!(metrics.cost_microdollars_per_batch_article, 20.0);
    assert_eq!(metrics.batch_latency_excluded, 1);
    assert_eq!(metrics.sync_zero_latency_excluded, 1);
    assert_eq!(metrics.batch_records, 1);
    let latency = metrics.historical_sync_latency.as_ref().unwrap();
    assert_eq!(latency.count, 4);
    assert_eq!(latency.median_ms, 20);
    assert_eq!(latency.p95_ms, 40);
    assert!(rate_label(1, 6).contains("n = 6"));
    assert_eq!(
        fs::read_to_string(experiment.join("rubric.txt")).unwrap(),
        "fixture rubric"
    );
    let report_dir = write_baseline_report(&manifest_path(&experiment), None).unwrap();
    let markdown = fs::read_to_string(report_dir.join("metrics.md")).unwrap();
    assert!(markdown.contains("Tag histogram:"));
    assert!(markdown.contains("Tag cardinality histogram:"));
    assert!(markdown.contains("38.00/sync article"));
    assert!(markdown.contains("20.00/batch article"));
    assert!(markdown.contains("n = 4, percentiles not reported"));
    fs::write(experiment.join("rubric.txt"), "changed").unwrap();
    let error = load_manifest(&manifest_path(&experiment))
        .unwrap_err()
        .to_string();
    assert!(error.contains("rubric.txt"));
    fs::write(experiment.join("rubric.txt"), "fixture rubric").unwrap();
    let article = &result.manifest.articles[0];
    fs::write(experiment.join(&article.text_path), "changed").unwrap();
    let error = load_manifest(&manifest_path(&experiment))
        .unwrap_err()
        .to_string();
    assert!(error.contains(&article.article_id));
}

#[test]
fn validation_failures_are_counted_not_selected() {
    let (_temp, output, experiment, context) = setup();
    let text = "evidence";
    let mut failed = record(
        &context,
        "bad",
        text,
        content_hash(text),
        false,
        "2026-01-01T00:00:00Z",
        3,
    );
    failed.validated_output = Some(json!({
        "category": "Technology",
        "priority": 9,
        "tags": ["ai"],
        "rationale": "stored before today's validator"
    }));
    write_record(&output, "bad.json", &failed);
    let good_text = "good evidence";
    write_record(
        &output,
        "good.json",
        &record(
            &context,
            "good",
            good_text,
            content_hash(good_text),
            false,
            "2026-01-02T00:00:00Z",
            3,
        ),
    );
    let result = freeze(&options(output, experiment, context)).unwrap();
    assert_eq!(result.manifest.articles.len(), 1);
    assert_eq!(result.manifest.articles[0].baseline.request_id, "good");
    assert_eq!(result.manifest.counts.validation_failed, 1);
}

#[test]
fn recovery_failures_are_counted_and_doctored_identity_is_reported() {
    let (_temp, output, experiment, context) = setup();
    let mut malformed = record(
        &context,
        "malformed",
        "evidence",
        content_hash("evidence"),
        false,
        "2026-01-03T00:00:00Z",
        3,
    );
    malformed.rendered_user_message =
        "Document:\n<document-aaaaaaaaaaaa>\nunterminated".to_string();
    write_record(&output, "malformed.json", &malformed);

    let doctored_text = "doctored evidence";
    write_record(
        &output,
        "doctored.json",
        &record(
            &context,
            "doctored",
            doctored_text,
            content_hash("different evidence"),
            false,
            "2026-01-02T00:00:00Z",
            3,
        ),
    );
    let full = "batch full evidence with material after the prefix";
    let truncated = format!("batch full{}", harvester_engine::TRUNCATION_MARKER);
    write_record(
        &output,
        "batch.json",
        &record(
            &context,
            "batch-truncated",
            &truncated,
            content_hash(full),
            true,
            "2026-01-01T00:00:00Z",
            3,
        ),
    );

    let result = freeze(&options(output, experiment, context)).unwrap();
    assert_eq!(result.manifest.counts.recovery_failed, 1);
    assert_eq!(result.manifest.counts.identity_anomalies, 1);
    let doctored = result
        .manifest
        .articles
        .iter()
        .find(|article| article.baseline.request_id == "doctored")
        .unwrap();
    assert!(doctored.identity_anomaly.is_some());
    let batch = result
        .manifest
        .articles
        .iter()
        .find(|article| article.baseline.request_id == "batch-truncated")
        .unwrap();
    assert!(batch.truncated);
    assert!(batch.identity_anomaly.is_none());
    assert_eq!(result.manifest.counts.batch_records, 1);
}

#[test]
fn phase_one_rejects_non_v4_prompt_selection() {
    let (_temp, output, experiment, context) = setup();
    let mut opts = options(output, experiment, context);
    opts.prompt_version = 3;
    assert!(freeze(&opts)
        .unwrap_err()
        .to_string()
        .contains("only --prompt-version 4"));
}
