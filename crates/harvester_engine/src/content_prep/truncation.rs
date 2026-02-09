use crate::text_safety::truncate_to_byte_boundary;

use super::types::TruncationBoundary;

pub const TRUNCATION_MARKER: &str = "\n\n[content truncated]";

pub fn truncate_to_budget(text: &str, max_bytes: usize) -> (String, Option<TruncationBoundary>) {
    if text.len() <= max_bytes {
        return (text.to_string(), None);
    }

    if max_bytes <= TRUNCATION_MARKER.len() {
        let trimmed = truncate_to_byte_boundary(text, max_bytes);
        return (trimmed.to_string(), Some(TruncationBoundary::Character));
    }

    let available = max_bytes - TRUNCATION_MARKER.len();
    let min_utilization = ((max_bytes as f64 * 0.2).ceil() as usize).max(1);
    let search_space = truncate_to_byte_boundary(text, available);

    if let Some(idx) = find_paragraph_boundary(search_space) {
        if idx >= min_utilization {
            let trimmed = &text[..idx];
            return (
                format!("{trimmed}{TRUNCATION_MARKER}"),
                Some(TruncationBoundary::Paragraph),
            );
        }
    }

    if let Some(idx) = find_sentence_boundary(search_space) {
        if idx >= min_utilization {
            let trimmed = &text[..idx];
            return (
                format!("{trimmed}{TRUNCATION_MARKER}"),
                Some(TruncationBoundary::Sentence),
            );
        }
    }

    let trimmed = search_space;
    (
        format!("{trimmed}{TRUNCATION_MARKER}"),
        Some(TruncationBoundary::Character),
    )
}

fn find_paragraph_boundary(text: &str) -> Option<usize> {
    text.rmatch_indices("\n\n").map(|(idx, _)| idx).next()
}

fn find_sentence_boundary(text: &str) -> Option<usize> {
    let boundaries = [". ", "! ", "? "];
    boundaries
        .iter()
        .filter_map(|boundary| text.rfind(boundary).map(|idx| idx + boundary.len() - 1))
        .max()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn within_budget_text() -> &'static str {
        "Short text complete."
    }

    #[test]
    fn returns_original_if_within_budget() {
        let text = within_budget_text();
        let max = text.len();
        let (result, boundary) = truncate_to_budget(text, max);
        assert_eq!(result, text);
        assert!(boundary.is_none());
    }

    #[test]
    fn paragraph_boundary_truncation() {
        let text = "Para one.\n\nPara two has more sentences.\n\nPara three";
        let max = text.len() - 10;
        let (result, boundary) = truncate_to_budget(text, max);
        assert!(result.contains("[content truncated]"));
        assert_eq!(boundary, Some(TruncationBoundary::Paragraph));
        assert!(result.len() <= max);
    }

    #[test]
    fn sentence_boundary_fallback() {
        let text = "Sentence one. Sentence two is the longest yet. No paragraph break.";
        let max = text.len() - 15;
        let (result, boundary) = truncate_to_budget(text, max);
        assert!(result.contains("[content truncated]"));
        assert_eq!(boundary, Some(TruncationBoundary::Sentence));
        assert!(result.len() <= max);
    }

    #[test]
    fn character_fallback_when_no_boundaries() {
        let text = "Averylongsinglelinewithoutspacesormarkers";
        let max = 30;
        let (result, boundary) = truncate_to_budget(text, max);
        assert!(result.contains("[content truncated]"));
        assert_eq!(boundary, Some(TruncationBoundary::Character));
        assert!(result.len() <= max);
    }

    #[test]
    fn respects_unicode_boundaries() {
        let text = "Emoji ❤️ emoji ❤️ emoji ❤️ emoji";
        let max = text.len() - 5;
        let (result, boundary) = truncate_to_budget(text, max);
        assert!(result.len() <= max);
        assert!(result.is_char_boundary(result.len()));
        assert_eq!(boundary, Some(TruncationBoundary::Character));
    }

    #[test]
    fn enforces_minimum_utilization_before_paragraph_cut() {
        let text = "Short paragraph.\n\nTiny tail.";
        let max = text.len() - 1;
        let (result, boundary) = truncate_to_budget(text, max);
        assert_eq!(boundary, Some(TruncationBoundary::Character));
        assert!(result.len() <= max);
    }

    #[test]
    fn marker_not_added_when_budget_too_small() {
        let text = "Some content repeated to exceed the marker length so truncation kicks in.";
        let max = TRUNCATION_MARKER.len();
        let (result, boundary) = truncate_to_budget(text, max);
        assert!(!result.contains("[content truncated]"));
        assert_eq!(boundary, Some(TruncationBoundary::Character));
        assert!(result.len() <= max);
    }
}
