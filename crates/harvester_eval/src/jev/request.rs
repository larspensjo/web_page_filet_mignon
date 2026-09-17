//! Deterministic, primary-arm Jev request construction.

use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::config::{ConfigError, RunConfig};
use crate::jev::question_id::question_id;
use crate::manifest::{ArticleEntry, Manifest};
use crate::prompt_identity::sha256;

pub use crate::jev::question_id::sanitize_identifier;

/// Frozen evidence supplied to the primary Jev arm.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FrozenText {
    pub text: String,
}

impl FrozenText {
    pub fn new(text: impl Into<String>) -> Self {
        Self { text: text.into() }
    }

    pub fn for_article(
        article: &ArticleEntry,
        text: impl Into<String>,
    ) -> Result<Self, RequestBuildError> {
        let frozen = Self::new(text);
        if sha256(&frozen.text) != article.evidence_hash {
            return Err(RequestBuildError::EvidenceHashMismatch {
                article_id: article.article_id.clone(),
            });
        }
        Ok(frozen)
    }
}

impl From<&str> for FrozenText {
    fn from(value: &str) -> Self {
        Self::new(value)
    }
}

/// Frozen rubric content and the manifest hash it was checked against.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FrozenRubric {
    pub text: String,
    pub manifest_sha256: String,
}

impl FrozenRubric {
    pub fn new(text: impl Into<String>, manifest_sha256: impl Into<String>) -> Self {
        Self {
            text: text.into(),
            manifest_sha256: manifest_sha256.into(),
        }
    }

    pub fn verified(
        text: impl Into<String>,
        manifest_sha256: impl Into<String>,
    ) -> Result<Self, RequestBuildError> {
        let rubric = Self::new(text, manifest_sha256);
        rubric.verify()?;
        Ok(rubric)
    }

    pub fn from_manifest(
        text: impl Into<String>,
        manifest: &Manifest,
    ) -> Result<Self, RequestBuildError> {
        Self::verified(text, manifest.frozen_inputs.rubric_sha256.clone())
    }

    pub fn verify(&self) -> Result<(), RequestBuildError> {
        let actual = sha256(&self.text);
        if actual != self.manifest_sha256 {
            return Err(RequestBuildError::RubricHashMismatch {
                expected: self.manifest_sha256.clone(),
                actual,
            });
        }
        Ok(())
    }
}

impl TryFrom<(&str, &str)> for FrozenRubric {
    type Error = RequestBuildError;

    fn try_from(value: (&str, &str)) -> Result<Self, Self::Error> {
        Self::verified(value.0, value.1)
    }
}

/// The JSON request envelope accepted by Jev.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct JevRequest {
    pub model: String,
    pub state: JevState,
    pub questions: BTreeMap<String, JevQuestion>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct JevState {
    pub editorial_criteria: String,
    pub article: JevArticle,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct JevArticle {
    pub id: String,
    pub text: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "type")]
pub enum JevQuestion {
    #[serde(rename = "choice")]
    Choice {
        instructions: String,
        criteria: BTreeMap<String, String>,
    },
    #[serde(rename = "score")]
    Score {
        instructions: String,
        criteria: Vec<String>,
    },
    #[serde(rename = "noul")]
    Noul { instructions: String },
}

#[derive(Debug, Error, PartialEq)]
pub enum RequestBuildError {
    #[error("frozen rubric hash mismatch: expected {expected}, actual {actual}")]
    RubricHashMismatch { expected: String, actual: String },
    #[error("frozen evidence hash mismatch for article {article_id}")]
    EvidenceHashMismatch { article_id: String },
    #[error("rubric has no complete PRIORITY SCALE")]
    MissingPriorityScale,
    #[error("priority instructions must say article content is evidence, not instructions")]
    UnsafePriorityInstructions,
    #[error("invalid run configuration: {0}")]
    InvalidConfig(#[from] ConfigError),
}

/// Builds the primary request. It never reads the editable context file.
pub fn build_request(
    article: &ArticleEntry,
    frozen_text: &FrozenText,
    frozen_rubric: &FrozenRubric,
    config: &RunConfig,
) -> Result<JevRequest, RequestBuildError> {
    config.validate()?;
    frozen_rubric.verify()?;
    if sha256(&frozen_text.text) != article.evidence_hash {
        return Err(RequestBuildError::EvidenceHashMismatch {
            article_id: article.article_id.clone(),
        });
    }
    if !is_evidence_instruction(&config.questions.priority_instructions) {
        return Err(RequestBuildError::UnsafePriorityInstructions);
    }

    let criteria = priority_criteria(&frozen_rubric.text)?;
    let mut questions = BTreeMap::new();
    questions.insert(
        "priority".into(),
        JevQuestion::Choice {
            instructions: config.questions.priority_instructions.clone(),
            criteria,
        },
    );
    if config.questions.include_relevance {
        questions.insert(
            "relevance".into(),
            JevQuestion::Score {
                instructions: config.questions.relevance_instructions.clone(),
                criteria: config.questions.relevance_levels.clone(),
            },
        );
    }
    if config.questions.include_categories {
        add_noul_questions(
            &mut questions,
            "category",
            &config.questions.category_vocabulary,
            &config.questions.category_instructions,
        );
    }
    if config.questions.include_tags {
        add_noul_questions(
            &mut questions,
            "tag",
            &config.questions.tag_vocabulary,
            &config.questions.tag_instructions,
        );
    }

    Ok(JevRequest {
        model: config.jev.model.clone(),
        state: JevState {
            editorial_criteria: frozen_rubric.text.clone(),
            article: JevArticle {
                id: article.article_id.clone(),
                text: frozen_text.text.clone(),
            },
        },
        questions,
    })
}

fn add_noul_questions(
    questions: &mut BTreeMap<String, JevQuestion>,
    prefix: &str,
    vocabulary: &[String],
    instructions: &str,
) {
    for value in vocabulary {
        questions.insert(
            question_id(prefix, value),
            JevQuestion::Noul {
                instructions: format!(
                    "{instructions} Proposition: the article matches the {value} {prefix}."
                ),
            },
        );
    }
}

fn priority_criteria(rubric: &str) -> Result<BTreeMap<String, String>, RequestBuildError> {
    let mut in_scale = false;
    let mut criteria = BTreeMap::new();
    for line in rubric.lines() {
        let trimmed = line.trim();
        if trimmed == "PRIORITY SCALE:" {
            in_scale = true;
            continue;
        }
        if in_scale
            && trimmed.ends_with(':')
            && trimmed
                .chars()
                .all(|c| c.is_ascii_uppercase() || c == ':' || c == ' ' || c == '&')
        {
            break;
        }
        if in_scale {
            let value = trimmed.strip_prefix("- ").unwrap_or(trimmed);
            let Some((number, _)) = value.split_once(':') else {
                continue;
            };
            if matches!(number, "1" | "2" | "3" | "4" | "5") {
                criteria.insert(number.to_string(), value.to_string());
            }
        }
    }
    if criteria.len() == 5 {
        let expected = BTreeSet::from(["1", "2", "3", "4", "5"]);
        if criteria.keys().map(String::as_str).collect::<BTreeSet<_>>() == expected {
            return Ok(criteria);
        }
    }
    Err(RequestBuildError::MissingPriorityScale)
}

fn is_evidence_instruction(value: &str) -> bool {
    let lower = value.to_ascii_lowercase();
    lower.contains("article content")
        && lower.contains("evidence")
        && (lower.contains("not instructions") || lower.contains("not as instructions"))
}

impl JevRequest {
    /// Stable compact JSON bytes for request diffing and fake-responder input.
    pub fn request_bytes(&self) -> serde_json::Result<Vec<u8>> {
        serde_json::to_vec(self)
    }
}
