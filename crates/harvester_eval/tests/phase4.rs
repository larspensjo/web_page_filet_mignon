use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

use harvester_engine::llm::content_hash as sha256;
use harvester_eval::config::{AcceptanceConfig, DatasetSelection, RunConfig, Transport};
use harvester_eval::identity::PathKind;
use harvester_eval::jev::question_id::question_id;
use harvester_eval::manifest::{
    ArticleEntry, ArticleFile, Baseline, DatasetSplit, FrozenInputs, Manifest, ManifestCounts,
    Selection, Usage,
};
use harvester_eval::metrics::compare::compute;
use harvester_eval::metrics::small_sample::{rate_label, Rate};
use harvester_eval::report::compare::{
    render_markdown as render_comparison_markdown, ComparisonReport,
};
use harvester_eval::review::build::{
    build_review_rows, csv_escape, render_csv, ReviewCsvRow, ReviewKey, ReviewKeyRow,
};
use harvester_eval::review::diagnose::render_diagnosis;
use harvester_eval::review::score::{parse_csv, render_markdown, score_review, Evaluation};
use harvester_eval::runner::store::{select_records, ResultRecord, RESULT_SCHEMA_VERSION};
use serde_json::{json, Map, Value};
use tempfile::TempDir;

const RUBRIC: &str = "PRIORITY SCALE:\n- 5: highest\n- 4: high\n- 3: medium\n- 2: low\n- 1: lowest\n\nTAG GUIDANCE:\n";

fn article(
    id: &str,
    priority: u8,
    tags: &[&str],
    _categories: &[&str],
    path: PathKind,
) -> ArticleEntry {
    ArticleEntry {
        article_id: id.into(),
        evidence_hash: format!("hash-{id}"),
        source_content_hash: format!("source-{id}"),
        source_content_hashes: vec![],
        path_kind: path,
        text_path: format!("articles/{id}.txt"),
        text_bytes: 20,
        truncated: false,
        nonce: "nonce".into(),
        nonce_verified: true,
        evidence_matches_source: true,
        identity_anomaly: None,
        split: DatasetSplit::Heldout,
        duplicate_group: id.into(),
        instruction_like: false,
        article_file: Some(ArticleFile {
            path: format!("{id}.md"),
            title: Some(format!("Title {id}")),
            url: None,
            fetched_utc: None,
        }),
        ambiguous_paths: vec![],
        mapping_status: "mapped".into(),
        baseline: Baseline {
            record_path: format!("{id}.json"),
            request_id: id.into(),
            path_kind: path,
            model_id: "gpt".into(),
            recorded_utc: "2026-01-01T00:00:00Z".into(),
            priority,
            category: "Technology".into(),
            tags: tags.iter().map(|tag| (*tag).into()).collect(),
            rationale: format!("why {id}"),
            usage: Usage {
                input_tokens: 10,
                output_tokens: 2,
                cached_input_tokens: 0,
            },
            cost_microdollars: 100,
            wall_ms: if path == PathKind::Sync { 10 } else { 0 },
            cache_status: "miss".into(),
        },
    }
}

fn manifest(articles: Vec<ArticleEntry>) -> Manifest {
    Manifest {
        manifest_version: 1,
        tool_version: "test".into(),
        created_utc: String::new(),
        source_output_dir: "test".into(),
        selection: Selection {
            prompt_id: "ArticleTriage".into(),
            prompt_version: 4,
            limit: articles.len(),
            ordering: "test".into(),
            dev_share: 0.5,
            split_seed: 0,
            context_file: "test".into(),
        },
        frozen_inputs: FrozenInputs {
            rubric_path: "rubric.txt".into(),
            rubric_sha256: "rubric".into(),
            prompt_identity_path: "prompt.txt".into(),
            system_sha256: "system".into(),
            template_sha256: "template".into(),
            context_version: 3,
        },
        counts: ManifestCounts::default(),
        articles,
        manifest_hash: "manifest".into(),
    }
}

fn record(id: &str, priority: Option<u8>, outcome: &str, repetition: u32) -> ResultRecord {
    let mut priority_probabilities = BTreeMap::new();
    for value in 1..=5 {
        priority_probabilities.insert(
            value.to_string(),
            if Some(value) == priority { 1.0 } else { 0.0 },
        );
    }
    ResultRecord {
        schema_version: RESULT_SCHEMA_VERSION,
        run_id: "run".into(),
        config_hash: "config".into(),
        manifest_hash: "manifest".into(),
        prompt_identity_hash: "prompt".into(),
        rubric_sha256: "rubric".into(),
        transport_kind: harvester_eval::config::Transport::Fake,
        article_id: id.into(),
        evidence_hash: format!("hash-{id}"),
        split: "heldout".into(),
        repetition,
        attempt_sequence: 1,
        provider: "jev".into(),
        requested_model: "jev".into(),
        returned_model: Some("jev".into()),
        timestamp_utc: "2026-01-01T00:00:00Z".into(),
        outcome: outcome.into(),
        priority,
        priority_probabilities: priority.map(|_| priority_probabilities),
        priority_confidence: Some(0.8),
        p_high: Some(0.5),
        relevance_score: Some(3.0),
        relevance_legend: None,
        relevance_probabilities: None,
        category_probabilities: Some(BTreeMap::new()),
        categories_selected: Some(vec![]),
        category_threshold: 0.5,
        tag_probabilities: Some(BTreeMap::new()),
        tags_selected: Some(vec![]),
        tag_threshold: 0.5,
        input_tokens: Some(1_000_000),
        output_tokens: Some(1),
        first_attempt_latency_ms: Some(10),
        model_latency_ms: Some(10),
        total_elapsed_ms: 20,
        attempt_count: 1,
        retry_reasons: vec![],
        http_status: Some(200),
        error_type: (outcome != "ok").then(|| "test_failure".into()),
        error_detail: None,
        estimated_cost_microdollars: Some(42_000),
        pricing_version: "test-price".into(),
        request_bytes: 1,
        response_bytes: 1,
        raw_response_paths: vec![],
    }
}

struct CliFixture {
    _temp: TempDir,
    experiment: PathBuf,
    manifest_path: PathBuf,
    config_path: PathBuf,
    original_manifest_hash: String,
}

fn fake_response(config: &RunConfig, priority: &str) -> String {
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
    for category in &config.questions.category_vocabulary {
        answers.insert(
            question_id("category", category),
            json!({"noul": if category == "Technology" { 0.8 } else { 0.2 }}),
        );
    }
    for tag in &config.questions.tag_vocabulary {
        answers.insert(
            question_id("tag", tag),
            json!({"noul": if tag == "capex" { 0.8 } else { 0.2 }}),
        );
    }
    json!({
        "model": "jev-test",
        "usage": {"input_tokens": 10, "output_tokens": 2},
        "answers": answers
    })
    .to_string()
}

fn cli_fixture() -> CliFixture {
    let temp = TempDir::new().unwrap();
    let frozen = temp.path().join("frozen-outside-experiment");
    let fake = temp.path().join("fake");
    let experiment = temp.path().join("experiment");
    fs::create_dir_all(frozen.join("articles")).unwrap();
    fs::create_dir_all(&fake).unwrap();
    let id = "aaaaaaaaaaaaaaaa";
    let text = "article text for phase four";
    let mut entry = article(id, 3, &[], &[], PathKind::Sync);
    entry.split = DatasetSplit::Dev;
    entry.evidence_hash = sha256(text);
    entry.source_content_hash = sha256(text);
    entry.source_content_hashes = vec![sha256(text)];
    entry.text_bytes = text.len();
    fs::write(frozen.join(&entry.text_path), text).unwrap();
    fs::write(frozen.join("rubric.txt"), RUBRIC).unwrap();
    let mut frozen_manifest = manifest(vec![entry]);
    frozen_manifest.created_utc = "2026-01-01T00:00:00Z".into();
    frozen_manifest.frozen_inputs.rubric_path = "rubric.txt".into();
    frozen_manifest.frozen_inputs.rubric_sha256 = sha256(RUBRIC);
    frozen_manifest.refresh_hash().unwrap();
    let original_manifest_hash = frozen_manifest.manifest_hash.clone();
    let manifest_path = frozen.join("manifest.json");
    fs::write(
        &manifest_path,
        serde_json::to_vec_pretty(&frozen_manifest).unwrap(),
    )
    .unwrap();
    let mut config = RunConfig::default();
    config.dataset.manifest = manifest_path.to_string_lossy().into();
    config.dataset.split = DatasetSelection::Dev;
    config.run.run_id = Some("phase4-e2e".into());
    config.run.transport = Transport::Fake;
    config.run.fake_dir = Some(fake.to_string_lossy().into());
    config.jev.max_attempts = 1;
    config.jev.backoff_initial_ms = 0;
    config.jev.backoff_max_ms = 0;
    config.jev.jitter = false;
    fs::write(fake.join("default.json"), fake_response(&config, "4")).unwrap();
    let config_path = temp.path().join("run.toml");
    fs::write(&config_path, toml::to_string(&config).unwrap()).unwrap();
    CliFixture {
        _temp: temp,
        experiment,
        manifest_path,
        config_path,
        original_manifest_hash,
    }
}

fn run_cli(fixture: &CliFixture, arguments: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_harvester_eval"))
        .arg("--experiment-dir")
        .arg(&fixture.experiment)
        .args(arguments)
        .output()
        .unwrap()
}

fn assert_cli_success(output: &Output) {
    assert!(
        output.status.success(),
        "stdout: {}\nstderr: {}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}

fn fill_review(path: &Path, priority: &str) {
    let mut rows = parse_csv(&fs::read_to_string(path).unwrap()).unwrap();
    for row in rows.iter_mut().skip(1) {
        row[7] = priority.into();
    }
    let mut csv = rows
        .iter()
        .map(|row| {
            row.iter()
                .map(|field| csv_escape(field))
                .collect::<Vec<_>>()
                .join(",")
        })
        .collect::<Vec<_>>()
        .join("\n");
    csv.push('\n');
    fs::write(path, csv).unwrap();
}

#[test]
fn comparison_confusion_agreement_within_one_and_mae_are_hand_computed() {
    let baseline_priorities = [1, 2, 4, 5, 4, 3, 4, 4];
    let articles = baseline_priorities
        .iter()
        .enumerate()
        .map(|(index, priority)| {
            article(
                &(index + 1).to_string(),
                *priority,
                &[],
                &[],
                PathKind::Sync,
            )
        })
        .collect::<Vec<_>>();
    let manifest = manifest(articles);
    let priorities = [1, 2, 3, 4, 5, 5, 1, 2];
    let records = priorities
        .iter()
        .enumerate()
        .map(|(index, priority)| record(&(index + 1).to_string(), Some(*priority), "ok", 1))
        .collect::<Vec<_>>();
    let metrics = compute(&manifest, &records, &RunConfig::default());
    assert_eq!(metrics.successful_records, 8);
    assert_eq!(metrics.exact_agreement, Rate::new(2, 8));
    assert_eq!(metrics.within_one, Rate::new(5, 8));
    assert_eq!(metrics.mean_absolute_error, Some(1.25));
    assert_eq!(
        metrics.confusion_matrix,
        [
            [1, 0, 0, 0, 0],
            [0, 1, 0, 0, 0],
            [0, 0, 0, 0, 1],
            [1, 1, 1, 0, 1],
            [0, 0, 0, 1, 0],
        ]
    );
}

#[test]
fn high_priority_and_priority_five_rates_and_small_sample_suppression_are_explicit() {
    let manifest = manifest(vec![
        article("a", 5, &[], &[], PathKind::Sync),
        article("b", 4, &[], &[], PathKind::Sync),
        article("c", 2, &[], &[], PathKind::Sync),
    ]);
    let records = vec![
        record("a", Some(4), "ok", 1),
        record("b", Some(5), "ok", 1),
        record("c", Some(5), "ok", 1),
    ];
    let metrics = compute(&manifest, &records, &RunConfig::default());
    assert_eq!(metrics.priority_five_precision, Rate::new(0, 2));
    assert_eq!(metrics.priority_five_recall, Rate::new(0, 1));
    assert_eq!(metrics.high_priority_precision, Rate::new(2, 3));
    assert_eq!(metrics.high_priority_recall, Rate::new(2, 2));
    assert!(rate_label(2, 3, 30).contains("rate not reported"));
}

#[test]
fn severe_boundary_counts_and_tags_include_empty_sets() {
    let mut one = article("one", 5, &[], &[], PathKind::Sync);
    let mut two = article("two", 1, &["x"], &[], PathKind::Sync);
    let mut three = article("three", 1, &[], &[], PathKind::Sync);
    let mut four = article("four", 2, &["x", "y"], &[], PathKind::Sync);
    let mut five = article("five", 5, &["x"], &[], PathKind::Sync);
    let mut records = vec![
        record("one", Some(2), "ok", 1),
        record("two", Some(5), "ok", 1),
        record("three", Some(3), "ok", 1),
        record("four", Some(2), "ok", 1),
        record("five", Some(3), "ok", 1),
    ];
    one.baseline.tags.clear();
    two.baseline.tags = vec!["x".into()];
    three.baseline.tags.clear();
    four.baseline.tags = vec!["x".into(), "y".into()];
    five.baseline.tags = vec!["x".into()];
    for (record, tags) in records.iter_mut().zip([
        vec![],
        vec![],
        vec!["y".into()],
        vec!["x".into(), "z".into()],
        vec!["x".into()],
    ]) {
        record.tags_selected = Some(tags);
    }
    let metrics = compute(
        &manifest(vec![one, two, three, four, five]),
        &records,
        &RunConfig::default(),
    );
    assert_eq!(metrics.severe_demotions, 1);
    assert_eq!(metrics.severe_promotions, 1);
    assert_eq!(metrics.tags.both_empty_excluded, 1);
    assert_eq!(metrics.tags.articles, 4);
    assert_eq!(metrics.tags.mean_jaccard, Some(1.0 / 3.0));
    assert_eq!(metrics.tags.per_tag["x"].precision, Rate::new(2, 2));
    assert_eq!(metrics.tags.per_tag["x"].recall, Rate::new(2, 3));
    assert_eq!(metrics.tags.per_tag["x"].support, 3);
    assert_eq!(metrics.tags.per_tag["y"].precision, Rate::new(0, 1));
    assert_eq!(metrics.tags.per_tag["y"].recall, Rate::new(0, 1));
    assert_eq!(metrics.tags.per_tag["z"].precision, Rate::new(0, 1));
    assert_eq!(metrics.tags.per_tag["z"].recall, Rate::new(0, 0));
}

#[test]
fn categories_are_multilabel_symmetric_and_none_is_counted() {
    let manifest = manifest(vec![
        article("a", 3, &[], &[], PathKind::Sync),
        article("b", 3, &[], &[], PathKind::Sync),
    ]);
    let mut first = record("a", Some(3), "ok", 1);
    first.categories_selected = Some(vec![
        "Business".into(),
        "Technology".into(),
        "Finance & Markets".into(),
    ]);
    let mut second = record("b", Some(3), "ok", 1);
    second.categories_selected = Some(vec![]);
    let metrics = compute(&manifest, &[first, second], &RunConfig::default());
    assert_eq!(metrics.category.positive_rates["Business"], Rate::new(1, 2));
    assert_eq!(
        metrics.category.positive_rates["Technology"],
        Rate::new(1, 2)
    );
    assert_eq!(
        metrics.category.positive_rates["Finance & Markets"],
        Rate::new(1, 2)
    );
    assert_eq!(
        metrics.category.positive_rates["Politics & Regulation"],
        Rate::new(0, 2)
    );
    assert_eq!(
        metrics.category.positive_rates["Science & Research"],
        Rate::new(0, 2)
    );
    assert_eq!(metrics.category.cooccurrence["Business"]["Technology"], 1);
    assert_eq!(metrics.category.cooccurrence["Technology"]["Business"], 1);
    assert_eq!(
        metrics.category.cooccurrence["Business"]["Finance & Markets"],
        1
    );
    assert_eq!(metrics.category.cooccurrence["Business"]["Business"], 1);
    assert_eq!(metrics.category.zero_category_share, Rate::new(1, 2));
}

#[test]
fn stability_and_append_order_selection_match_the_store_contract() {
    let records = vec![
        record("a", Some(3), "timeout", 1),
        record("a", Some(4), "ok", 1),
        record("b", Some(2), "ok", 1),
        record("b", Some(5), "timeout", 1),
        record("c", Some(1), "timeout", 1),
    ];
    let (selected, superseded) = select_records(&records);
    assert_eq!(
        selected
            .iter()
            .map(|r| (&r.article_id, &r.outcome))
            .collect::<Vec<_>>(),
        vec![
            (&String::from("a"), &String::from("ok")),
            (&String::from("b"), &String::from("ok")),
            (&String::from("c"), &String::from("timeout")),
        ]
    );
    assert_eq!(superseded, 2);
    let selection_metrics = compute(
        &manifest(vec![
            article("a", 4, &[], &[], PathKind::Sync),
            article("b", 2, &[], &[], PathKind::Sync),
            article("c", 1, &[], &[], PathKind::Sync),
        ]),
        &records,
        &RunConfig::default(),
    );
    assert_eq!(selection_metrics.successful_records, 2);
    assert_eq!(selection_metrics.failure_count, 1);
    assert_eq!(selection_metrics.failures_by_outcome["timeout"], 1);
    assert_eq!(selection_metrics.superseded_records, 2);
    let mut config = RunConfig::default();
    config.dataset.repeat = 2;
    let manifest = manifest(vec![article("a", 3, &[], &[], PathKind::Sync)]);
    let repeated = vec![
        record("a", Some(3), "ok", 1),
        record("a", Some(4), "ok", 2),
        record("a", Some(4), "ok", 3),
    ];
    let metrics = compute(&manifest, &repeated, &config);
    assert_eq!(
        metrics.operational.stability.choice_change_rate,
        Rate::new(1, 1)
    );
}

#[test]
fn review_is_blinded_ranked_reproducible_and_excludes_failures() {
    let manifest = manifest(vec![
        article("gap4", 5, &[], &[], PathKind::Sync),
        article("gap2", 5, &[], &[], PathKind::Sync),
        article("boundary-a", 2, &[], &[], PathKind::Sync),
        article("boundary-b", 2, &[], &[], PathKind::Sync),
        article("nonboundary", 3, &[], &[], PathKind::Sync),
        article("agree", 3, &[], &[], PathKind::Sync),
        article("failure", 1, &[], &[], PathKind::Batch),
    ]);
    let records = vec![
        record("gap4", Some(1), "ok", 1),
        record("gap2", Some(3), "ok", 1),
        record("boundary-a", Some(3), "ok", 1),
        record("boundary-b", Some(3), "ok", 1),
        record("nonboundary", Some(4), "ok", 1),
        record("agree", Some(3), "ok", 1),
        record("failure", None, "invalid", 1),
    ];
    let texts = [
        "gap4",
        "gap2",
        "boundary-a",
        "boundary-b",
        "nonboundary",
        "agree",
        "failure",
    ]
    .into_iter()
    .map(|id| (id.into(), format!("text for {id}")))
    .collect();
    let first = build_review_rows(&manifest, &records, &texts, "run", 7, 100);
    let second = build_review_rows(&manifest, &records, &texts, "run", 7, 100);
    let changed = build_review_rows(&manifest, &records, &texts, "run", 8, 100);
    assert_eq!(first, second);
    assert!(first
        .key
        .rows
        .iter()
        .zip(changed.key.rows.iter())
        .any(|(a, b)| a.a_provider != b.a_provider));
    assert_eq!(
        first
            .rows
            .iter()
            .map(|r| r.article_id.as_str())
            .collect::<Vec<_>>(),
        vec!["gap4", "gap2", "boundary-a", "boundary-b", "nonboundary"]
    );
    let csv = render_csv(&first.rows);
    assert_eq!(
        csv.lines().next().unwrap(),
        "review_row,article_id,title,text_path,excerpt,priority_a,priority_b,your_priority,notes"
    );
    assert!(
        !csv.contains("openai")
            && !csv.contains("jev")
            && !csv.contains("why")
            && !csv.contains("tag")
            && !csv.contains("category")
    );
    assert_eq!(first.failures.len(), 1);
    assert_eq!(first.failures[0].article_id, "failure");
    for (row, key_row) in first.rows.iter().zip(&first.key.rows) {
        let baseline = manifest
            .articles
            .iter()
            .find(|article| article.article_id == row.article_id)
            .unwrap()
            .baseline
            .priority;
        let jev = records
            .iter()
            .find(|record| record.article_id == row.article_id)
            .unwrap()
            .priority
            .unwrap();
        assert_eq!(key_row.article_id, row.article_id);
        match (key_row.a_provider.as_str(), key_row.b_provider.as_str()) {
            ("jev", "openai") => {
                assert_eq!((row.priority_a, row.priority_b), (jev, baseline));
            }
            ("openai", "jev") => {
                assert_eq!((row.priority_a, row.priority_b), (baseline, jev));
            }
            providers => panic!("invalid provider assignment: {providers:?}"),
        }
    }
}

#[test]
fn scoring_arithmetic_stopping_thresholds_and_validation_are_public() {
    let rows = vec![
        ReviewCsvRow {
            review_row: 1,
            article_id: "a".into(),
            title: "".into(),
            text_path: "a".into(),
            excerpt: "".into(),
            priority_a: 1,
            priority_b: 2,
            your_priority: "1".into(),
            notes: "".into(),
        },
        ReviewCsvRow {
            review_row: 2,
            article_id: "b".into(),
            title: "".into(),
            text_path: "b".into(),
            excerpt: "".into(),
            priority_a: 4,
            priority_b: 5,
            your_priority: "4".into(),
            notes: "".into(),
        },
        ReviewCsvRow {
            review_row: 3,
            article_id: "c".into(),
            title: "".into(),
            text_path: "c".into(),
            excerpt: "".into(),
            priority_a: 5,
            priority_b: 2,
            your_priority: "4".into(),
            notes: "".into(),
        },
    ];
    let key = ReviewKey {
        blinding_seed: 1,
        run_id: "run".into(),
        rows: vec![
            ReviewKeyRow {
                review_row: 1,
                article_id: "a".into(),
                a_provider: "openai".into(),
                b_provider: "jev".into(),
            },
            ReviewKeyRow {
                review_row: 2,
                article_id: "b".into(),
                a_provider: "openai".into(),
                b_provider: "jev".into(),
            },
            ReviewKeyRow {
                review_row: 3,
                article_id: "c".into(),
                a_provider: "openai".into(),
                b_provider: "jev".into(),
            },
        ],
    };
    let csv = render_csv(&rows);
    let manifest = manifest(vec![
        article("a", 1, &[], &[], PathKind::Sync),
        article("b", 4, &[], &[], PathKind::Sync),
        article("c", 5, &[], &[], PathKind::Sync),
    ]);
    let records = vec![
        record("a", Some(2), "ok", 1),
        record("b", Some(5), "ok", 1),
        record("c", Some(2), "ok", 1),
    ];
    let mut config = RunConfig::default();
    config.review.min_reviewed_rows = 3;
    config.review.min_high_priority_support = 2;
    config.review.large_gap = 2;
    config.acceptance = Some(AcceptanceConfig {
        high_priority_recall_min: 0.5,
        severe_miss_rate_max: 0.5,
        max_p95_latency_ms: 20,
        agreed_utc: "now".into(),
        agreed_note: "test".into(),
    });
    let report = score_review(&csv, &key, &manifest, &records, &config).unwrap();
    assert_eq!(report.openai.exact_agreement, Rate::new(2, 3));
    assert_eq!(report.openai.within_one, Rate::new(3, 3));
    assert_eq!(report.openai.mean_absolute_error, Some(1.0 / 3.0));
    assert_eq!(report.openai.high_priority_recall, Rate::new(2, 2));
    assert_eq!(report.openai.high_priority_precision, Rate::new(2, 2));
    assert_eq!(report.openai.severe_miss_rate, Rate::new(0, 2));
    assert_eq!(report.openai.severe_false_positive_rate, Rate::new(0, 1));
    assert_eq!(report.openai.admission_boundary_agreement, Rate::new(3, 3));
    assert_eq!(report.jev.exact_agreement, Rate::new(0, 3));
    assert_eq!(report.jev.within_one, Rate::new(2, 3));
    assert_eq!(report.jev.mean_absolute_error, Some(4.0 / 3.0));
    assert_eq!(report.jev.high_priority_recall, Rate::new(1, 2));
    assert_eq!(report.jev.high_priority_precision, Rate::new(1, 1));
    assert_eq!(report.jev.severe_miss_rate, Rate::new(1, 2));
    assert_eq!(report.jev.severe_false_positive_rate, Rate::new(0, 1));
    assert_eq!(report.jev.admission_boundary_agreement, Rate::new(2, 3));
    assert!(report.stopping_rule.missing.is_empty());
    assert_eq!(report.overall, Evaluation::Pass);
    assert!(!report
        .thresholds
        .iter()
        .any(|threshold| threshold.name.contains("cost")));
    let rendered = render_markdown(&report);
    assert!(rendered.contains("biased reviewed subset"));
    assert!(rendered.contains("1/2"));
    assert!(rendered.contains("Overall: **pass**"));
    assert!(!rendered.contains("Some("));
    let bad = render_csv(&[ReviewCsvRow {
        your_priority: "7".into(),
        ..rows[0].clone()
    }]);
    let bad_key = ReviewKey {
        rows: vec![key.rows[0].clone()],
        ..key.clone()
    };
    assert!(score_review(&bad, &bad_key, &manifest, &records, &config).is_err());
}

#[test]
fn review_selection_uses_the_lowest_repetition_even_when_later_results_differ() {
    let manifest = manifest(vec![
        article("lowest-fails", 3, &[], &[], PathKind::Sync),
        article("lowest-succeeds", 3, &[], &[], PathKind::Sync),
    ]);
    let records = vec![
        record("lowest-fails", None, "timeout", 1),
        record("lowest-fails", Some(4), "ok", 2),
        record("lowest-succeeds", Some(4), "ok", 1),
        record("lowest-succeeds", None, "timeout", 2),
    ];
    let build = build_review_rows(&manifest, &records, &BTreeMap::new(), "run", 1, 100);
    assert_eq!(
        build
            .rows
            .iter()
            .map(|row| row.article_id.as_str())
            .collect::<Vec<_>>(),
        ["lowest-succeeds"]
    );
    assert_eq!(build.failures.len(), 1);
    assert_eq!(build.failures[0].article_id, "lowest-fails");
    assert_eq!(build.failures[0].repetition, 1);
}

#[test]
fn cost_latency_and_pricing_are_reported_without_a_cost_gate() {
    let manifest = manifest(vec![
        article("sync", 3, &[], &[], PathKind::Sync),
        article("batch", 3, &[], &[], PathKind::Batch),
        article("failed-high", 5, &[], &[], PathKind::Sync),
    ]);
    let mut sync_record = record("sync", Some(3), "ok", 1);
    sync_record.input_tokens = Some(1);
    let mut batch_record = record("batch", Some(3), "ok", 1);
    batch_record.input_tokens = Some(1);
    batch_record.model_latency_ms = Some(30);
    batch_record.total_elapsed_ms = 40;
    let records = vec![
        sync_record,
        batch_record,
        record("failed-high", None, "timeout", 1),
    ];
    let metrics = compute(&manifest, &records, &RunConfig::default());
    assert_eq!(metrics.attempted_records, 3);
    assert_eq!(metrics.successful_records, 2);
    assert_eq!(metrics.articles_without_successful_record, 1);
    assert_eq!(metrics.failed_high_priority_articles, 1);
    assert_eq!(metrics.failed_priority_five_articles, 1);
    assert_eq!(
        metrics.baseline_priority_histogram,
        BTreeMap::from([(3, 2)])
    );
    assert_eq!(metrics.jev_priority_histogram, BTreeMap::from([(3, 2)]));
    assert_eq!(metrics.high_priority_recall, Rate::new(0, 0));
    assert_eq!(metrics.operational.attempted_article_count, 3);
    assert_eq!(metrics.operational.successful_article_count, 2);
    assert_eq!(metrics.operational.openai_cost.article_count, 2);
    assert_eq!(metrics.operational.jev_cost.article_count, 2);
    assert_eq!(metrics.operational.openai_sync_cost.total_microdollars, 100);
    assert_eq!(
        metrics.operational.openai_batch_cost.total_microdollars,
        100
    );
    assert_eq!(metrics.operational.openai_batch_latency_excluded, 1);
    assert_eq!(
        metrics
            .operational
            .openai_sync_latency_historical
            .as_ref()
            .unwrap()
            .p95_ms,
        10
    );
    assert_eq!(metrics.operational.jev_cost.total_microdollars, 2);
    assert_eq!(
        metrics.operational.jev_pricing_version,
        "typesafe-jev-input-v1"
    );
    assert_eq!(
        metrics.operational.jev_latency_model.as_ref().unwrap(),
        &harvester_eval::metrics::operational::LatencySummary {
            count: 2,
            median_ms: 10,
            p95_ms: 30,
        }
    );
    assert_eq!(
        metrics.operational.jev_latency_total.as_ref().unwrap(),
        &harvester_eval::metrics::operational::LatencySummary {
            count: 2,
            median_ms: 20,
            p95_ms: 40,
        }
    );
}

#[test]
fn comparison_markdown_leads_with_comparable_class_counts_and_lists_failures() {
    let manifest = manifest(vec![
        article("failed", 5, &[], &[], PathKind::Sync),
        article("ok", 3, &[], &[], PathKind::Sync),
    ]);
    let metrics = compute(
        &manifest,
        &[
            record("failed", None, "timeout", 1),
            record("ok", Some(3), "ok", 1),
        ],
        &RunConfig::default(),
    );
    let report = ComparisonReport {
        manifest_hash: "manifest".into(),
        config_hash: "config".into(),
        rubric_hash: "rubric".into(),
        split: "heldout".into(),
        run_id: "run".into(),
        transport: Transport::Fake,
        tool_version: "test".into(),
        metrics,
    };
    let markdown = render_comparison_markdown(&report);
    assert!(
        markdown.find("### Class counts").unwrap() < markdown.find("### Confusion matrix").unwrap()
    );
    assert!(markdown.contains("|3|1|1|"));
    assert!(markdown.contains("Articles with no successful Jev record: 1"));
    assert!(markdown.contains("baseline priority 4–5: 1; baseline priority 5: 1"));
    assert!(markdown.contains("|`failed`|1|timeout|test_failure|"));
    assert!(markdown.contains("1 pairs with both tag sets empty excluded"));
    assert!(markdown.contains("percentiles not reported below the small-sample floor"));
}

#[test]
fn scoring_input_validation_handles_blanks_bom_whitespace_and_named_errors() {
    let rows = vec![
        ReviewCsvRow {
            review_row: 1,
            article_id: "a".into(),
            title: "".into(),
            text_path: "a".into(),
            excerpt: "".into(),
            priority_a: 1,
            priority_b: 4,
            your_priority: " 4 ".into(),
            notes: "".into(),
        },
        ReviewCsvRow {
            review_row: 2,
            article_id: "b".into(),
            title: "".into(),
            text_path: "b".into(),
            excerpt: "".into(),
            priority_a: 2,
            priority_b: 3,
            your_priority: "".into(),
            notes: "".into(),
        },
    ];
    let key = ReviewKey {
        blinding_seed: 1,
        run_id: "run".into(),
        rows: vec![
            ReviewKeyRow {
                review_row: 1,
                article_id: "a".into(),
                a_provider: "openai".into(),
                b_provider: "jev".into(),
            },
            ReviewKeyRow {
                review_row: 2,
                article_id: "b".into(),
                a_provider: "openai".into(),
                b_provider: "jev".into(),
            },
        ],
    };
    let manifest = manifest(vec![
        article("a", 1, &[], &[], PathKind::Sync),
        article("b", 2, &[], &[], PathKind::Sync),
    ]);
    let records = vec![record("a", Some(4), "ok", 1), record("b", Some(3), "ok", 1)];
    let csv = format!("\u{feff}{}", render_csv(&rows).replace('\n', "\r\n"));
    let report = score_review(&csv, &key, &manifest, &records, &RunConfig::default()).unwrap();
    assert_eq!(report.reviewed_rows, 1);
    assert_eq!(report.blank_rows, 1);

    let mut mismatched = key.clone();
    mismatched.rows[0].article_id = "wrong".into();
    let error = score_review(
        &csv,
        &mismatched,
        &manifest,
        &records,
        &RunConfig::default(),
    )
    .unwrap_err()
    .to_string();
    assert!(error.contains("row 1"), "{error}");

    for invalid in ["0", "7"] {
        let mut invalid_rows = rows.clone();
        invalid_rows[0].your_priority = invalid.into();
        let error = score_review(
            &render_csv(&invalid_rows),
            &key,
            &manifest,
            &records,
            &RunConfig::default(),
        )
        .unwrap_err()
        .to_string();
        assert!(error.contains("row 1"), "{error}");
        assert!(error.contains(invalid), "{error}");
    }

    let deleted = render_csv(&rows[..1]);
    let error = score_review(&deleted, &key, &manifest, &records, &RunConfig::default())
        .unwrap_err()
        .to_string();
    assert!(error.contains("leave unreviewed rows in place"), "{error}");

    let bad_header = render_csv(&rows).replacen("review_row", "wrong_header", 1);
    let error = score_review(
        &bad_header,
        &key,
        &manifest,
        &records,
        &RunConfig::default(),
    )
    .unwrap_err()
    .to_string();
    assert!(error.contains("header cell 1"), "{error}");
    assert!(error.contains("wrong_header"), "{error}");

    let mut wrong_run_key = key.clone();
    wrong_run_key.run_id = "another-run".into();
    let error = score_review(
        &render_csv(&rows),
        &wrong_run_key,
        &manifest,
        &records,
        &RunConfig::default(),
    )
    .unwrap_err()
    .to_string();
    assert!(error.contains("review key run mismatch"), "{error}");
}

#[test]
fn stopping_rule_and_threshold_precedence_cover_inconclusive_and_fail() {
    let rows = vec![
        ReviewCsvRow {
            review_row: 1,
            article_id: "a".into(),
            title: "".into(),
            text_path: "a".into(),
            excerpt: "".into(),
            priority_a: 1,
            priority_b: 5,
            your_priority: "".into(),
            notes: "".into(),
        },
        ReviewCsvRow {
            review_row: 2,
            article_id: "b".into(),
            title: "".into(),
            text_path: "b".into(),
            excerpt: "".into(),
            priority_a: 4,
            priority_b: 5,
            your_priority: "4".into(),
            notes: "".into(),
        },
    ];
    let key = ReviewKey {
        blinding_seed: 1,
        run_id: "run".into(),
        rows: vec![
            ReviewKeyRow {
                review_row: 1,
                article_id: "a".into(),
                a_provider: "openai".into(),
                b_provider: "jev".into(),
            },
            ReviewKeyRow {
                review_row: 2,
                article_id: "b".into(),
                a_provider: "openai".into(),
                b_provider: "jev".into(),
            },
        ],
    };
    let manifest = manifest(vec![
        article("a", 1, &[], &[], PathKind::Sync),
        article("b", 4, &[], &[], PathKind::Sync),
    ]);
    let records = vec![record("a", Some(5), "ok", 1), record("b", Some(5), "ok", 1)];
    let mut config = RunConfig::default();
    config.review.min_reviewed_rows = 1;
    config.review.min_high_priority_support = 1;
    config.review.large_gap = 2;
    config.acceptance = Some(AcceptanceConfig {
        high_priority_recall_min: 1.0,
        severe_miss_rate_max: 0.0,
        max_p95_latency_ms: 20,
        agreed_utc: "now".into(),
        agreed_note: "test".into(),
    });
    let condition_a = score_review(&render_csv(&rows), &key, &manifest, &records, &config).unwrap();
    assert_eq!(condition_a.overall, Evaluation::Inconclusive);
    assert!(condition_a
        .stopping_rule
        .missing
        .iter()
        .any(|value| value.contains("(a)")));

    let mut condition_b_rows = rows.clone();
    condition_b_rows[0].priority_b = 2;
    config.review.min_reviewed_rows = 2;
    let condition_b = score_review(
        &render_csv(&condition_b_rows),
        &key,
        &manifest,
        &records,
        &config,
    )
    .unwrap();
    assert!(condition_b
        .stopping_rule
        .missing
        .iter()
        .any(|value| value.contains("(b)")));

    let one_row_key = ReviewKey {
        rows: vec![key.rows[0].clone()],
        ..key.clone()
    };
    let mut one_row = rows[0].clone();
    one_row.your_priority = "4".into();
    config.review.min_reviewed_rows = 1;
    config.review.min_high_priority_support = 2;
    let condition_c = score_review(
        &render_csv(&[one_row.clone()]),
        &one_row_key,
        &manifest,
        &records,
        &config,
    )
    .unwrap();
    assert!(condition_c
        .stopping_rule
        .missing
        .iter()
        .any(|value| value.contains("(c)")));
    let rendered = render_markdown(&condition_c);
    assert!(rendered.contains("Overall: **inconclusive**"));
    assert!(rendered.contains("n = 1, rate not reported"));
    assert!(!rendered.contains("Some("));

    config.review.min_high_priority_support = 1;
    let passed = score_review(
        &render_csv(&[one_row.clone()]),
        &one_row_key,
        &manifest,
        &records,
        &config,
    )
    .unwrap();
    assert_eq!(passed.overall, Evaluation::Pass);

    one_row.priority_b = 2;
    config.acceptance.as_mut().unwrap().high_priority_recall_min = 0.0;
    let mut severe_records = records.clone();
    severe_records[0].priority = Some(2);
    for record in &mut severe_records {
        record.model_latency_ms = None;
    }
    let failed = score_review(
        &render_csv(&[one_row]),
        &one_row_key,
        &manifest,
        &severe_records,
        &config,
    )
    .unwrap();
    assert_eq!(failed.overall, Evaluation::Fail);
    assert_eq!(
        failed
            .thresholds
            .iter()
            .find(|threshold| threshold.name == "severe_miss_rate")
            .unwrap()
            .status,
        Evaluation::Fail
    );
    assert_eq!(
        failed
            .thresholds
            .iter()
            .find(|threshold| threshold.name == "max_p95_latency_ms")
            .unwrap()
            .status,
        Evaluation::Inconclusive
    );
}

#[test]
fn diagnosis_is_unblinded_and_covers_the_review_rows() {
    let manifest = manifest(vec![article("a", 1, &["x"], &[], PathKind::Sync)]);
    let mut result = record("a", Some(5), "ok", 1);
    result.category_probabilities = Some(BTreeMap::from([("Business".into(), 0.8)]));
    result.categories_selected = Some(vec!["Business".into()]);
    result.priority_probabilities = Some(BTreeMap::from([("1".into(), 0.0), ("5".into(), 1.0)]));
    let key = ReviewKey {
        blinding_seed: 1,
        run_id: "run".into(),
        rows: vec![ReviewKeyRow {
            review_row: 1,
            article_id: "a".into(),
            a_provider: "openai".into(),
            b_provider: "jev".into(),
        }],
    };
    let csv = render_csv(&[ReviewCsvRow {
        review_row: 1,
        article_id: "a".into(),
        title: "Title a".into(),
        text_path: "articles/a.txt".into(),
        excerpt: "text".into(),
        priority_a: 1,
        priority_b: 5,
        your_priority: "4".into(),
        notes: "note".into(),
    }]);
    let diagnosis = render_diagnosis(&csv, &key, &manifest, &[result]).unwrap();
    assert!(diagnosis.contains("OpenAI rationale: why a"));
    assert!(diagnosis.contains("Jev category probabilities"));
    assert!(diagnosis.contains("Review row 1"));
}

#[test]
fn diagnosis_rejects_a_resorted_review_csv_with_the_row_named() {
    let manifest = manifest(vec![
        article("a", 1, &[], &[], PathKind::Sync),
        article("c", 3, &[], &[], PathKind::Sync),
    ]);
    let key = ReviewKey {
        blinding_seed: 1,
        run_id: "run".into(),
        rows: vec![
            ReviewKeyRow {
                review_row: 1,
                article_id: "a".into(),
                a_provider: "openai".into(),
                b_provider: "jev".into(),
            },
            ReviewKeyRow {
                review_row: 2,
                article_id: "c".into(),
                a_provider: "jev".into(),
                b_provider: "openai".into(),
            },
        ],
    };
    let sorted = render_csv(&[
        ReviewCsvRow {
            review_row: 2,
            article_id: "c".into(),
            title: "".into(),
            text_path: "c".into(),
            excerpt: "".into(),
            priority_a: 4,
            priority_b: 3,
            your_priority: "".into(),
            notes: "".into(),
        },
        ReviewCsvRow {
            review_row: 1,
            article_id: "a".into(),
            title: "".into(),
            text_path: "a".into(),
            excerpt: "".into(),
            priority_a: 1,
            priority_b: 2,
            your_priority: "".into(),
            notes: "".into(),
        },
    ]);
    let error = render_diagnosis(
        &sorted,
        &key,
        &manifest,
        &[record("a", Some(2), "ok", 1), record("c", Some(4), "ok", 1)],
    )
    .unwrap_err()
    .to_string();
    assert!(error.contains("row 2"), "{error}");
    assert!(error.contains("expected review row 1 article a"), "{error}");
}

#[test]
fn fake_run_then_all_phase_four_cli_commands_write_the_expected_artifacts() {
    let fixture = cli_fixture();
    let config = fixture.config_path.to_str().unwrap();
    assert_cli_success(&run_cli(&fixture, &["run", "--config", config]));
    assert_cli_success(&run_cli(&fixture, &["report", "--run", "phase4-e2e"]));
    assert_cli_success(&run_cli(
        &fixture,
        &["review-file", "--run", "phase4-e2e", "--split", "dev"],
    ));
    let report_dir = fixture.experiment.join("reports/phase4-e2e");
    assert!(report_dir.join("metrics.md").is_file());
    assert!(report_dir.join("metrics.json").is_file());
    assert!(report_dir.join("review.csv").is_file());
    assert!(report_dir.join("review.key.json").is_file());
    assert!(!report_dir.join("review.failures.json").exists());
    let metrics: ComparisonReport =
        serde_json::from_slice(&fs::read(report_dir.join("metrics.json")).unwrap()).unwrap();
    assert_eq!(metrics.manifest_hash, fixture.original_manifest_hash);
    assert_eq!(metrics.metrics.successful_records, 1);
    assert_eq!(metrics.metrics.baseline_priority_histogram[&3], 1);
    assert_eq!(metrics.metrics.jev_priority_histogram[&4], 1);
    fill_review(&report_dir.join("review.csv"), "4");
    assert_cli_success(&run_cli(&fixture, &["score", "--run", "phase4-e2e"]));
    assert_cli_success(&run_cli(&fixture, &["diagnose", "--run", "phase4-e2e"]));
    assert!(fs::read_to_string(report_dir.join("scores.md"))
        .unwrap()
        .contains("biased reviewed subset"));
    let diagnosis = fs::read_to_string(report_dir.join("diagnosis.md")).unwrap();
    assert!(diagnosis.contains("OpenAI rationale"));
    assert!(diagnosis.contains("Jev category probabilities"));
    let log = fs::read_to_string(fixture.experiment.join("harvester_eval.log")).unwrap();
    for operation in ["report", "review-file", "score", "diagnose"] {
        assert!(log.contains(&format!("operation={operation} artifact=")));
    }
}

#[test]
fn every_phase_four_cli_command_rejects_a_refrozen_manifest_with_both_hashes() {
    let fixture = cli_fixture();
    let config = fixture.config_path.to_str().unwrap();
    assert_cli_success(&run_cli(&fixture, &["run", "--config", config]));
    assert_cli_success(&run_cli(
        &fixture,
        &["review-file", "--run", "phase4-e2e", "--split", "dev"],
    ));
    let review = fixture.experiment.join("reports/phase4-e2e/review.csv");
    fill_review(&review, "4");

    let mut changed: Manifest =
        serde_json::from_slice(&fs::read(&fixture.manifest_path).unwrap()).unwrap();
    changed.selection.ordering = "refrozen".into();
    changed.refresh_hash().unwrap();
    let changed_hash = changed.manifest_hash.clone();
    fs::write(
        &fixture.manifest_path,
        serde_json::to_vec_pretty(&changed).unwrap(),
    )
    .unwrap();

    for arguments in [
        vec!["report", "--run", "phase4-e2e"],
        vec!["review-file", "--run", "phase4-e2e", "--split", "dev"],
        vec!["score", "--run", "phase4-e2e"],
        vec!["diagnose", "--run", "phase4-e2e"],
    ] {
        let output = run_cli(&fixture, &arguments);
        assert!(
            !output.status.success(),
            "command unexpectedly succeeded: {arguments:?}"
        );
        let stderr = String::from_utf8_lossy(&output.stderr);
        assert!(stderr.contains(&fixture.original_manifest_hash), "{stderr}");
        assert!(stderr.contains(&changed_hash), "{stderr}");
    }
    let log = fs::read_to_string(fixture.experiment.join("harvester_eval.log")).unwrap();
    for operation in ["report", "review-file", "score", "diagnose"] {
        assert!(log.contains(&format!(
            "run_id=phase4-e2e operation={operation} status=failed"
        )));
    }
    assert!(!log.contains(&fixture.original_manifest_hash));
    assert!(!log.contains(&changed_hash));
}
