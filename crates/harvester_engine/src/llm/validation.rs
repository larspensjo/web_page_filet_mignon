use std::convert::TryFrom;

use serde_json::{Map, Value};
use thiserror::Error;

use crate::llm::dto::{
    AggregateBriefing, ArticleSummary, BriefingTheme, TriagePriority, TriageResult,
};
use crate::text_safety::truncate_to_char_boundary;

const MAX_CATEGORY_LEN: usize = 120;
const MAX_RATIONALE_LEN: usize = 1024;
const MAX_TAGS: usize = 12;
const MAX_TAG_LEN: usize = 64;
const MAX_TITLE_LEN: usize = 200;
const MAX_RESPONSE_SUMMARY_CHARS: usize = 1200;
const MAX_KEY_POINTS: usize = 8;
const MAX_KEY_POINT_LEN: usize = 256;
const MAX_RESPONSE_EXEC_SUMMARY_CHARS: usize = 3000;
const MAX_THEMES: usize = 10;
const MAX_THEME_NAME_LEN: usize = 120;
const MAX_THEME_DESCRIPTION_LEN: usize = 512;

const FIELD_CATEGORY: &str = "category";
const FIELD_PRIORITY: &str = "priority";
const FIELD_TAGS: &str = "tags";
const FIELD_RATIONALE: &str = "rationale";
const FIELD_TITLE: &str = "title";
const FIELD_SUMMARY: &str = "summary";
const FIELD_KEY_POINTS: &str = "key_points";
const FIELD_EXEC_SUMMARY: &str = "executive_summary";
const FIELD_THEMES: &str = "themes";
const FIELD_THEME_NAME: &str = "name";
const FIELD_THEME_DESCRIPTION: &str = "description";
const FIELD_ARTICLE_COUNT: &str = "article_count";
const EXEC_SUMMARY_TRUNCATION_SUFFIX: &str =
    "\n\n[Truncated response: removed {removed} characters to fit the 3000-character limit.]";

/// Errors produced while validating parsed LLM output.
#[derive(Debug, PartialEq, Eq, Error)]
pub enum ValidationError {
    #[error("invalid JSON: {0}")]
    InvalidJson(String),
    #[error("schema violation: {0}")]
    SchemaViolation(String),
    #[error("value out of range: {0}")]
    ValueOutOfRange(&'static str),
    #[error("missing field: {0}")]
    MissingField(&'static str),
    #[error("field too long: {field} (actual_chars={actual_chars}, max_chars={max_chars})")]
    FieldTooLong {
        field: &'static str,
        max_chars: usize,
        actual_chars: usize,
    },
}

pub fn validate_triage(content: &str) -> Result<TriageResult, ValidationError> {
    let document = parse_document(content)?;
    let category = require_string(&document, FIELD_CATEGORY)?;
    ensure_max_length(category, MAX_CATEGORY_LEN, FIELD_CATEGORY)?;

    let priority_value = require_u64(&document, FIELD_PRIORITY)?;
    ensure_in_range(priority_value, FIELD_PRIORITY)?;
    let priority = TriagePriority::new(
        u8::try_from(priority_value)
            .map_err(|_| ValidationError::ValueOutOfRange(FIELD_PRIORITY))?,
    )
    .ok_or(ValidationError::ValueOutOfRange(FIELD_PRIORITY))?;

    let tags_array = require_array(&document, FIELD_TAGS)?;
    ensure_max_items(tags_array.len(), MAX_TAGS, FIELD_TAGS)?;
    let tags = tags_array
        .iter()
        .map(|value| {
            let tag = value.as_str().ok_or_else(|| {
                ValidationError::SchemaViolation("each tag must be a string".into())
            })?;
            ensure_max_length(tag, MAX_TAG_LEN, FIELD_TAGS)?;
            Ok(tag.to_string())
        })
        .collect::<Result<Vec<_>, ValidationError>>()?;

    let rationale = require_string(&document, FIELD_RATIONALE)?;
    ensure_max_length(rationale, MAX_RATIONALE_LEN, FIELD_RATIONALE)?;

    Ok(TriageResult {
        category: category.to_string(),
        priority,
        tags,
        rationale: rationale.to_string(),
    })
}

pub fn validate_summary(content: &str) -> Result<ArticleSummary, ValidationError> {
    let document = parse_document(content)?;
    let title = require_string(&document, FIELD_TITLE)?;
    ensure_max_length(title, MAX_TITLE_LEN, FIELD_TITLE)?;

    let summary = require_string(&document, FIELD_SUMMARY)?;
    ensure_max_length(summary, MAX_RESPONSE_SUMMARY_CHARS, FIELD_SUMMARY)?;

    let key_points_array = require_array(&document, FIELD_KEY_POINTS)?;
    ensure_max_items(key_points_array.len(), MAX_KEY_POINTS, FIELD_KEY_POINTS)?;
    let key_points = key_points_array
        .iter()
        .map(|value| {
            let point = value.as_str().ok_or_else(|| {
                ValidationError::SchemaViolation("key point must be a string".into())
            })?;
            ensure_max_length(point, MAX_KEY_POINT_LEN, FIELD_KEY_POINTS)?;
            Ok(point.to_string())
        })
        .collect::<Result<Vec<_>, ValidationError>>()?;

    Ok(ArticleSummary {
        title: title.to_string(),
        summary: summary.to_string(),
        key_points,
    })
}

pub fn validate_briefing(content: &str) -> Result<AggregateBriefing, ValidationError> {
    let document = parse_document(content)?;
    let executive_summary = require_string(&document, FIELD_EXEC_SUMMARY)?;
    let executive_summary = truncate_executive_summary(executive_summary);

    let themes_array = require_array(&document, FIELD_THEMES)?;
    ensure_max_items(themes_array.len(), MAX_THEMES, FIELD_THEMES)?;
    let themes = themes_array
        .iter()
        .map(|value| {
            let obj = value.as_object().ok_or_else(|| {
                ValidationError::SchemaViolation("each theme must be an object".into())
            })?;
            let name = require_string(obj, FIELD_THEME_NAME)?;
            ensure_max_length(name, MAX_THEME_NAME_LEN, FIELD_THEME_NAME)?;
            let description = require_string(obj, FIELD_THEME_DESCRIPTION)?;
            ensure_max_length(
                description,
                MAX_THEME_DESCRIPTION_LEN,
                FIELD_THEME_DESCRIPTION,
            )?;
            Ok(BriefingTheme {
                name: name.to_string(),
                description: description.to_string(),
            })
        })
        .collect::<Result<Vec<_>, ValidationError>>()?;

    let article_count_value = require_u64(&document, FIELD_ARTICLE_COUNT)?;
    let article_count = u32::try_from(article_count_value)
        .map_err(|_| ValidationError::ValueOutOfRange(FIELD_ARTICLE_COUNT))?;

    Ok(AggregateBriefing {
        executive_summary,
        themes,
        article_count,
    })
}

fn parse_document(content: &str) -> Result<Map<String, Value>, ValidationError> {
    let value = serde_json::from_str::<Value>(content)
        .map_err(|err| ValidationError::InvalidJson(err.to_string()))?;
    value
        .as_object()
        .cloned()
        .ok_or_else(|| ValidationError::SchemaViolation("root document must be an object".into()))
}

fn require_field<'a>(
    document: &'a Map<String, Value>,
    key: &'static str,
) -> Result<&'a Value, ValidationError> {
    document.get(key).ok_or(ValidationError::MissingField(key))
}

fn require_string<'a>(
    document: &'a Map<String, Value>,
    key: &'static str,
) -> Result<&'a str, ValidationError> {
    let value = require_field(document, key)?;
    value
        .as_str()
        .ok_or_else(|| ValidationError::SchemaViolation(format!("{key} must be a string")))
}

fn require_array<'a>(
    document: &'a Map<String, Value>,
    key: &'static str,
) -> Result<&'a Vec<Value>, ValidationError> {
    let value = require_field(document, key)?;
    value
        .as_array()
        .ok_or_else(|| ValidationError::SchemaViolation(format!("{key} must be an array")))
}

fn require_u64(document: &Map<String, Value>, key: &'static str) -> Result<u64, ValidationError> {
    let value = require_field(document, key)?;
    value.as_u64().ok_or_else(|| {
        ValidationError::SchemaViolation(format!("{key} must be a positive integer"))
    })
}

fn ensure_max_length(value: &str, max: usize, field: &'static str) -> Result<(), ValidationError> {
    let actual_chars = value.chars().count();
    if actual_chars > max {
        Err(ValidationError::FieldTooLong {
            field,
            max_chars: max,
            actual_chars,
        })
    } else {
        Ok(())
    }
}

fn truncate_executive_summary(value: &str) -> String {
    let actual_chars = value.chars().count();
    if actual_chars <= MAX_RESPONSE_EXEC_SUMMARY_CHARS {
        return value.to_string();
    }

    let mut removed_chars = actual_chars - MAX_RESPONSE_EXEC_SUMMARY_CHARS;
    loop {
        let suffix =
            EXEC_SUMMARY_TRUNCATION_SUFFIX.replace("{removed}", &removed_chars.to_string());
        let suffix_chars = suffix.chars().count();
        if suffix_chars >= MAX_RESPONSE_EXEC_SUMMARY_CHARS {
            return truncate_to_char_boundary(&suffix, MAX_RESPONSE_EXEC_SUMMARY_CHARS).to_string();
        }

        let preserved_chars = MAX_RESPONSE_EXEC_SUMMARY_CHARS - suffix_chars;
        let recalculated_removed = actual_chars - preserved_chars;
        if recalculated_removed == removed_chars {
            let prefix = truncate_to_char_boundary(value, preserved_chars);
            return format!("{prefix}{suffix}");
        }
        removed_chars = recalculated_removed;
    }
}

fn ensure_max_items(count: usize, max: usize, field: &'static str) -> Result<(), ValidationError> {
    if count > max {
        Err(ValidationError::ValueOutOfRange(field))
    } else {
        Ok(())
    }
}

fn ensure_in_range(value: u64, field: &'static str) -> Result<(), ValidationError> {
    if value == 0 || value > 5 {
        Err(ValidationError::ValueOutOfRange(field))
    } else {
        Ok(())
    }
}
