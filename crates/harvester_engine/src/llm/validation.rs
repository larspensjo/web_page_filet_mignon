use std::convert::TryFrom;

use serde_json::{Map, Value};
use thiserror::Error;

use crate::llm::dto::{
    AggregateBriefing, ArticleSummary, BriefingStory, SummaryEntities, TriagePriority, TriageResult,
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
const MAX_TOP_STORIES: usize = 5;
const MAX_STORY_HEADLINE_LEN: usize = 160;
const MAX_STORY_BODY_WORDS: usize = 150;

const FIELD_CATEGORY: &str = "category";
const FIELD_PRIORITY: &str = "priority";
const FIELD_TAGS: &str = "tags";
const FIELD_RATIONALE: &str = "rationale";
const FIELD_TITLE: &str = "title";
const FIELD_SUMMARY: &str = "summary";
const FIELD_KEY_POINTS: &str = "key_points";
const FIELD_EXEC_SUMMARY: &str = "executive_summary";
const FIELD_TOP_STORIES: &str = "top_stories";
const FIELD_STORY_HEADLINE: &str = "headline";
const FIELD_STORY_BODY: &str = "body";
const FIELD_ARTICLE_COUNT: &str = "article_count";
const FIELD_ENTITIES: &str = "entities";
const FIELD_ENTITIES_COMPANIES: &str = "companies";
const FIELD_ENTITIES_TECHNOLOGIES: &str = "technologies";
const FIELD_ENTITIES_PRODUCTS: &str = "products";
const MAX_ENTITY_ITEMS: usize = 15;
const MAX_ENTITY_INPUT_ITEMS: usize = 100;
const MAX_ENTITY_ITEM_LEN: usize = 100;
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
            Ok(truncate_to_char_boundary(point, MAX_KEY_POINT_LEN).to_string())
        })
        .collect::<Result<Vec<_>, ValidationError>>()?;

    // Optional entities block — absent means V3 response; treat as empty.
    let entities = match document.get(FIELD_ENTITIES) {
        None => SummaryEntities::default(),
        Some(val) => {
            let obj = val.as_object().ok_or_else(|| {
                ValidationError::SchemaViolation("entities must be an object".into())
            })?;
            SummaryEntities {
                companies: parse_entity_list(obj, FIELD_ENTITIES_COMPANIES)?,
                technologies: parse_entity_list(obj, FIELD_ENTITIES_TECHNOLOGIES)?,
                products: parse_entity_list(obj, FIELD_ENTITIES_PRODUCTS)?,
            }
        }
    };

    Ok(ArticleSummary {
        title: title.to_string(),
        summary: summary.to_string(),
        key_points,
        entities,
    })
}

/// Parse, validate, and deduplicate an entity list from a JSON object field.
///
/// - Accepts modest model overshoot up to `MAX_ENTITY_INPUT_ITEMS` raw items.
/// - Silently keeps at most `MAX_ENTITY_ITEMS` items after deduplication.
/// - Each item: non-empty string, no internal newlines, max `MAX_ENTITY_ITEM_LEN` chars.
/// - Deduplicate case-insensitively; first occurrence wins.
/// - Missing field treated as empty array (backward compatible).
fn parse_entity_list(
    obj: &Map<String, Value>,
    field: &'static str,
) -> Result<Vec<String>, ValidationError> {
    let array = match obj.get(field) {
        None => return Ok(Vec::new()),
        Some(v) => v.as_array().ok_or_else(|| {
            ValidationError::SchemaViolation(format!("entities.{field} must be an array"))
        })?,
    };
    ensure_max_items(array.len(), MAX_ENTITY_INPUT_ITEMS, field)?;

    let mut seen_lower: Vec<String> = Vec::new();
    let mut result: Vec<String> = Vec::new();

    for val in array {
        let item = val.as_str().ok_or_else(|| {
            ValidationError::SchemaViolation(format!("each entities.{field} item must be a string"))
        })?;
        if item.is_empty() {
            return Err(ValidationError::SchemaViolation(format!(
                "entities.{field} items must be non-empty"
            )));
        }
        if item.contains('\n') || item.contains('\r') {
            return Err(ValidationError::SchemaViolation(format!(
                "entities.{field} items must not contain newlines"
            )));
        }
        ensure_max_length(item, MAX_ENTITY_ITEM_LEN, field)?;
        let lower = item.to_lowercase();
        if !seen_lower.contains(&lower) {
            seen_lower.push(lower);
            if result.len() < MAX_ENTITY_ITEMS {
                result.push(item.to_string());
            }
        }
    }
    Ok(result)
}

pub fn validate_briefing(content: &str) -> Result<AggregateBriefing, ValidationError> {
    let document = parse_document(content)?;
    let executive_summary = require_string(&document, FIELD_EXEC_SUMMARY)?;
    let executive_summary = truncate_executive_summary(executive_summary);

    let top_stories = parse_briefing_stories(&document)?;

    let article_count_value = require_u64(&document, FIELD_ARTICLE_COUNT)?;
    let article_count = u32::try_from(article_count_value)
        .map_err(|_| ValidationError::ValueOutOfRange(FIELD_ARTICLE_COUNT))?;

    Ok(AggregateBriefing {
        executive_summary,
        top_stories,
        article_count,
    })
}

fn parse_briefing_stories(
    document: &Map<String, Value>,
) -> Result<Vec<BriefingStory>, ValidationError> {
    if let Some(stories_value) = document.get(FIELD_TOP_STORIES) {
        let stories_array = stories_value.as_array().ok_or_else(|| {
            ValidationError::SchemaViolation(format!("{FIELD_TOP_STORIES} must be an array"))
        })?;
        ensure_max_items(stories_array.len(), MAX_TOP_STORIES, FIELD_TOP_STORIES)?;
        return stories_array
            .iter()
            .map(|value| {
                let obj = value.as_object().ok_or_else(|| {
                    ValidationError::SchemaViolation("each top story must be an object".into())
                })?;
                let headline = require_string(obj, FIELD_STORY_HEADLINE)?;
                ensure_max_length(headline, MAX_STORY_HEADLINE_LEN, FIELD_STORY_HEADLINE)?;
                let body = require_string(obj, FIELD_STORY_BODY)?;
                Ok(BriefingStory {
                    headline: headline.to_string(),
                    body: truncate_to_word_limit(body, MAX_STORY_BODY_WORDS),
                })
            })
            .collect::<Result<Vec<_>, ValidationError>>();
    }

    let themes_array = require_array(document, "themes")?;
    ensure_max_items(themes_array.len(), MAX_TOP_STORIES, "themes")?;
    themes_array
        .iter()
        .map(|value| {
            let obj = value.as_object().ok_or_else(|| {
                ValidationError::SchemaViolation("each theme must be an object".into())
            })?;
            let headline = require_string(obj, "name")?;
            ensure_max_length(headline, MAX_STORY_HEADLINE_LEN, "name")?;
            let body = require_string(obj, "description")?;
            Ok(BriefingStory {
                headline: headline.to_string(),
                body: truncate_to_word_limit(body, MAX_STORY_BODY_WORDS),
            })
        })
        .collect::<Result<Vec<_>, ValidationError>>()
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

fn truncate_to_word_limit(value: &str, max_words: usize) -> String {
    let words: Vec<&str> = value.split_whitespace().collect();
    if words.len() <= max_words {
        return value.trim().to_string();
    }
    format!("{}...", words[..max_words].join(" "))
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

#[cfg(test)]
mod tests {
    use super::*;

    // ── validate_summary entity tests ────────────────────────────────────────

    #[test]
    fn validate_summary_v4_response_with_all_entities() {
        let json = r#"{
            "title": "AI Chip Race",
            "summary": "Nvidia leads with H100.",
            "key_points": ["Point A", "Point B"],
            "entities": {
                "companies": ["Nvidia", "TSMC"],
                "technologies": ["custom silicon", "large language models"],
                "products": ["H100"]
            }
        }"#;
        let result = validate_summary(json).expect("valid V4 response");
        assert_eq!(result.entities.companies, vec!["Nvidia", "TSMC"]);
        assert_eq!(
            result.entities.technologies,
            vec!["custom silicon", "large language models"]
        );
        assert_eq!(result.entities.products, vec!["H100"]);
    }

    #[test]
    fn validate_summary_v4_response_with_empty_entity_arrays() {
        let json = r#"{
            "title": "No Entities",
            "summary": "A generic article.",
            "key_points": ["Point A"],
            "entities": {
                "companies": [],
                "technologies": [],
                "products": []
            }
        }"#;
        let result = validate_summary(json).expect("valid V4 response with empty entities");
        assert!(result.entities.companies.is_empty());
        assert!(result.entities.technologies.is_empty());
        assert!(result.entities.products.is_empty());
    }

    #[test]
    fn validate_summary_v3_response_without_entities_gives_empty() {
        let json = r#"{
            "title": "Old Title",
            "summary": "Old summary text.",
            "key_points": ["Key point one"]
        }"#;
        let result = validate_summary(json).expect("valid V3 response");
        assert!(
            result.entities.companies.is_empty(),
            "V3 response should have empty companies"
        );
        assert!(
            result.entities.technologies.is_empty(),
            "V3 response should have empty technologies"
        );
        assert!(
            result.entities.products.is_empty(),
            "V3 response should have empty products"
        );
    }

    #[test]
    fn validate_summary_rejects_entity_string_exceeding_max_length() {
        let long_name = "A".repeat(MAX_ENTITY_ITEM_LEN + 1);
        let json = format!(
            r#"{{"title":"T","summary":"S","key_points":["P"],"entities":{{"companies":["{long_name}"],"technologies":[],"products":[]}}}}"#
        );
        let err = validate_summary(&json).expect_err("should reject oversized entity string");
        assert!(
            matches!(err, ValidationError::FieldTooLong { .. }),
            "expected FieldTooLong, got {err:?}"
        );
    }

    #[test]
    fn validate_summary_deduplicates_entity_strings_case_insensitively() {
        let json = r#"{
            "title": "Dedup Test",
            "summary": "Summary.",
            "key_points": ["P"],
            "entities": {
                "companies": ["Nvidia", "nvidia", "NVIDIA"],
                "technologies": [],
                "products": []
            }
        }"#;
        let result = validate_summary(json).expect("valid response");
        // Only the first occurrence should survive deduplication
        assert_eq!(result.entities.companies, vec!["Nvidia"]);
    }

    #[test]
    fn validate_summary_rejects_entity_with_newline() {
        let json = r#"{
            "title": "T",
            "summary": "S",
            "key_points": ["P"],
            "entities": {
                "companies": ["Bad\nEntity"],
                "technologies": [],
                "products": []
            }
        }"#;
        let err = validate_summary(json).expect_err("should reject entity with newline");
        assert!(
            matches!(err, ValidationError::SchemaViolation(_)),
            "expected SchemaViolation, got {err:?}"
        );
    }

    #[test]
    fn validate_summary_rejects_empty_entity_string() {
        let json = r#"{
            "title": "T",
            "summary": "S",
            "key_points": ["P"],
            "entities": {
                "companies": [""],
                "technologies": [],
                "products": []
            }
        }"#;
        let err = validate_summary(json).expect_err("should reject empty entity string");
        assert!(
            matches!(err, ValidationError::SchemaViolation(_)),
            "expected SchemaViolation, got {err:?}"
        );
    }

    #[test]
    fn validate_summary_clamps_entity_lists_after_deduplication() {
        let companies = (0..20)
            .map(|idx| format!(r#""Company {idx}""#))
            .collect::<Vec<_>>()
            .join(",");
        let json = format!(
            r#"{{
                "title": "T",
                "summary": "S",
                "key_points": ["P"],
                "entities": {{
                    "companies": ["Company 0", "company 0", {companies}],
                    "technologies": [],
                    "products": []
                }}
            }}"#
        );

        let result = validate_summary(&json).expect("entity overflow should be clamped");
        assert_eq!(result.entities.companies.len(), MAX_ENTITY_ITEMS);
        assert_eq!(result.entities.companies[0], "Company 0");
        assert_eq!(result.entities.companies[14], "Company 14");
    }

    #[test]
    fn validate_summary_rejects_pathological_entity_list_size() {
        let companies = (0..=MAX_ENTITY_INPUT_ITEMS)
            .map(|idx| format!(r#""Company {idx}""#))
            .collect::<Vec<_>>()
            .join(",");
        let json = format!(
            r#"{{
                "title": "T",
                "summary": "S",
                "key_points": ["P"],
                "entities": {{
                    "companies": [{companies}],
                    "technologies": [],
                    "products": []
                }}
            }}"#
        );

        assert_eq!(
            validate_summary(&json).unwrap_err(),
            ValidationError::ValueOutOfRange("companies")
        );
    }

    #[test]
    fn validate_summary_rejects_entities_not_an_object() {
        let json = r#"{
            "title": "T",
            "summary": "S",
            "key_points": ["P"],
            "entities": ["not", "an", "object"]
        }"#;
        let err = validate_summary(json).expect_err("entities must be an object");
        assert!(
            matches!(err, ValidationError::SchemaViolation(_)),
            "expected SchemaViolation, got {err:?}"
        );
    }
}
