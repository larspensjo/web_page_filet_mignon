use serde::{Deserialize, Serialize};

/// A single item parsed from a Brave News Search API response.
/// Preserves metadata for downstream use (triage, preview, provenance).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BraveNewsItem {
    pub url: String,
    pub title: String,
    pub description: String,
    /// Raw age string from the API, e.g. "2 hours ago".
    pub age: Option<String>,
}

#[derive(Debug, thiserror::Error)]
pub enum BravePollError {
    #[error("JSON parse failed: {0}")]
    JsonParse(String),
    #[error("unexpected response structure: {0}")]
    UnexpectedStructure(String),
}

/// Parse raw Brave News Search API JSON bytes into a list of items.
///
/// Only accepts the News Search shape (`results[]`).
/// Does NOT apply any limit or dedup — the caller handles that.
pub fn parse_brave_news_response(json_bytes: &[u8]) -> Result<Vec<BraveNewsItem>, BravePollError> {
    let value: serde_json::Value =
        serde_json::from_slice(json_bytes).map_err(|e| BravePollError::JsonParse(e.to_string()))?;

    let results_array = value
        .get("results")
        .and_then(|v| v.as_array())
        .ok_or_else(|| {
            BravePollError::UnexpectedStructure(
                "expected 'results' array in News Search response".to_string(),
            )
        })?;

    let mut items = Vec::new();
    for entry in results_array {
        if let Some(url) = entry.get("url").and_then(|v| v.as_str()) {
            let title = entry
                .get("title")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();
            let description = entry
                .get("description")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();
            let age = entry
                .get("age")
                .and_then(|v| v.as_str())
                .map(|s| s.to_string());
            items.push(BraveNewsItem {
                url: url.to_string(),
                title,
                description,
                age,
            });
        }
    }

    Ok(items)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn news_json(items: &[(&str, &str)]) -> Vec<u8> {
        let entries: Vec<String> = items
            .iter()
            .map(|(url, title)| {
                format!(
                    r#"{{"url":"{}","title":"{}","description":"desc"}}"#,
                    url, title
                )
            })
            .collect();
        format!(r#"{{"results":[{}]}}"#, entries.join(",")).into_bytes()
    }

    #[test]
    fn parses_news_api_response() {
        let json = news_json(&[
            ("https://example.com/1", "Title 1"),
            ("https://example.com/2", "Title 2"),
        ]);
        let items = parse_brave_news_response(&json).unwrap();
        assert_eq!(items.len(), 2);
        assert_eq!(items[0].url, "https://example.com/1");
        assert_eq!(items[0].title, "Title 1");
    }

    #[test]
    fn rejects_web_search_shape() {
        let json =
            br#"{"web":{"results":[{"url":"https://a.com","title":"A","description":"d"}]}}"#;
        let err = parse_brave_news_response(json).unwrap_err();
        assert!(matches!(err, BravePollError::UnexpectedStructure(_)));
    }

    #[test]
    fn rejects_invalid_json() {
        let err = parse_brave_news_response(b"not json").unwrap_err();
        assert!(matches!(err, BravePollError::JsonParse(_)));
    }

    #[test]
    fn rejects_missing_results_key() {
        let err = parse_brave_news_response(b"{}").unwrap_err();
        assert!(matches!(err, BravePollError::UnexpectedStructure(_)));
    }

    #[test]
    fn skips_entries_without_url() {
        let json = br#"{"results":[{"title":"no url"},{"url":"https://a.com","title":"A"}]}"#;
        let items = parse_brave_news_response(json).unwrap();
        assert_eq!(items.len(), 1);
        assert_eq!(items[0].url, "https://a.com");
    }

    #[test]
    fn preserves_age_field() {
        let json = br#"{"results":[{"url":"https://a.com","title":"A","description":"d","age":"2 hours ago"}]}"#;
        let items = parse_brave_news_response(json).unwrap();
        assert_eq!(items[0].age.as_deref(), Some("2 hours ago"));
    }
}
