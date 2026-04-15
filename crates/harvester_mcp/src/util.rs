/// Check whether fetched_utc matches the date range filters.
///
/// Returns `true` when no filters are specified. When filters are present, an
/// article without a `fetched_utc` value never matches.
pub(crate) fn date_in_range(
    fetched_utc: Option<&str>,
    date_from: Option<&str>,
    date_to: Option<&str>,
) -> bool {
    if date_from.is_none() && date_to.is_none() {
        return true;
    }
    match fetched_utc {
        None => false,
        Some(ts) => {
            if let Some(from) = date_from {
                if ts < from {
                    return false;
                }
            }
            if let Some(to) = date_to {
                if ts > to {
                    return false;
                }
            }
            true
        }
    }
}

pub(crate) fn compact_whitespace(text: &str) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

pub(crate) fn is_frontmatter_line(line: &str) -> bool {
    let trimmed = line.trim();
    if trimmed.is_empty() || trimmed == "---" {
        return true;
    }

    [
        "url:",
        "title:",
        "fetched_utc:",
        "token_count:",
        "summary_created_at_utc:",
        "summary_input_tokens:",
        "summary_output_tokens:",
    ]
    .iter()
    .any(|prefix| trimmed.starts_with(prefix))
}

pub(crate) fn is_low_signal_snippet_line(line: &str) -> bool {
    let trimmed = line.trim();
    if trimmed.is_empty() || is_frontmatter_line(trimmed) {
        return true;
    }

    let lower = trimmed.to_lowercase();
    if [
        "window.__",
        "__next_data__",
        "\"routing\":",
        "\"navstatus\":",
        "megamenu",
        "videoplayer",
        "newsletter",
        "cookie",
        "privacy policy",
        "terms of use",
        "all rights reserved",
        "related articles",
        "advertisement",
        "share this article",
        "follow us",
        "subscribe",
    ]
    .iter()
    .any(|pattern| lower.contains(pattern))
    {
        return true;
    }

    let alpha_chars = trimmed.chars().filter(|ch| ch.is_alphabetic()).count();
    let symbol_chars = trimmed
        .chars()
        .filter(|ch| !ch.is_alphanumeric() && !ch.is_whitespace())
        .count();
    trimmed.len() > 80 && alpha_chars > 0 && symbol_chars > alpha_chars
}

pub(crate) fn truncate_text_boundary(text: &str, max_chars: usize) -> String {
    if text.chars().count() <= max_chars {
        return text.to_string();
    }

    let mut end = text.len();
    for (char_count, (index, _)) in text.char_indices().enumerate() {
        if char_count == max_chars {
            end = index;
            break;
        }
    }

    let candidate = &text[..end];
    let trimmed = candidate
        .rfind(|ch: char| ch.is_whitespace() || [',', ';', ':', '.'].contains(&ch))
        .filter(|index| *index >= candidate.len() / 2)
        .map(|index| &candidate[..index])
        .unwrap_or(candidate)
        .trim();

    if trimmed.is_empty() {
        format!("{}...", candidate.trim())
    } else {
        format!("{trimmed}...")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn no_filters_always_matches() {
        assert!(date_in_range(None, None, None));
        assert!(date_in_range(Some("2026-01-01T00:00:00Z"), None, None));
    }

    #[test]
    fn missing_timestamp_never_matches_filtered_range() {
        assert!(!date_in_range(None, Some("2026-01-01"), None));
        assert!(!date_in_range(None, None, Some("2026-12-31")));
    }

    #[test]
    fn inclusive_bounds() {
        assert!(date_in_range(
            Some("2026-04-01T00:00:00Z"),
            Some("2026-04-01"),
            Some("2026-04-30")
        ));
        assert!(!date_in_range(
            Some("2026-03-31T23:59:59Z"),
            Some("2026-04-01"),
            None
        ));
        assert!(!date_in_range(
            Some("2026-05-01T00:00:00Z"),
            None,
            Some("2026-04-30")
        ));
    }

    #[test]
    fn low_signal_snippet_lines_detect_frontmatter_and_js_blob_lines() {
        assert!(is_low_signal_snippet_line("title: \"Alpha\""));
        assert!(is_low_signal_snippet_line(
            "window.__s_data={\"routing\":{\"locationBeforeTransitions\":null}}"
        ));
        assert!(!is_low_signal_snippet_line(
            "Microsoft and OpenAI are renegotiating data center capacity."
        ));
    }
}
