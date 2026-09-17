//! Pure parsing and strict validation of Jev response envelopes.

use std::collections::{BTreeMap, BTreeSet};

use serde_json::Value;
use thiserror::Error;

use crate::config::{ConfigError, RunConfig};
use crate::jev::labels::select_above_threshold;
use crate::jev::question_id::question_id;
use crate::metrics::ordinal::{p_high, Priority, ValidatedPriorityDistribution};

#[derive(Debug, Clone, PartialEq)]
pub struct JevAnswers {
    pub model: String,
    pub answers: BTreeMap<String, JevAnswer>,
    pub usage: Usage,
}

#[derive(Debug, Clone, PartialEq)]
pub enum JevAnswer {
    Choice {
        choice: String,
        probabilities: Option<BTreeMap<String, f64>>,
        confidence: Option<f64>,
    },
    Score {
        score: f64,
        legend: BTreeMap<String, String>,
        probabilities: Option<BTreeMap<String, f64>>,
    },
    Noul {
        probability: f64,
    },
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Usage {
    pub input_tokens: u64,
    pub output_tokens: u64,
}

#[derive(Debug, Error, PartialEq)]
pub enum JevParseError {
    #[error("invalid Jev JSON: {0}")]
    InvalidJson(String),
    #[error("Jev response root must be an object")]
    RootNotObject,
    #[error("Jev response is missing answers")]
    MissingAnswers,
    #[error("Jev response answers must be an object")]
    AnswersNotObject,
    #[error("Jev response is missing model")]
    MissingModel,
    #[error("Jev response model must be a string")]
    InvalidModel,
    #[error("answer {key:?} has an invalid shape: expected {expected}")]
    InvalidAnswerShape { key: String, expected: &'static str },
    #[error("answer {key:?} is missing field {field:?}")]
    MissingAnswerField { key: String, field: &'static str },
    #[error("answer {key:?} field {field:?} has an invalid type")]
    InvalidAnswerFieldType { key: String, field: &'static str },
    #[error("Jev response is missing usage")]
    MissingUsage,
    #[error("response usage has an invalid shape")]
    InvalidUsage,
    #[error("response usage is missing field {field:?}")]
    MissingUsageField { field: &'static str },
    #[error("response usage field {field:?} has an invalid type")]
    InvalidUsageField { field: &'static str },
}

#[derive(Debug, Error, PartialEq)]
pub enum JevValidationError {
    #[error("invalid run configuration: {0}")]
    InvalidConfig(#[from] ConfigError),
    #[error("unknown answer key {key:?}")]
    UnknownAnswerKey { key: String },
    #[error("missing answer {key:?}")]
    MissingAnswer { key: String },
    #[error("choice label {label:?} for {key:?} is outside the requested options")]
    ChoiceLabelOutsideOptions { key: String, label: String },
    #[error("distribution keys for {key:?} do not match: missing={missing:?}, extra={extra:?}")]
    DistributionKeysMismatch {
        key: String,
        missing: Vec<String>,
        extra: Vec<String>,
    },
    #[error("probability for option {option:?} in {question:?} is not finite")]
    NonFiniteProbability { question: String, option: String },
    #[error("probability for option {option:?} in {question:?} is outside [0, 1]: {value}")]
    ProbabilityOutsideUnitInterval {
        question: String,
        option: String,
        value: f64,
    },
    #[error("probability mass for {question:?} is outside tolerance: {sum}")]
    ProbabilityMassOutOfTolerance { question: String, sum: f64 },
    #[error("noul probability for {key:?} is outside [0, 1]: {value}")]
    NoulProbabilityOutOfRange { key: String, value: f64 },
    #[error("score {score} for {key:?} is outside its returned legend")]
    ScoreOutsideLegend {
        key: String,
        score: f64,
        legend_len: usize,
    },
    #[error("legend for {key:?} must not be empty")]
    EmptyLegend { key: String },
    #[error("legend for {key:?} must use contiguous integer keys starting at 0 or 1")]
    LegendKeysInvalid { key: String },
    #[error("legend for {key:?} has {actual} entries, expected {expected}")]
    LegendLengthMismatch {
        key: String,
        expected: usize,
        actual: usize,
    },
    #[error("answer {key:?} has type mismatch: expected {expected}")]
    AnswerTypeMismatch { key: String, expected: &'static str },
    #[error("choice answer {0:?} has no probability distribution")]
    ChoiceDistributionMissing(String),
}

#[derive(Debug, Clone, PartialEq)]
pub struct ValidatedAnswers {
    pub model: String,
    pub usage: Usage,
    pub priority: Priority,
    pub priority_distribution: ValidatedPriorityDistribution,
    pub confidence: Option<f64>,
    pub p_high: f64,
    /// The score index returned by Jev. It uses the returned legend's 0- or
    /// 1-based index and is deliberately not rounded or converted into priority.
    pub relevance: Option<f64>,
    pub relevance_legend: Option<BTreeMap<String, String>>,
    pub category_probabilities: BTreeMap<String, f64>,
    pub tag_probabilities: BTreeMap<String, f64>,
    pub selected_categories: Vec<String>,
    pub selected_tags: Vec<String>,
}

/// Parses documented response JSON without applying any semantic coercion.
pub fn parse_jev_response(bytes: &[u8]) -> Result<JevAnswers, JevParseError> {
    let value: Value = serde_json::from_slice(bytes)
        .map_err(|error| JevParseError::InvalidJson(error.to_string()))?;
    let root = value.as_object().ok_or(JevParseError::RootNotObject)?;
    let answers_value = root.get("answers").ok_or(JevParseError::MissingAnswers)?;
    let answers_object = answers_value
        .as_object()
        .ok_or(JevParseError::AnswersNotObject)?;
    let mut answers = BTreeMap::new();
    for (key, answer) in answers_object {
        answers.insert(key.clone(), parse_answer(key, answer)?);
    }
    let model = root
        .get("model")
        .ok_or(JevParseError::MissingModel)?
        .as_str()
        .ok_or(JevParseError::InvalidModel)?
        .to_string();
    let usage = parse_usage(root.get("usage"))?;
    Ok(JevAnswers {
        model,
        answers,
        usage,
    })
}

fn parse_answer(key: &str, value: &Value) -> Result<JevAnswer, JevParseError> {
    let object = value
        .as_object()
        .ok_or_else(|| JevParseError::InvalidAnswerShape {
            key: key.into(),
            expected: "an object",
        })?;
    if let Some(choice) = object.get("choice") {
        let choice = choice
            .as_str()
            .ok_or_else(|| JevParseError::InvalidAnswerFieldType {
                key: key.into(),
                field: "choice",
            })?
            .to_string();
        return Ok(JevAnswer::Choice {
            choice,
            probabilities: object
                .get("probabilities")
                .map(parse_probabilities)
                .transpose()?,
            confidence: object
                .get("confidence")
                .map(|value| {
                    value
                        .as_f64()
                        .ok_or_else(|| JevParseError::InvalidAnswerFieldType {
                            key: key.into(),
                            field: "confidence",
                        })
                })
                .transpose()?,
        });
    }
    if let Some(score) = object.get("score") {
        let score = score
            .as_f64()
            .ok_or_else(|| JevParseError::InvalidAnswerFieldType {
                key: key.into(),
                field: "score",
            })?;
        let legend_value =
            object
                .get("legend")
                .ok_or_else(|| JevParseError::MissingAnswerField {
                    key: key.into(),
                    field: "legend",
                })?;
        let legend = if let Some(array) = legend_value.as_array() {
            array
                .iter()
                .enumerate()
                .map(|(index, item)| {
                    item.as_str()
                        .map(|value| (index.to_string(), value.into()))
                        .ok_or_else(|| JevParseError::InvalidAnswerFieldType {
                            key: key.into(),
                            field: "legend",
                        })
                })
                .collect::<Result<BTreeMap<_, _>, _>>()?
        } else if let Some(object) = legend_value.as_object() {
            object
                .iter()
                .map(|(index, value)| {
                    index
                        .parse::<usize>()
                        .map_err(|_| JevParseError::InvalidAnswerFieldType {
                            key: key.into(),
                            field: "legend",
                        })?;
                    let value =
                        value
                            .as_str()
                            .ok_or_else(|| JevParseError::InvalidAnswerFieldType {
                                key: key.into(),
                                field: "legend",
                            })?;
                    Ok((index.clone(), value.to_string()))
                })
                .collect::<Result<BTreeMap<_, _>, JevParseError>>()?
        } else {
            return Err(JevParseError::InvalidAnswerFieldType {
                key: key.into(),
                field: "legend",
            });
        };
        return Ok(JevAnswer::Score {
            score,
            legend,
            probabilities: object
                .get("probabilities")
                .map(parse_probabilities)
                .transpose()?,
        });
    }
    if let Some(noul) = object.get("noul") {
        let probability = noul
            .as_f64()
            .ok_or_else(|| JevParseError::InvalidAnswerFieldType {
                key: key.into(),
                field: "noul",
            })?;
        return Ok(JevAnswer::Noul { probability });
    }
    Err(JevParseError::InvalidAnswerShape {
        key: key.into(),
        expected: "choice, score, or noul",
    })
}

fn parse_probabilities(value: &Value) -> Result<BTreeMap<String, f64>, JevParseError> {
    if let Some(object) = value.as_object() {
        object
            .iter()
            .map(|(key, value)| {
                value
                    .as_f64()
                    .map(|number| (key.clone(), number))
                    .ok_or_else(|| JevParseError::InvalidAnswerFieldType {
                        key: key.clone(),
                        field: "probabilities",
                    })
            })
            .collect()
    } else if let Some(array) = value.as_array() {
        array
            .iter()
            .enumerate()
            .map(|(index, value)| {
                value
                    .as_f64()
                    .map(|number| (index.to_string(), number))
                    .ok_or_else(|| JevParseError::InvalidAnswerFieldType {
                        key: index.to_string(),
                        field: "probabilities",
                    })
            })
            .collect()
    } else {
        Err(JevParseError::InvalidAnswerFieldType {
            key: String::new(),
            field: "probabilities",
        })
    }
}

fn parse_usage(value: Option<&Value>) -> Result<Usage, JevParseError> {
    let value = value.ok_or(JevParseError::MissingUsage)?;
    let object = value.as_object().ok_or(JevParseError::InvalidUsage)?;
    let input_value = object
        .get("input_tokens")
        .or_else(|| object.get("inputTokens"))
        .ok_or(JevParseError::MissingUsageField {
            field: "input_tokens",
        })?;
    let input_tokens = input_value
        .as_u64()
        .ok_or(JevParseError::InvalidUsageField {
            field: "input_tokens",
        })?;
    let output_value = object
        .get("output_tokens")
        .or_else(|| object.get("outputTokens"))
        .ok_or(JevParseError::MissingUsageField {
            field: "output_tokens",
        })?;
    let output_tokens = output_value
        .as_u64()
        .ok_or(JevParseError::InvalidUsageField {
            field: "output_tokens",
        })?;
    Ok(Usage {
        input_tokens,
        output_tokens,
    })
}

/// Validates every answer against the request implied by the run config.
pub fn validate_answers(
    answers: &JevAnswers,
    config: &RunConfig,
) -> Result<ValidatedAnswers, JevValidationError> {
    config.validate()?;
    let expected = expected_keys(config);
    for key in answers.answers.keys() {
        if !expected.contains(key) {
            return Err(JevValidationError::UnknownAnswerKey { key: key.clone() });
        }
    }
    for key in &expected {
        if !answers.answers.contains_key(key) {
            return Err(JevValidationError::MissingAnswer { key: key.clone() });
        }
    }

    let priority_answer = answers.answers.get("priority").expect("checked above");
    let (priority, priority_distribution, confidence) = match priority_answer {
        JevAnswer::Choice {
            choice,
            probabilities,
            confidence,
        } => {
            let distribution = probabilities
                .as_ref()
                .ok_or_else(|| JevValidationError::ChoiceDistributionMissing("priority".into()))?;
            validate_distribution("priority", distribution, &choice_options())?;
            if !choice_options().contains(choice) {
                return Err(JevValidationError::ChoiceLabelOutsideOptions {
                    key: "priority".into(),
                    label: choice.clone(),
                });
            }
            let priority = choice.parse::<u8>().expect("choice options are numeric");
            (
                priority,
                ValidatedPriorityDistribution::new(distribution.clone()),
                *confidence,
            )
        }
        _ => {
            return Err(JevValidationError::AnswerTypeMismatch {
                key: "priority".into(),
                expected: "choice",
            })
        }
    };

    let (relevance, relevance_legend) = match answers.answers.get("relevance") {
        None => (None, None),
        Some(JevAnswer::Score {
            score,
            legend,
            probabilities,
        }) => {
            validate_score(
                "relevance",
                *score,
                legend,
                config.questions.relevance_levels.len(),
                probabilities.as_ref(),
            )?;
            (Some(*score), Some(legend.clone()))
        }
        Some(_) => {
            return Err(JevValidationError::AnswerTypeMismatch {
                key: "relevance".into(),
                expected: "score",
            })
        }
    };

    let mut category_probabilities = BTreeMap::new();
    if config.questions.include_categories {
        for category in &config.questions.category_vocabulary {
            let key = question_id("category", category);
            let probability =
                validate_noul(&key, answers.answers.get(&key).expect("checked above"))?;
            category_probabilities.insert(category.clone(), probability);
        }
    }
    let mut tag_probabilities = BTreeMap::new();
    if config.questions.include_tags {
        for tag in &config.questions.tag_vocabulary {
            let key = question_id("tag", tag);
            let probability =
                validate_noul(&key, answers.answers.get(&key).expect("checked above"))?;
            tag_probabilities.insert(tag.clone(), probability);
        }
    }
    let selected_categories = select_above_threshold(
        &category_probabilities,
        config.questions.category_probability_threshold,
    );
    let selected_tags = select_above_threshold(
        &tag_probabilities,
        config.questions.tag_probability_threshold,
    );

    let high_probability = p_high(&priority_distribution);
    Ok(ValidatedAnswers {
        model: answers.model.clone(),
        usage: answers.usage.clone(),
        priority,
        priority_distribution,
        confidence,
        p_high: high_probability,
        relevance,
        relevance_legend,
        category_probabilities,
        tag_probabilities,
        selected_categories,
        selected_tags,
    })
}

fn expected_keys(config: &RunConfig) -> BTreeSet<String> {
    let mut keys = BTreeSet::from([String::from("priority")]);
    if config.questions.include_relevance {
        keys.insert("relevance".into());
    }
    if config.questions.include_categories {
        keys.extend(
            config
                .questions
                .category_vocabulary
                .iter()
                .map(|value| question_id("category", value)),
        );
    }
    if config.questions.include_tags {
        keys.extend(
            config
                .questions
                .tag_vocabulary
                .iter()
                .map(|value| question_id("tag", value)),
        );
    }
    keys
}

fn choice_options() -> BTreeSet<String> {
    BTreeSet::from_iter((1..=5).map(|value| value.to_string()))
}

fn validate_score(
    key: &str,
    score: f64,
    legend: &BTreeMap<String, String>,
    expected_len: usize,
    probabilities: Option<&BTreeMap<String, f64>>,
) -> Result<(), JevValidationError> {
    if legend.is_empty() {
        return Err(JevValidationError::EmptyLegend { key: key.into() });
    }
    if legend.len() != expected_len {
        return Err(JevValidationError::LegendLengthMismatch {
            key: key.into(),
            expected: expected_len,
            actual: legend.len(),
        });
    }
    let mut indices = legend
        .keys()
        .map(|index| {
            index
                .parse::<usize>()
                .ok()
                .filter(|parsed| parsed.to_string() == *index)
                .ok_or(())
        })
        .collect::<Result<Vec<_>, _>>()
        .map_err(|_| JevValidationError::LegendKeysInvalid { key: key.into() })?;
    indices.sort_unstable();
    let Some(base) = indices.first().copied() else {
        return Err(JevValidationError::EmptyLegend { key: key.into() });
    };
    if !matches!(base, 0 | 1)
        || indices
            .iter()
            .copied()
            .ne(base..base.saturating_add(legend.len()))
    {
        return Err(JevValidationError::LegendKeysInvalid { key: key.into() });
    }
    let score_key = if score.is_finite() && score.fract() == 0.0 && score >= 0.0 {
        Some(format!("{score:.0}"))
    } else {
        None
    };
    if score_key
        .as_ref()
        .is_none_or(|value| !legend.contains_key(value))
    {
        return Err(JevValidationError::ScoreOutsideLegend {
            key: key.into(),
            score,
            legend_len: legend.len(),
        });
    }
    if let Some(probabilities) = probabilities {
        validate_distribution(key, probabilities, &legend.keys().cloned().collect())?;
    }
    Ok(())
}

fn validate_noul(key: &str, answer: &JevAnswer) -> Result<f64, JevValidationError> {
    let JevAnswer::Noul { probability } = answer else {
        return Err(JevValidationError::AnswerTypeMismatch {
            key: key.into(),
            expected: "noul",
        });
    };
    if !probability.is_finite() || !(0.0..=1.0).contains(probability) {
        return Err(JevValidationError::NoulProbabilityOutOfRange {
            key: key.into(),
            value: *probability,
        });
    }
    Ok(*probability)
}

fn validate_distribution(
    key: &str,
    distribution: &BTreeMap<String, f64>,
    expected: &BTreeSet<String>,
) -> Result<(), JevValidationError> {
    let actual = distribution.keys().cloned().collect::<BTreeSet<_>>();
    if actual != *expected {
        return Err(JevValidationError::DistributionKeysMismatch {
            key: key.into(),
            missing: expected.difference(&actual).cloned().collect(),
            extra: actual.difference(expected).cloned().collect(),
        });
    }
    for (option, probability) in distribution {
        if !probability.is_finite() {
            return Err(JevValidationError::NonFiniteProbability {
                question: key.into(),
                option: option.clone(),
            });
        }
        if !(0.0..=1.0).contains(probability) {
            return Err(JevValidationError::ProbabilityOutsideUnitInterval {
                question: key.into(),
                option: option.clone(),
                value: *probability,
            });
        }
    }
    let sum = distribution.values().sum::<f64>();
    if (sum - 1.0).abs() > 1e-3 {
        return Err(JevValidationError::ProbabilityMassOutOfTolerance {
            question: key.into(),
            sum,
        });
    }
    Ok(())
}
