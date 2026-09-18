use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use harvester_engine::llm::content_hash as sha256;
use harvester_eval::config::{
    load_config, AcceptanceConfig, DatasetSelection, RunConfig, Transport,
};
use harvester_eval::identity::PathKind;
use harvester_eval::jev::question_id::question_id;
use harvester_eval::manifest::{
    ArticleEntry, Baseline, DatasetSplit, FrozenInputs, Manifest, ManifestCounts, Selection, Usage,
};
use harvester_eval::runner::store::{load_completed, load_records, ResultRecord};
use harvester_eval::runner::{run, RunOptions};
use serde_json::{json, Map, Value};
use tempfile::TempDir;

const RUBRIC: &str = "PRIORITY SCALE:\n- 5: highest\n- 4: high\n- 3: medium\n- 2: low\n- 1: lowest\n\nTAG GUIDANCE:\n";
const IDS: [&str; 3] = ["aaaaaaaaaaaaaaaa", "bbbbbbbbbbbbbbbb", "cccccccccccccccc"];
const TEXTS: [&str; 3] = ["article one", "article two", "article three"];

struct Fixture {
    _temp: TempDir,
    root: PathBuf,
    fake: PathBuf,
    experiment: PathBuf,
    config: RunConfig,
}

fn article(id: &str, text: &str, split: DatasetSplit) -> ArticleEntry {
    ArticleEntry {
        article_id: id.into(),
        evidence_hash: sha256(text),
        source_content_hash: sha256(text),
        source_content_hashes: vec![sha256(text)],
        path_kind: PathKind::Sync,
        text_path: format!("articles/{id}.txt"),
        text_bytes: text.len(),
        truncated: false,
        nonce: "000000000000".into(),
        nonce_verified: true,
        evidence_matches_source: true,
        identity_anomaly: None,
        split,
        duplicate_group: id.into(),
        instruction_like: false,
        article_file: None,
        ambiguous_paths: vec![],
        mapping_status: "unmapped".into(),
        baseline: Baseline {
            record_path: "fixture".into(),
            request_id: id.into(),
            path_kind: PathKind::Sync,
            model_id: "baseline".into(),
            recorded_utc: "2026-01-01T00:00:00Z".into(),
            priority: 3,
            category: "Technology".into(),
            tags: vec![],
            rationale: "fixture".into(),
            usage: Usage {
                input_tokens: 1,
                output_tokens: 1,
                cached_input_tokens: 0,
            },
            cost_microdollars: 1,
            wall_ms: 1,
            cache_status: "miss".into(),
        },
    }
}

fn full_response(config: &RunConfig, priority: &str) -> String {
    let mut answers = Map::new();
    answers.insert(
        "priority".into(),
        json!({
            "choice": priority,
            "probabilities": {"1":0.1,"2":0.1,"3":0.2,"4":0.3,"5":0.3},
            "confidence": 0.8
        }),
    );
    if config.questions.include_relevance {
        let legend = config
            .questions
            .relevance_levels
            .iter()
            .enumerate()
            .map(|(index, label)| (index.to_string(), Value::String(label.clone())))
            .collect::<Map<_, _>>();
        answers.insert(
            "relevance".into(),
            json!({
                "score": 3,
                "legend": legend,
                "probabilities": {"0":0.1,"1":0.1,"2":0.2,"3":0.3,"4":0.3}
            }),
        );
    }
    if config.questions.include_categories {
        for category in &config.questions.category_vocabulary {
            answers.insert(
                question_id("category", category),
                json!({"noul": if category == "Technology" { 0.8 } else { 0.2 }}),
            );
        }
    }
    if config.questions.include_tags {
        for tag in &config.questions.tag_vocabulary {
            answers.insert(
                question_id("tag", tag),
                json!({"noul": if tag == "capex" { 0.8 } else { 0.2 }}),
            );
        }
    }
    json!({
        "model":"jev-test",
        "usage":{"input_tokens":10,"output_tokens":2},
        "answers": answers
    })
    .to_string()
}

fn write_manifest(root: &Path, splits: [DatasetSplit; 3]) -> Manifest {
    let entries = IDS
        .iter()
        .zip(TEXTS)
        .zip(splits)
        .map(|((id, text), split)| article(id, text, split))
        .collect::<Vec<_>>();
    for (entry, text) in entries.iter().zip(TEXTS) {
        fs::write(root.join(&entry.text_path), text).unwrap();
    }
    fs::write(root.join("rubric.txt"), RUBRIC).unwrap();
    let mut manifest = Manifest {
        manifest_version: 1,
        tool_version: "0.1.0".into(),
        created_utc: "2026-01-01T00:00:00Z".into(),
        source_output_dir: "fixture".into(),
        selection: Selection {
            prompt_id: "ArticleTriage".into(),
            prompt_version: 4,
            limit: 3,
            ordering: "fixture".into(),
            dev_share: 1.0,
            split_seed: 0,
            context_file: "fixture".into(),
        },
        frozen_inputs: FrozenInputs {
            rubric_path: "rubric.txt".into(),
            rubric_sha256: sha256(RUBRIC),
            prompt_identity_path: "prompt-identity.txt".into(),
            system_sha256: "prompt".into(),
            template_sha256: "template".into(),
            context_version: 1,
        },
        counts: ManifestCounts::default(),
        articles: entries,
        manifest_hash: String::new(),
    };
    manifest.refresh_hash().unwrap();
    fs::write(
        root.join("manifest.json"),
        serde_json::to_vec(&manifest).unwrap(),
    )
    .unwrap();
    manifest
}

fn setup(splits: [DatasetSplit; 3]) -> Fixture {
    let temp = TempDir::new().unwrap();
    let root = temp.path().join("frozen");
    let fake = temp.path().join("fake");
    let experiment = temp.path().join("experiment");
    fs::create_dir_all(root.join("articles")).unwrap();
    fs::create_dir_all(&fake).unwrap();
    write_manifest(&root, splits);
    let mut config = RunConfig::default();
    config.dataset.manifest = root.join("manifest.json").to_string_lossy().into();
    config.dataset.split = DatasetSelection::Dev;
    config.run.run_id = Some("phase3".into());
    config.run.transport = Transport::Fake;
    config.run.fake_dir = Some(fake.to_string_lossy().into());
    config.jev.max_attempts = 3;
    config.jev.backoff_initial_ms = 0;
    config.jev.backoff_max_ms = 0;
    config.jev.jitter = false;
    Fixture {
        _temp: temp,
        root,
        fake,
        experiment,
        config,
    }
}

fn dev_fixture() -> Fixture {
    setup([DatasetSplit::Dev; 3])
}

fn results_path(fixture: &Fixture, run_id: &str) -> PathBuf {
    fixture
        .experiment
        .join("runs")
        .join(run_id)
        .join("results.jsonl")
}

async fn execute(fixture: &Fixture, config: RunConfig) -> anyhow::Result<()> {
    run(RunOptions {
        experiment_dir: fixture.experiment.clone(),
        config,
        dry_run: false,
    })
    .await
}

fn write_default(fixture: &Fixture) -> String {
    let body = full_response(&fixture.config, "4");
    fs::write(fixture.fake.join("default.json"), &body).unwrap();
    body
}

#[tokio::test]
async fn fake_runner_exercises_all_questions_retries_raw_and_idempotent_resume() {
    let fixture = dev_fixture();
    let body = write_default(&fixture);
    fs::write(
        fixture.fake.join("scenario.json"),
        json!({IDS[0]:[
            {"status":429,"latency_ms":1},
            {"status":529,"latency_ms":2},
            {"body":body,"latency_ms":3}
        ]})
        .to_string(),
    )
    .unwrap();

    execute(&fixture, fixture.config.clone()).await.unwrap();
    let results = results_path(&fixture, "phase3");
    let records = load_records(&results).unwrap();
    assert_eq!(records.len(), 3);
    let retried = records
        .iter()
        .find(|record| record.article_id == IDS[0])
        .unwrap();
    assert_eq!(retried.attempt_count, 3);
    assert_eq!(
        retried.retry_reasons,
        ["rate_limited", "provider_overloaded"]
    );
    assert_eq!(retried.first_attempt_latency_ms, Some(1));
    assert_eq!(retried.model_latency_ms, Some(3));
    assert!(retried.model_latency_ms.unwrap() <= retried.total_elapsed_ms);

    for record in &records {
        assert_eq!(record.relevance_score, Some(3.0));
        assert!(record.relevance_probabilities.is_some());
        assert_eq!(
            record.categories_selected.as_deref().unwrap(),
            ["Technology"]
        );
        assert_eq!(record.tags_selected.as_deref().unwrap(), ["capex"]);
        assert_eq!(record.raw_response_paths.len(), 1);
        assert_eq!(
            fs::read(
                fixture
                    .experiment
                    .join("runs/phase3")
                    .join(&record.raw_response_paths[0])
            )
            .unwrap(),
            body.as_bytes()
        );
    }

    let run_file: Value =
        serde_json::from_slice(&fs::read(fixture.experiment.join("runs/phase3/run.json")).unwrap())
            .unwrap();
    assert_eq!(run_file["identity"]["run_id"], "phase3");
    assert_eq!(run_file["records_written"], 3);
    assert_eq!(run_file["superseded_records"], 0);
    assert!(run_file["recovery_bytes"].is_null());
    assert_eq!(run_file["recovery_paths"], json!([]));
    assert_eq!(run_file["resolved_config"]["run"]["concurrency"], 1);

    let before = fs::read(&results).unwrap();
    let started = run_file["started_utc"].clone();
    execute(&fixture, fixture.config.clone()).await.unwrap();
    assert_eq!(before, fs::read(&results).unwrap());
    let resumed: Value =
        serde_json::from_slice(&fs::read(fixture.experiment.join("runs/phase3/run.json")).unwrap())
            .unwrap();
    assert_eq!(resumed["records_written"], 3);
    assert_eq!(resumed["started_utc"], started);
}

#[tokio::test]
async fn malformed_body_is_invalid_without_priority_and_raw_bytes_are_exact() {
    let fixture = dev_fixture();
    let mut config = fixture.config.clone();
    config.dataset.limit = 1;
    let malformed = full_response(&config, "9");
    fs::write(fixture.fake.join("default.json"), &malformed).unwrap();

    let error = execute(&fixture, config).await.unwrap_err();
    assert!(error.to_string().contains("zero successful"));
    let records = load_records(&results_path(&fixture, "phase3")).unwrap();
    assert_eq!(records.len(), 1);
    let record = &records[0];
    assert_eq!(record.outcome, "invalid");
    assert_eq!(record.error_type.as_deref(), Some("validation"));
    assert!(record
        .error_detail
        .as_deref()
        .unwrap()
        .contains("choice label \"9\" for \"priority\" is outside"));
    assert_eq!(record.priority, None);
    assert_eq!(record.raw_response_paths.len(), 1);
    assert_eq!(
        fs::read(
            fixture
                .experiment
                .join("runs/phase3")
                .join(&record.raw_response_paths[0])
        )
        .unwrap(),
        malformed.as_bytes()
    );
}

#[tokio::test]
async fn terminal_401_aborts_and_preserves_prior_records() {
    let fixture = dev_fixture();
    write_default(&fixture);
    fs::write(
        fixture.fake.join("scenario.json"),
        json!({IDS[1]:[{"status":401}]}).to_string(),
    )
    .unwrap();

    let error = execute(&fixture, fixture.config.clone()).await.unwrap_err();
    assert!(error.to_string().contains("terminal provider failure"));
    let records = load_records(&results_path(&fixture, "phase3")).unwrap();
    assert_eq!(records.len(), 2);
    assert_eq!(records[0].outcome, "ok");
    assert_eq!(records[1].http_status, Some(401));
    assert_eq!(records[1].error_type.as_deref(), Some("access_denied"));
}

#[tokio::test]
async fn incompatible_resume_names_manifest_and_transport_and_new_run_id_proceeds() {
    let fixture = dev_fixture();
    write_default(&fixture);
    execute(&fixture, fixture.config.clone()).await.unwrap();
    let results = results_path(&fixture, "phase3");
    let before = fs::read(&results).unwrap();

    let manifest_path = fixture.root.join("manifest.json");
    let mut manifest: Manifest =
        serde_json::from_slice(&fs::read(&manifest_path).unwrap()).unwrap();
    manifest.selection.ordering = "changed fixture ordering".into();
    manifest.refresh_hash().unwrap();
    fs::write(&manifest_path, serde_json::to_vec(&manifest).unwrap()).unwrap();
    let error = execute(&fixture, fixture.config.clone()).await.unwrap_err();
    assert!(error.to_string().contains("manifest_hash changed"));
    assert_eq!(before, fs::read(&results).unwrap());

    let mut new_run = fixture.config.clone();
    new_run.run.run_id = Some("new-run".into());
    execute(&fixture, new_run).await.unwrap();
    assert_eq!(
        load_records(&results_path(&fixture, "new-run"))
            .unwrap()
            .len(),
        3
    );

    let new_results = results_path(&fixture, "new-run");
    let new_before = fs::read(&new_results).unwrap();
    let mut live = fixture.config.clone();
    live.run.run_id = Some("new-run".into());
    live.run.transport = Transport::Live;
    let error = execute(&fixture, live).await.unwrap_err();
    assert!(error.to_string().contains("transport_kind changed"));
    assert_eq!(new_before, fs::read(&new_results).unwrap());
}

#[tokio::test]
async fn retry_failed_appends_sequences_and_noncolliding_raw_paths() {
    let fixture = dev_fixture();
    let mut config = fixture.config.clone();
    config.dataset.limit = 1;
    let invalid = full_response(&config, "9");
    let valid = full_response(&config, "4");
    fs::write(fixture.fake.join("default.json"), &valid).unwrap();
    fs::write(
        fixture.fake.join("scenario.json"),
        json!({IDS[0]:{"passes":[
            [{"body":invalid}],
            [{"body":invalid}],
            [{"body":valid}]
        ]}})
        .to_string(),
    )
    .unwrap();

    assert!(execute(&fixture, config.clone()).await.is_err());
    config.run.retry_failed = true;
    assert!(execute(&fixture, config.clone()).await.is_err());
    execute(&fixture, config.clone()).await.unwrap();
    let path = results_path(&fixture, "phase3");
    let records = load_records(&path).unwrap();
    assert_eq!(records.len(), 3);
    assert_eq!(
        records
            .iter()
            .map(|record| record.attempt_sequence)
            .collect::<Vec<_>>(),
        [1, 2, 3]
    );
    let raw_paths = records
        .iter()
        .map(|record| record.raw_response_paths[0].clone())
        .collect::<std::collections::BTreeSet<_>>();
    assert_eq!(raw_paths.len(), 3);
    let completion = load_completed(&path).unwrap();
    assert!(completion.completed.contains(&(IDS[0].into(), 1)));
    assert_eq!(completion.superseded_records, 2);
    let before = fs::read(&path).unwrap();
    execute(&fixture, config).await.unwrap();
    assert_eq!(before, fs::read(&path).unwrap());
    let run_file: Value =
        serde_json::from_slice(&fs::read(fixture.experiment.join("runs/phase3/run.json")).unwrap())
            .unwrap();
    assert_eq!(run_file["records_written"], 3);
    assert_eq!(run_file["superseded_records"], 2);
}

#[tokio::test]
async fn fake_scenarios_restart_for_each_repetition() {
    let fixture = dev_fixture();
    let mut config = fixture.config.clone();
    config.dataset.limit = 1;
    config.dataset.repeat = 2;
    let body = full_response(&config, "4");
    fs::write(fixture.fake.join("default.json"), &body).unwrap();
    fs::write(
        fixture.fake.join("scenario.json"),
        json!({IDS[0]:[{"status":429},{"body":body}]}).to_string(),
    )
    .unwrap();
    execute(&fixture, config).await.unwrap();
    let records = load_records(&results_path(&fixture, "phase3")).unwrap();
    assert_eq!(records.len(), 2);
    assert!(records.iter().all(|record| record.attempt_count == 2));
    assert_eq!(
        records
            .iter()
            .map(|record| record.repetition)
            .collect::<Vec<_>>(),
        [1, 2]
    );
}

#[tokio::test]
async fn repeated_store_recovery_is_recorded_and_results_remain_valid() {
    let fixture = dev_fixture();
    write_default(&fixture);
    execute(&fixture, fixture.config.clone()).await.unwrap();
    let path = results_path(&fixture, "phase3");
    for fragment in [b"first".as_slice(), b"second".as_slice()] {
        let mut bytes = fs::read(&path).unwrap();
        bytes.extend_from_slice(fragment);
        fs::write(&path, bytes).unwrap();
        execute(&fixture, fixture.config.clone()).await.unwrap();
        assert_eq!(load_records(&path).unwrap().len(), 3);
    }
    let run_dir = fixture.experiment.join("runs/phase3");
    let quarantines = fs::read_dir(&run_dir)
        .unwrap()
        .filter_map(Result::ok)
        .filter(|entry| entry.file_name().to_string_lossy().contains(".broken-"))
        .count();
    assert_eq!(quarantines, 2);
    let run_file: Value =
        serde_json::from_slice(&fs::read(run_dir.join("run.json")).unwrap()).unwrap();
    assert_eq!(run_file["recovery_bytes"], 11);
    assert_eq!(run_file["recovery_paths"].as_array().unwrap().len(), 2);
}

fn comparable_records(records: Vec<ResultRecord>) -> BTreeMap<(String, u32), Value> {
    records
        .into_iter()
        .map(|record| {
            let pair = record.pair();
            let mut value = serde_json::to_value(record).unwrap();
            let object = value.as_object_mut().unwrap();
            for dynamic in [
                "run_id",
                "config_hash",
                "timestamp_utc",
                "total_elapsed_ms",
                "raw_response_paths",
            ] {
                object.remove(dynamic);
            }
            (pair, value)
        })
        .collect()
}

#[tokio::test]
async fn bounded_concurrency_matches_sequential_record_set() {
    let fixture = dev_fixture();
    let body = write_default(&fixture);
    fs::write(
        fixture.fake.join("scenario.json"),
        json!({
            IDS[0]: [{"body":body,"latency_ms":20}],
            IDS[1]: [{"body":body,"latency_ms":10}],
            IDS[2]: [{"body":body,"latency_ms":1}]
        })
        .to_string(),
    )
    .unwrap();
    let mut sequential = fixture.config.clone();
    sequential.run.run_id = Some("sequential".into());
    sequential.run.concurrency = 1;
    execute(&fixture, sequential).await.unwrap();
    let mut concurrent = fixture.config.clone();
    concurrent.run.run_id = Some("concurrent".into());
    concurrent.run.concurrency = 3;
    execute(&fixture, concurrent).await.unwrap();
    assert_eq!(
        comparable_records(load_records(&results_path(&fixture, "sequential")).unwrap()),
        comparable_records(load_records(&results_path(&fixture, "concurrent")).unwrap())
    );
}

#[tokio::test]
async fn heldout_gate_names_requirements_and_dev_only_all_proceeds() {
    let fixture = setup([DatasetSplit::Heldout, DatasetSplit::Dev, DatasetSplit::Dev]);
    write_default(&fixture);
    for split in [DatasetSelection::Heldout, DatasetSelection::All] {
        let mut config = fixture.config.clone();
        config.dataset.split = split;
        let error = execute(&fixture, config).await.unwrap_err();
        let message = error.to_string();
        for key in [
            "high_priority_recall_min",
            "severe_miss_rate_max",
            "max_p95_latency_ms",
            "agreed_utc",
            "agreed_note",
        ] {
            assert!(message.contains(key));
        }
    }

    let dev_only = dev_fixture();
    write_default(&dev_only);
    let mut config = dev_only.config.clone();
    config.dataset.split = DatasetSelection::All;
    execute(&dev_only, config).await.unwrap();
}

#[tokio::test]
async fn empty_acceptance_agreement_metadata_is_rejected() {
    let fixture = setup([DatasetSplit::Heldout; 3]);
    let mut config = fixture.config.clone();
    config.dataset.split = DatasetSelection::Heldout;
    config.acceptance = Some(AcceptanceConfig {
        high_priority_recall_min: 0.9,
        severe_miss_rate_max: 0.1,
        max_p95_latency_ms: 1_000,
        agreed_utc: " ".into(),
        agreed_note: "agreed".into(),
    });
    let error = execute(&fixture, config).await.unwrap_err();
    assert!(error
        .to_string()
        .contains("acceptance.agreed_utc must not be empty"));
}

#[test]
fn incomplete_acceptance_table_reports_the_toml_field() {
    let fixture = dev_fixture();
    let path = fixture.root.join("incomplete.toml");
    fs::write(
        &path,
        "[acceptance]\nhigh_priority_recall_min = 0.9\nsevere_miss_rate_max = 0.1\n",
    )
    .unwrap();
    let error = load_config(&path).unwrap_err();
    assert!(error
        .to_string()
        .contains("missing field `max_p95_latency_ms`"));
}

#[tokio::test]
async fn exhausted_transport_failures_keep_diagnostic_detail() {
    let fixture = dev_fixture();
    let mut config = fixture.config.clone();
    config.dataset.limit = 2;
    config.jev.max_attempts = 1;
    fs::write(
        fixture.fake.join("scenario.json"),
        json!({
            IDS[0]: [{"connection_error":"connection reset by peer"}],
            IDS[1]: [{"timeout":true}]
        })
        .to_string(),
    )
    .unwrap();
    assert!(execute(&fixture, config).await.is_err());
    let records = load_records(&results_path(&fixture, "phase3")).unwrap();
    assert_eq!(records[0].outcome, "transport_error");
    assert_eq!(
        records[0].error_detail.as_deref(),
        Some("connection reset by peer")
    );
    assert_eq!(records[1].outcome, "timeout");
    assert_eq!(
        records[1].error_detail.as_deref(),
        Some("request timed out")
    );
}

#[test]
fn dry_run_multiplies_estimates_by_repeat_and_writes_no_run() {
    let fixture = dev_fixture();
    let mut config = fixture.config.clone();
    config.dataset.repeat = 3;
    let config_path = fixture.root.join("dry-run.toml");
    fs::write(&config_path, toml::to_string(&config).unwrap()).unwrap();
    let expected_tokens = TEXTS.iter().map(|text| text.len() / 4).sum::<usize>() * 3;
    let output = Command::new(env!("CARGO_BIN_EXE_harvester_eval"))
        .args([
            "--experiment-dir",
            fixture.experiment.to_str().unwrap(),
            "run",
            "--config",
            config_path.to_str().unwrap(),
            "--dry-run",
        ])
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8(output.stdout).unwrap();
    assert!(stdout.contains(&format!("estimated input tokens {expected_tokens}")));
    assert!(!fixture.experiment.join("runs/phase3").exists());
}

#[test]
fn dummy_key_never_appears_in_fake_run_artefacts() {
    let fixture = dev_fixture();
    let body = write_default(&fixture);
    let config_path = fixture.root.join("key-hygiene.toml");
    fs::write(&config_path, toml::to_string(&fixture.config).unwrap()).unwrap();
    let secret = "dummy-typesafe-key-must-not-leak";
    let output = Command::new(env!("CARGO_BIN_EXE_harvester_eval"))
        .args([
            "--experiment-dir",
            fixture.experiment.to_str().unwrap(),
            "run",
            "--config",
            config_path.to_str().unwrap(),
        ])
        .env("TYPESAFE_AI_API_KEY", secret)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let run_dir = fixture.experiment.join("runs/phase3");
    let records = fs::read(run_dir.join("results.jsonl")).unwrap();
    let run_file = fs::read(run_dir.join("run.json")).unwrap();
    let log = fs::read(fixture.experiment.join("harvester_eval.log")).unwrap();
    assert!(!records
        .windows(secret.len())
        .any(|window| window == secret.as_bytes()));
    assert!(!run_file
        .windows(secret.len())
        .any(|window| window == secret.as_bytes()));
    assert!(!log
        .windows(secret.len())
        .any(|window| window == secret.as_bytes()));
    for record in load_records(&run_dir.join("results.jsonl")).unwrap() {
        for raw_path in record.raw_response_paths {
            let raw = fs::read(run_dir.join(raw_path)).unwrap();
            assert_eq!(raw, body.as_bytes());
            assert!(!raw
                .windows(secret.len())
                .any(|window| window == secret.as_bytes()));
        }
    }
}
