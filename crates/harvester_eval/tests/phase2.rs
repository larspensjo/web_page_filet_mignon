use std::collections::BTreeMap;
use std::fs;

use harvester_engine::llm::content_hash as sha256;
use harvester_eval::config::{
    config_hash, load_config, resolve_config, ConfigError, ConfigOverrides, ConfigWarning,
    RunConfig, Transport,
};
use harvester_eval::identity::PathKind;
use harvester_eval::jev::labels::select_above_threshold;
use harvester_eval::jev::request::{
    build_request, FrozenRubric, FrozenText, JevQuestion, RequestBuildError,
};
use harvester_eval::jev::response::{
    parse_jev_response, validate_answers, JevAnswer, JevAnswers, JevParseError, JevValidationError,
    Usage,
};
use harvester_eval::manifest::{ArticleEntry, Baseline, DatasetSplit, Usage as ManifestUsage};
use harvester_eval::metrics::ordinal::p_high;
use harvester_eval::pricing::jev_cost_microdollars;
use serde_json::{json, Value};
use tempfile::TempDir;

const RUBRIC: &str = "\
TRIAGE OBJECTIVE:\nSelect articles for review.\n\n\
PRIORITY SCALE:\n\
- 5: Highly material, specific, decision-relevant development.\n\
- 4: Strong business signal with meaningful implications.\n\
- 3: Potentially relevant signal that deserves review.\n\
- 2: Background or weakly actionable context.\n\
- 1: Generic hype or low-signal content.\n\n\
TAG GUIDANCE:\n- capex\n";

fn article(text: &str) -> ArticleEntry {
    ArticleEntry {
        article_id: "0123456789abcdef".into(),
        evidence_hash: sha256(text),
        source_content_hash: sha256(text),
        source_content_hashes: vec![sha256(text)],
        path_kind: PathKind::Sync,
        text_path: "articles/0123456789abcdef.txt".into(),
        text_bytes: text.len(),
        truncated: false,
        nonce: "0123456789ab".into(),
        nonce_verified: true,
        evidence_matches_source: true,
        identity_anomaly: None,
        split: DatasetSplit::Dev,
        duplicate_group: "group".into(),
        instruction_like: false,
        article_file: None,
        ambiguous_paths: Vec::new(),
        mapping_status: "unmapped".into(),
        baseline: Baseline {
            record_path: "fixture.json".into(),
            request_id: "fixture".into(),
            path_kind: PathKind::Sync,
            model_id: "fixture".into(),
            recorded_utc: "2026-01-01T00:00:00Z".into(),
            priority: 3,
            category: "Technology".into(),
            tags: Vec::new(),
            rationale: "fixture".into(),
            usage: ManifestUsage {
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

fn valid_response_value() -> Value {
    let mut answers = serde_json::Map::new();
    answers.insert(
        "priority".into(),
        json!({
            "choice": "4",
            "probabilities": {"1": 0.05, "2": 0.05, "3": 0.10, "4": 0.70, "5": 0.10},
            "confidence": 0.12
        }),
    );
    answers.insert(
        "relevance".into(),
        json!({
            "score": 4,
            "legend": ["none", "tangential", "partial", "strong", "direct"],
            "probabilities": {"0": 0.0, "1": 0.0, "2": 0.1, "3": 0.2, "4": 0.7}
        }),
    );
    for category in &RunConfig::default().questions.category_vocabulary {
        answers.insert(
            format!(
                "category__{}",
                harvester_eval::jev::request::sanitize_identifier(category)
            ),
            json!({"noul": 0.5}),
        );
    }
    for tag in &RunConfig::default().questions.tag_vocabulary {
        answers.insert(
            format!(
                "tag__{}",
                harvester_eval::jev::request::sanitize_identifier(tag)
            ),
            json!({"noul": 0.5}),
        );
    }
    json!({"model": "jev-latest", "answers": answers, "usage": {"input_tokens": 100, "output_tokens": 2}})
}

#[test]
fn request_identity_and_questions_are_primary_arm_only_and_stable() {
    let config = RunConfig::default();
    let text = "frozen evidence";
    let article = article(text);
    let rubric = FrozenRubric::verified(RUBRIC, sha256(RUBRIC)).unwrap();
    let frozen = FrozenText::for_article(&article, text).unwrap();
    let first = build_request(&article, &frozen, &rubric, &config).unwrap();
    let second = build_request(&article, &frozen, &rubric, &config).unwrap();
    assert_eq!(
        first.request_bytes().unwrap(),
        second.request_bytes().unwrap()
    );
    assert_eq!(first.state.editorial_criteria, RUBRIC);
    assert_eq!(first.state.article.text, text);
    let serialized = String::from_utf8(first.request_bytes().unwrap()).unwrap();
    assert!(!serialized.contains("\"title\""));
    assert!(!serialized.contains("\"url\""));
    assert!(!serialized.contains("\"published_at\""));
    assert!(!serialized.contains("<document-"));
    let JevQuestion::Choice { criteria, .. } = first.questions.get("priority").unwrap() else {
        panic!("priority is not choice")
    };
    assert_eq!(
        criteria.keys().cloned().collect::<Vec<_>>(),
        vec!["1", "2", "3", "4", "5"]
    );
    assert!(criteria["5"].contains("Highly material"));
    assert!(first
        .questions
        .contains_key("category__politics_regulation"));
    assert!(first.questions.contains_key("tag__stranded_capacity_risk"));
    assert_eq!(
        first
            .questions
            .keys()
            .filter(|key| key.starts_with("category__"))
            .count(),
        5
    );
    assert_eq!(
        first
            .questions
            .keys()
            .filter(|key| key.starts_with("tag__"))
            .count(),
        33
    );
    assert_eq!(
        config.questions.category_vocabulary,
        vec![
            "Business",
            "Technology",
            "Politics & Regulation",
            "Finance & Markets",
            "Science & Research"
        ]
    );
    assert!(!config
        .questions
        .category_vocabulary
        .iter()
        .any(|value| value == "Other"));
    let category_ids = first
        .questions
        .keys()
        .filter(|key| key.starts_with("category__"))
        .cloned()
        .collect::<Vec<_>>();
    assert!(category_ids.windows(2).all(|pair| pair[0] < pair[1]));
    let mut custom_relevance = config.clone();
    custom_relevance.questions.relevance_instructions =
        "Custom relevance instruction from the hashed config.".into();
    let custom = build_request(&article, &frozen, &rubric, &custom_relevance).unwrap();
    assert!(matches!(
        &custom.questions["relevance"],
        JevQuestion::Score { instructions, .. }
            if instructions == "Custom relevance instruction from the hashed config."
    ));
    let mut no_relevance = config.clone();
    no_relevance.questions.include_relevance = false;
    let without = build_request(&article, &frozen, &rubric, &no_relevance).unwrap();
    assert!(!without.questions.contains_key("relevance"));
    assert_eq!(without.questions.len(), first.questions.len() - 1);
}

#[test]
fn checked_in_triage_rubric_still_parses_the_priority_scale() {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .join("contexts/article_triage.toml");
    let context: toml::Value = toml::from_str(&fs::read_to_string(path).unwrap()).unwrap();
    let rubric = context["variables"]["triage_instructions"]
        .as_str()
        .unwrap();
    let request = build_request(
        &article("evidence"),
        &FrozenText::new("evidence"),
        &FrozenRubric::new(rubric, sha256(rubric)),
        &RunConfig::default(),
    )
    .unwrap();
    let JevQuestion::Choice { criteria, .. } = &request.questions["priority"] else {
        panic!("priority is not choice")
    };
    assert_eq!(
        criteria.keys().cloned().collect::<Vec<_>>(),
        ["1", "2", "3", "4", "5"]
    );
}

#[test]
fn rubric_hash_and_placeholder_are_refused() {
    let config = RunConfig::default();
    let article = article("evidence");
    let text = FrozenText::new("evidence");
    let bad_rubric = FrozenRubric::new(RUBRIC, "wrong");
    assert!(matches!(
        build_request(&article, &text, &bad_rubric, &config),
        Err(RequestBuildError::RubricHashMismatch { .. })
    ));
    let mut placeholder = config.clone();
    placeholder.questions.tag_instructions = "REPLACE_WITH_TAGS".into();
    assert!(matches!(
        placeholder.validate(),
        Err(ConfigError::Placeholder {
            key: "questions.tag_instructions"
        })
    ));
}

#[test]
fn documented_response_shapes_parse_and_validate() {
    let value = valid_response_value();
    let parsed = parse_jev_response(&serde_json::to_vec(&value).unwrap()).unwrap();
    assert!(matches!(
        parsed.answers["priority"],
        JevAnswer::Choice { .. }
    ));
    assert!(matches!(
        parsed.answers["relevance"],
        JevAnswer::Score { .. }
    ));
    assert_eq!(parsed.usage.input_tokens, 100);
    let validated = validate_answers(&parsed, &RunConfig::default()).unwrap();
    assert_eq!(validated.priority, 4);
    assert_eq!(validated.relevance, Some(4.0));
    assert_eq!(validated.selected_categories.len(), 5);
    assert_eq!(
        validated.selected_tags.len(),
        RunConfig::default().questions.tag_vocabulary.len()
    );

    let score_without_probabilities = json!({
        "model": "jev-latest",
        "answers": {"priority": {
            "choice": "1",
            "probabilities": {"1": 1.0, "2": 0.0, "3": 0.0, "4": 0.0, "5": 0.0},
            "confidence": 1.0
        }, "relevance": {"score": 0, "legend": ["none", "some"]}},
        "usage": {"input_tokens": 1, "output_tokens": 1}
    });
    let parsed =
        parse_jev_response(&serde_json::to_vec(&score_without_probabilities).unwrap()).unwrap();
    assert!(matches!(
        parsed.answers["relevance"],
        JevAnswer::Score {
            probabilities: None,
            ..
        }
    ));
    let noul = json!({"model": "jev-latest", "answers": {"priority": {
        "choice": "1", "probabilities": {"1": 1.0, "2": 0.0, "3": 0.0, "4": 0.0, "5": 0.0}
    }, "relevance": {"score": 0, "legend": ["none"]}, "category__x": {"noul": 0.1}},
    "usage": {"input_tokens": 1, "output_tokens": 1}});
    let parsed = parse_jev_response(&serde_json::to_vec(&noul).unwrap()).unwrap();
    assert!(matches!(
        parsed.answers["category__x"],
        JevAnswer::Noul { .. }
    ));
}

fn priority_answers() -> JevAnswers {
    JevAnswers {
        model: "jev-latest".into(),
        answers: BTreeMap::from([
            (
                "priority".into(),
                JevAnswer::Choice {
                    choice: "4".into(),
                    probabilities: Some(BTreeMap::from([
                        ("1".into(), 0.1),
                        ("2".into(), 0.1),
                        ("3".into(), 0.1),
                        ("4".into(), 0.5),
                        ("5".into(), 0.2),
                    ])),
                    confidence: Some(0.01),
                },
            ),
            (
                "relevance".into(),
                JevAnswer::Score {
                    score: 0.0,
                    legend: BTreeMap::from([
                        ("0".into(), "none".into()),
                        ("1".into(), "tangential".into()),
                        ("2".into(), "partial".into()),
                        ("3".into(), "strong".into()),
                        ("4".into(), "direct".into()),
                    ]),
                    probabilities: None,
                },
            ),
        ]),
        usage: Usage::default(),
    }
}

#[test]
fn malformed_responses_use_specific_validation_failures() {
    assert!(matches!(
        parse_jev_response(b"not json"),
        Err(JevParseError::InvalidJson(_))
    ));
    assert_eq!(
        parse_jev_response(br#"{"model":"jev-latest"}"#).unwrap_err(),
        JevParseError::MissingAnswers
    );
    let mut config = RunConfig::default();
    config.questions.include_categories = false;
    config.questions.include_tags = false;
    for invalid_choice in ["6", "P1", "high"] {
        let mut answers = priority_answers();
        if let JevAnswer::Choice { choice, .. } = answers.answers.get_mut("priority").unwrap() {
            *choice = invalid_choice.into();
        }
        assert!(matches!(
            validate_answers(&answers, &config),
            Err(JevValidationError::ChoiceLabelOutsideOptions { .. })
        ));
    }
    let mut answers = priority_answers();
    if let JevAnswer::Choice { probabilities, .. } = answers.answers.get_mut("priority").unwrap() {
        probabilities.as_mut().unwrap().insert("1".into(), -0.1);
    }
    assert!(matches!(
        validate_answers(&answers, &config),
        Err(JevValidationError::ProbabilityOutsideUnitInterval { .. })
    ));
    let mut answers = priority_answers();
    if let JevAnswer::Choice { probabilities, .. } = answers.answers.get_mut("priority").unwrap() {
        probabilities.as_mut().unwrap().insert("6".into(), 0.0);
    }
    assert!(matches!(
        validate_answers(&answers, &config),
        Err(JevValidationError::DistributionKeysMismatch { .. })
    ));
    let mut answers = priority_answers();
    if let JevAnswer::Choice { probabilities, .. } = answers.answers.get_mut("priority").unwrap() {
        probabilities.as_mut().unwrap().remove("3");
    }
    assert!(matches!(
        validate_answers(&answers, &config),
        Err(JevValidationError::DistributionKeysMismatch { .. })
    ));
    let mut answers = priority_answers();
    if let JevAnswer::Choice { probabilities, .. } = answers.answers.get_mut("priority").unwrap() {
        probabilities.as_mut().unwrap().insert("1".into(), f64::NAN);
    }
    assert!(matches!(
        validate_answers(&answers, &config),
        Err(JevValidationError::NonFiniteProbability {
            ref question,
            ref option
        }) if question == "priority" && option == "1"
    ));
    let mut answers = priority_answers();
    if let JevAnswer::Choice { probabilities, .. } = answers.answers.get_mut("priority").unwrap() {
        probabilities.as_mut().unwrap().insert("1".into(), 1.2);
    }
    assert!(matches!(
        validate_answers(&answers, &config),
        Err(JevValidationError::ProbabilityOutsideUnitInterval {
            ref question,
            ref option,
            ..
        }) if question == "priority" && option == "1"
    ));
    let mut answers = priority_answers();
    if let JevAnswer::Choice { probabilities, .. } = answers.answers.get_mut("priority").unwrap() {
        probabilities.as_mut().unwrap().insert("1".into(), 0.0);
        probabilities.as_mut().unwrap().insert("2".into(), 0.0);
        probabilities.as_mut().unwrap().insert("3".into(), 0.0);
        probabilities.as_mut().unwrap().insert("4".into(), 0.4);
        probabilities.as_mut().unwrap().insert("5".into(), 0.4);
    }
    assert!(matches!(
        validate_answers(&answers, &config),
        Err(JevValidationError::ProbabilityMassOutOfTolerance {
            ref question,
            ..
        }) if question == "priority"
    ));
    let mut answers = priority_answers();
    answers
        .answers
        .insert("category__x".into(), JevAnswer::Noul { probability: 1.4 });
    let mut noul_config = config.clone();
    noul_config.questions.include_categories = true;
    noul_config.questions.category_vocabulary = vec!["x".into()];
    assert!(matches!(
        validate_answers(&answers, &noul_config),
        Err(JevValidationError::NoulProbabilityOutOfRange { .. })
    ));
    let mut answers = priority_answers();
    answers
        .answers
        .insert("surprise".into(), JevAnswer::Noul { probability: 0.2 });
    assert!(matches!(
        validate_answers(&answers, &config),
        Err(JevValidationError::UnknownAnswerKey { .. })
    ));
    let mut answers = priority_answers();
    if let JevAnswer::Score { score, .. } = answers.answers.get_mut("relevance").unwrap() {
        *score = 5.0;
    }
    assert!(matches!(
        validate_answers(&answers, &config),
        Err(JevValidationError::ScoreOutsideLegend { .. })
    ));
    let mut answers = priority_answers();
    if let JevAnswer::Score {
        legend,
        probabilities,
        ..
    } = answers.answers.get_mut("relevance").unwrap()
    {
        *legend = BTreeMap::from([
            ("0".into(), "none".into()),
            ("1".into(), "some".into()),
            ("2".into(), "partial".into()),
            ("3".into(), "strong".into()),
            ("4".into(), "direct".into()),
        ]);
        *probabilities = Some(BTreeMap::from([("0".into(), 1.0)]));
    }
    assert!(matches!(
        validate_answers(&answers, &config),
        Err(JevValidationError::DistributionKeysMismatch { .. })
    ));
    let mut answers = priority_answers();
    answers.answers.remove("priority");
    assert!(matches!(
        validate_answers(&answers, &config),
        Err(JevValidationError::MissingAnswer { .. })
    ));
}

#[test]
fn response_envelope_requires_typed_model_and_complete_usage() {
    let complete = json!({
        "model": "jev-latest",
        "answers": {},
        "usage": {"input_tokens": 1, "output_tokens": 2}
    });
    let mut value = complete.clone();
    value.as_object_mut().unwrap().remove("model");
    assert_eq!(
        parse_jev_response(&serde_json::to_vec(&value).unwrap()).unwrap_err(),
        JevParseError::MissingModel
    );
    let mut value = complete.clone();
    value["model"] = json!(3);
    assert_eq!(
        parse_jev_response(&serde_json::to_vec(&value).unwrap()).unwrap_err(),
        JevParseError::InvalidModel
    );
    let mut value = complete.clone();
    value.as_object_mut().unwrap().remove("usage");
    assert_eq!(
        parse_jev_response(&serde_json::to_vec(&value).unwrap()).unwrap_err(),
        JevParseError::MissingUsage
    );
    let mut value = complete.clone();
    value["usage"]
        .as_object_mut()
        .unwrap()
        .remove("output_tokens");
    assert_eq!(
        parse_jev_response(&serde_json::to_vec(&value).unwrap()).unwrap_err(),
        JevParseError::MissingUsageField {
            field: "output_tokens"
        }
    );
    let mut value = complete;
    value["usage"]["output_tokens"] = json!("2");
    assert_eq!(
        parse_jev_response(&serde_json::to_vec(&value).unwrap()).unwrap_err(),
        JevParseError::InvalidUsageField {
            field: "output_tokens"
        }
    );
}

#[test]
fn returned_score_legend_indices_are_preserved_and_validated() {
    let mut value = valid_response_value();
    value["answers"]["relevance"] = json!({
        "score": 5,
        "legend": {
            "1": "none", "2": "tangential", "3": "partial", "4": "strong", "5": "direct"
        },
        "probabilities": {"1": 0.0, "2": 0.0, "3": 0.1, "4": 0.2, "5": 0.7}
    });
    let parsed = parse_jev_response(&serde_json::to_vec(&value).unwrap()).unwrap();
    let JevAnswer::Score { legend, .. } = &parsed.answers["relevance"] else {
        panic!("relevance is not a score")
    };
    assert_eq!(
        legend.keys().cloned().collect::<Vec<_>>(),
        ["1", "2", "3", "4", "5"]
    );
    assert_eq!(
        validate_answers(&parsed, &RunConfig::default())
            .unwrap()
            .relevance,
        Some(5.0)
    );

    let mut config = RunConfig::default();
    config.questions.include_categories = false;
    config.questions.include_tags = false;
    let mut answers = priority_answers();
    if let JevAnswer::Score { legend, .. } = answers.answers.get_mut("relevance").unwrap() {
        legend.remove("4");
    }
    assert!(matches!(
        validate_answers(&answers, &config),
        Err(JevValidationError::LegendLengthMismatch { .. })
    ));

    let mut answers = priority_answers();
    if let JevAnswer::Score { legend, .. } = answers.answers.get_mut("relevance").unwrap() {
        legend.clear();
    }
    assert!(matches!(
        validate_answers(&answers, &config),
        Err(JevValidationError::EmptyLegend { .. })
    ));

    let mut answers = priority_answers();
    if let JevAnswer::Score { legend, .. } = answers.answers.get_mut("relevance").unwrap() {
        *legend = BTreeMap::from([
            ("0".into(), "none".into()),
            ("2".into(), "tangential".into()),
            ("3".into(), "partial".into()),
            ("4".into(), "strong".into()),
            ("7".into(), "direct".into()),
        ]);
    }
    assert!(matches!(
        validate_answers(&answers, &config),
        Err(JevValidationError::LegendKeysInvalid { .. })
    ));
}

#[test]
fn answer_kind_mismatches_have_a_dedicated_error() {
    let mut config = RunConfig::default();
    config.questions.include_categories = false;
    config.questions.include_tags = false;

    let mut answers = priority_answers();
    answers
        .answers
        .insert("priority".into(), JevAnswer::Noul { probability: 0.5 });
    assert!(matches!(
        validate_answers(&answers, &config),
        Err(JevValidationError::AnswerTypeMismatch {
            ref key,
            expected: "choice"
        }) if key == "priority"
    ));

    let mut answers = priority_answers();
    answers.answers.insert(
        "relevance".into(),
        JevAnswer::Choice {
            choice: "1".into(),
            probabilities: None,
            confidence: None,
        },
    );
    assert!(matches!(
        validate_answers(&answers, &config),
        Err(JevValidationError::AnswerTypeMismatch {
            ref key,
            expected: "score"
        }) if key == "relevance"
    ));

    config.questions.include_categories = true;
    config.questions.category_vocabulary = vec!["x".into()];
    let mut answers = priority_answers();
    answers.answers.insert(
        "category__x".into(),
        JevAnswer::Choice {
            choice: "1".into(),
            probabilities: None,
            confidence: None,
        },
    );
    assert!(matches!(
        validate_answers(&answers, &config),
        Err(JevValidationError::AnswerTypeMismatch {
            ref key,
            expected: "noul"
        }) if key == "category__x"
    ));
}

#[test]
fn p_high_uses_validated_distribution_and_invalid_distributions_do_not_validate() {
    let mut answers = priority_answers();
    if let JevAnswer::Choice { confidence, .. } = answers.answers.get_mut("priority").unwrap() {
        *confidence = Some(0.99);
    }
    let config = {
        let mut value = RunConfig::default();
        value.questions.include_categories = false;
        value.questions.include_tags = false;
        value
    };
    let validated = validate_answers(&answers, &config).unwrap();
    assert_eq!(validated.p_high, 0.7);
    assert_eq!(p_high(&validated.priority_distribution), 0.7);
    if let JevAnswer::Choice { probabilities, .. } = answers.answers.get_mut("priority").unwrap() {
        probabilities.as_mut().unwrap().insert("5".into(), 1.2);
    }
    assert!(validate_answers(&answers, &config).is_err());
}

#[test]
fn thresholding_is_inclusive_sorted_and_empty_is_valid() {
    let values = BTreeMap::from([("z".into(), 0.7), ("a".into(), 0.5), ("b".into(), 0.49)]);
    assert_eq!(select_above_threshold(&values, 0.5), vec!["a", "z"]);
    assert!(select_above_threshold(&values, 0.9).is_empty());

    let mut config = RunConfig::default();
    config.questions.include_tags = false;
    let mut answers = priority_answers();
    for category in &config.questions.category_vocabulary {
        answers.answers.insert(
            format!(
                "category__{}",
                harvester_eval::jev::request::sanitize_identifier(category)
            ),
            JevAnswer::Noul { probability: 0.49 },
        );
    }
    assert!(validate_answers(&answers, &config)
        .unwrap()
        .selected_categories
        .is_empty());
}

#[test]
fn pricing_units_and_config_hash_and_precedence_are_stable() {
    assert_eq!(jev_cost_microdollars(1_000_000, 42_000), 42_000);
    assert_eq!(jev_cost_microdollars(250_000, 42_000), 10_500);
    assert_eq!(jev_cost_microdollars(1, 42_000), 1);
    let mut low = RunConfig::default();
    low.pricing.input_microdollars_per_million = 42;
    assert!(matches!(
        low.validate(),
        Err(ConfigError::PricingRateTooLow(42))
    ));

    let temp = TempDir::new().unwrap();
    let first_path = temp.path().join("first.toml");
    let second_path = temp.path().join("second.toml");
    fs::write(
        &first_path,
        "[meta]\nname='x'\nnotes='y'\n[run]\ntransport='live'\nconcurrency=1\n",
    )
    .unwrap();
    fs::write(
        &second_path,
        "[run]\nconcurrency=1\ntransport='live'\n[meta]\nnotes='y'\nname='x'\n",
    )
    .unwrap();
    assert_eq!(
        config_hash(&load_config(&first_path).unwrap()),
        config_hash(&load_config(&second_path).unwrap())
    );
    let base = load_config(&first_path).unwrap();
    let mut changed = base.clone();
    changed.questions.tag_probability_threshold = 0.6;
    assert_ne!(config_hash(&changed), config_hash(&base));
    let mut changed = base.clone();
    changed.questions.tag_instructions.push_str(" changed");
    assert_ne!(config_hash(&changed), config_hash(&base));
    let mut changed = base.clone();
    changed
        .questions
        .relevance_instructions
        .push_str(" changed");
    assert_ne!(config_hash(&changed), config_hash(&base));
    let mut changed = base.clone();
    changed.pricing.input_microdollars_per_million += 1;
    assert_ne!(config_hash(&changed), config_hash(&base));

    let config = load_config(&first_path).unwrap();
    assert_eq!(
        config.warnings(),
        vec![ConfigWarning::LiveTransportWithoutRunId]
    );
    let overrides = ConfigOverrides {
        transport: Some(Transport::Fake),
        fake_dir: Some("fake".into()),
        ..ConfigOverrides::default()
    };
    let resolved = resolve_config(Some(&config), &overrides).unwrap();
    assert_eq!(resolved.run.transport, Transport::Fake);
}

#[test]
fn configuration_rejects_other_invalid_values_and_question_identifier_collisions() {
    let mut config = RunConfig::default();
    config.questions.category_vocabulary = vec!["".into()];
    assert!(matches!(
        config.validate(),
        Err(ConfigError::EmptyVocabulary {
            key: "questions.category_vocabulary"
        })
    ));
    config.questions.category_vocabulary = vec!["A-B".into(), "A B".into()];
    assert!(matches!(
        config.validate(),
        Err(ConfigError::IdentifierCollision {
            key: "questions.category_vocabulary",
            ..
        })
    ));
    assert!(matches!(
        validate_answers(&priority_answers(), &config),
        Err(JevValidationError::InvalidConfig(
            ConfigError::IdentifierCollision { .. }
        ))
    ));
    config.questions.category_vocabulary = vec!["!!!".into()];
    assert!(matches!(
        config.validate(),
        Err(ConfigError::EmptyIdentifier {
            key: "questions.category_vocabulary",
            ..
        })
    ));
    config.questions.category_vocabulary = vec!["Business".into()];
    config.jev.max_attempts = 0;
    assert!(matches!(
        config.validate(),
        Err(ConfigError::MaxAttemptsZero)
    ));
    config.jev.max_attempts = 1;
    config.run.transport = Transport::Fake;
    config.run.fake_dir = None;
    assert!(matches!(
        config.validate(),
        Err(ConfigError::FakeDirectoryMissing)
    ));
    config.run.fake_dir = Some("fake".into());
    config.questions.category_probability_threshold = 2.0;
    assert!(matches!(
        config.validate(),
        Err(ConfigError::ThresholdOutOfRange {
            key: "questions.category_probability_threshold"
        })
    ));

    let mut disabled = RunConfig::default();
    disabled.questions.include_relevance = false;
    disabled.questions.include_categories = false;
    disabled.questions.include_tags = false;
    disabled.questions.relevance_levels.clear();
    disabled.questions.category_vocabulary.clear();
    disabled.questions.tag_vocabulary.clear();
    assert_eq!(disabled.validate(), Ok(()));

    let mut placeholder = RunConfig::default();
    placeholder.questions.relevance_levels[0] = "REPLACE_LEVEL".into();
    assert!(matches!(
        placeholder.validate(),
        Err(ConfigError::Placeholder {
            key: "questions.relevance_levels"
        })
    ));
}

#[test]
fn config_loader_rejects_unknown_keys_and_invalid_enums() {
    let temp = TempDir::new().unwrap();
    for (name, text) in [
        ("typo.toml", "[questions]\ntag_probabilty_threshold = 0.8\n"),
        ("split.toml", "[dataset]\nsplit = 'training'\n"),
        ("transport.toml", "[run]\ntransport = 'network'\n"),
    ] {
        let path = temp.path().join(name);
        fs::write(&path, text).unwrap();
        assert!(matches!(load_config(&path), Err(ConfigError::Parse { .. })));
    }
}

#[test]
fn secret_value_is_not_read_or_rendered() {
    let secret = "test-only-typesafe-secret";
    unsafe { std::env::set_var("TYPESAFE_AI_API_KEY", secret) };
    let config = RunConfig::default();
    let article = article("safe evidence");
    let request = build_request(
        &article,
        &FrozenText::new("safe evidence"),
        &FrozenRubric::new(RUBRIC, sha256(RUBRIC)),
        &config,
    )
    .unwrap();
    // Request and config contracts intentionally expose Debug, not Display.
    let mut rendered = format!("{request:?}{config:?}");
    let request_error = RequestBuildError::EvidenceHashMismatch {
        article_id: "safe-article-id".into(),
    };
    let config_error = ConfigError::InvalidValue {
        key: "dataset.repeat",
        value: "safe-invalid-value".into(),
    };
    let parse_error = JevParseError::InvalidJson("safe-parser-detail".into());
    let validation_error = JevValidationError::AnswerTypeMismatch {
        key: "safe-question".into(),
        expected: "choice",
    };
    rendered.push_str(&format!(
        "{request_error:?}{request_error}{config_error:?}{config_error}{parse_error:?}{parse_error}{validation_error:?}{validation_error}"
    ));
    assert!(!rendered.contains(secret));
    unsafe { std::env::remove_var("TYPESAFE_AI_API_KEY") };
}
