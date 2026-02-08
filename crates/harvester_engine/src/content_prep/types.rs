use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CleanTextReport {
    pub source_url: String,
    pub source_title: Option<String>,
    pub original_bytes: usize,
    pub original_tokens: u32,
    pub clean_bytes: usize,
    pub clean_tokens: u32,
    pub boilerplate_lines_removed: usize,
    pub boilerplate_patterns: Vec<String>,
    pub content_hash: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum TruncationBoundary {
    Paragraph,
    Sentence,
    Character,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CleanText {
    text: String,
    content_hash: String,
    report: CleanTextReport,
}

impl CleanText {
    pub(crate) fn new(text: String, content_hash: String, report: CleanTextReport) -> Self {
        Self {
            text,
            content_hash,
            report,
        }
    }

    pub fn text(&self) -> &str {
        &self.text
    }

    pub fn content_hash(&self) -> &str {
        &self.content_hash
    }

    pub fn report(&self) -> &CleanTextReport {
        &self.report
    }

    pub fn byte_count(&self) -> usize {
        self.text.len()
    }

    pub fn token_count(&self) -> u32 {
        self.report.clean_tokens
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn clean_text_accessors_reflect_state() {
        let report = CleanTextReport {
            source_url: "https://example.com".to_string(),
            source_title: Some("Example".to_string()),
            original_bytes: 100,
            original_tokens: 10,
            clean_bytes: 60,
            clean_tokens: 6,
            boilerplate_lines_removed: 2,
            boilerplate_patterns: vec!["cookie policy".into()],
            content_hash: "abc123".into(),
        };
        let clean_text = CleanText::new("clean".to_string(), "hash".to_string(), report.clone());

        assert_eq!(clean_text.text(), "clean");
        assert_eq!(clean_text.content_hash(), "hash");
        assert_eq!(clean_text.report(), &report);
        assert_eq!(clean_text.byte_count(), 5);
        assert_eq!(clean_text.token_count(), 6);
    }

    #[test]
    fn clean_text_report_serde_round_trip() {
        let report = CleanTextReport {
            source_url: String::from("https://example.com"),
            source_title: Some(String::from("Example")),
            original_bytes: 128,
            original_tokens: 12,
            clean_bytes: 96,
            clean_tokens: 9,
            boilerplate_lines_removed: 1,
            boilerplate_patterns: vec![String::from("navigation block")],
            content_hash: String::from("beefcafe"),
        };
        let serialized = serde_json::to_string(&report).unwrap();
        let deserialized: CleanTextReport = serde_json::from_str(&serialized).unwrap();
        assert_eq!(report, deserialized);
    }

    #[test]
    fn truncation_boundary_can_serialize() {
        let boundary = TruncationBoundary::Sentence;
        let serialized = serde_json::to_string(&boundary).unwrap();
        let deserialized: TruncationBoundary = serde_json::from_str(&serialized).unwrap();
        assert_eq!(boundary, deserialized);
    }
}
