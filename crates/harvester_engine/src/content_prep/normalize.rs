#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NormalizationPolicy {
    pub max_consecutive_blank_lines: usize,
    pub trim_trailing_whitespace: bool,
    pub normalize_line_endings: bool,
    pub collapse_horizontal_rules: bool,
    pub strip_html_comments: bool,
    pub normalize_unicode_whitespace: bool,
}

impl Default for NormalizationPolicy {
    fn default() -> Self {
        Self {
            max_consecutive_blank_lines: 2,
            trim_trailing_whitespace: true,
            normalize_line_endings: true,
            collapse_horizontal_rules: true,
            strip_html_comments: true,
            normalize_unicode_whitespace: true,
        }
    }
}

/// Pure transformation pipeline: same input + same policy always yields the same output.
pub fn normalize_markdown(text: &str, policy: &NormalizationPolicy) -> String {
    let mut normalized = text.to_string();

    if policy.normalize_line_endings {
        normalized = normalize_line_endings(&normalized);
    }

    if policy.normalize_unicode_whitespace {
        normalized = normalize_unicode_whitespace(&normalized);
    }

    if policy.strip_html_comments {
        normalized = strip_html_comments(&normalized);
    }

    if policy.trim_trailing_whitespace {
        normalized = trim_trailing_whitespace(&normalized);
    }

    normalized = collapse_blank_lines(&normalized, policy.max_consecutive_blank_lines);

    if policy.collapse_horizontal_rules {
        normalized = collapse_horizontal_rules(&normalized);
    }

    if policy.trim_trailing_whitespace {
        normalized = normalized.trim().to_string();
    }

    normalized
}

fn normalize_line_endings(text: &str) -> String {
    let mut result = String::with_capacity(text.len());
    let mut chars = text.chars().peekable();
    while let Some(ch) = chars.next() {
        if ch == '\r' {
            if chars.peek() == Some(&'\n') {
                chars.next();
            }
            result.push('\n');
        } else {
            result.push(ch);
        }
    }
    result
}

fn normalize_unicode_whitespace(text: &str) -> String {
    text.chars()
        .filter_map(|ch| match ch {
            '\u{00A0}'
            | '\u{1680}'
            | '\u{2000}'..='\u{200A}'
            | '\u{202F}'
            | '\u{205F}'
            | '\u{3000}' => Some(' '),
            '\u{200B}' | '\u{200C}' | '\u{200D}' | '\u{2060}' | '\u{FEFF}' => None,
            other => Some(other),
        })
        .collect()
}

fn strip_html_comments(text: &str) -> String {
    let mut result = String::with_capacity(text.len());
    let mut remaining = text;

    while let Some(start) = remaining.find("<!--") {
        result.push_str(&remaining[..start]);
        remaining = &remaining[start + 4..];
        if let Some(end) = remaining.find("-->") {
            remaining = &remaining[end + 3..];
        } else {
            remaining = "";
            break;
        }
    }

    result.push_str(remaining);
    result
}

fn trim_trailing_whitespace(text: &str) -> String {
    let mut result = String::with_capacity(text.len());

    for segment in text.split_inclusive('\n') {
        if segment.ends_with('\n') {
            let (content, _) = segment.split_at(segment.len() - 1);
            let trimmed = content.trim_end_matches(|c: char| matches!(c, ' ' | '\t' | '\r'));
            result.push_str(trimmed);
            result.push('\n');
        } else {
            let trimmed =
                segment.trim_end_matches(|c: char| matches!(c, ' ' | '\t' | '\r'));
            result.push_str(trimmed);
        }
    }

    result
}

fn collapse_blank_lines(text: &str, max_consecutive_blank_lines: usize) -> String {
    if max_consecutive_blank_lines == usize::MAX {
        return text.to_string();
    }

    if text.is_empty() {
        return String::new();
    }

    let mut result = String::with_capacity(text.len());
    let mut blank_count = 0;

    for segment in text.split_inclusive('\n') {
        let has_newline = segment.ends_with('\n');
        let content = if has_newline {
            &segment[..segment.len() - 1]
        } else {
            segment
        };

        if content.trim().is_empty() {
            blank_count += 1;
            if blank_count <= max_consecutive_blank_lines {
                result.push_str(content);
                if has_newline {
                    result.push('\n');
                }
            }
            continue;
        }

        blank_count = 0;
        result.push_str(content);
        if has_newline {
            result.push('\n');
        }
    }

    result
}

fn collapse_horizontal_rules(text: &str) -> String {
    let mut result = String::with_capacity(text.len());
    let mut previous_was_hr = false;

    for segment in text.split_inclusive('\n') {
        let has_newline = segment.ends_with('\n');
        let content = if has_newline {
            &segment[..segment.len() - 1]
        } else {
            segment
        };

        if is_horizontal_rule_line(content) {
            if !previous_was_hr {
                result.push_str(content);
                if has_newline {
                    result.push('\n');
                }
            }
            previous_was_hr = true;
            continue;
        }

        previous_was_hr = false;
        result.push_str(content);
        if has_newline {
            result.push('\n');
        }
    }

    result
}

fn is_horizontal_rule_line(line: &str) -> bool {
    let trimmed = line.trim();
    if trimmed.len() < 3 {
        return false;
    }

    let mut chars = trimmed.chars().filter(|c| !c.is_whitespace());
    let first = match chars.next() {
        Some(ch) if matches!(ch, '-' | '_' | '*') => ch,
        _ => return false,
    };

    let mut count = 1;
    for ch in chars {
        if ch != first {
            return false;
        }
        count += 1;
    }

    count >= 3
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalizes_crlf_and_trims() {
        let policy = NormalizationPolicy::default();
        let input = "line1\r\nline2\rline3\n";
        let normalized = normalize_markdown(input, &policy);
        assert_eq!(normalized, "line1\nline2\nline3");
    }

    #[test]
    fn collapses_blank_lines_to_limit() {
        let policy = NormalizationPolicy::default();
        let input = "para1\n\n\n\n\npara2\n";
        let normalized = normalize_markdown(input, &policy);
        // At most two blank lines remain between paragraphs.
        assert!(normalized.contains("para1\n\n\npara2"));
        assert!(!normalized.contains("para1\n\n\n\npara2"));
    }

    #[test]
    fn trims_trailing_whitespace_per_line() {
        let policy = NormalizationPolicy::default();
        let input = "keep   \ntrim\t\nend  ";
        let normalized = normalize_markdown(input, &policy);
        assert_eq!(normalized, "keep\ntrim\nend");
    }

    #[test]
    fn removes_html_comments() {
        let policy = NormalizationPolicy::default();
        let input = "before<!-- note -->after";
        let normalized = normalize_markdown(input, &policy);
        assert_eq!(normalized, "beforeafter");
    }

    #[test]
    fn replaces_nbsp_with_space() {
        let policy = NormalizationPolicy::default();
        let input = "a\u{00A0}b";
        let normalized = normalize_markdown(input, &policy);
        assert_eq!(normalized, "a b");
    }

    #[test]
    fn idempotent() {
        let policy = NormalizationPolicy::default();
        let input = "first  \n\nsecond";
        let once = normalize_markdown(input, &policy);
        let twice = normalize_markdown(&once, &policy);
        assert_eq!(once, twice);
    }

    #[test]
    fn empty_input_returns_empty() {
        let policy = NormalizationPolicy::default();
        assert!(normalize_markdown("", &policy).is_empty());
    }

    #[test]
    fn disabled_policy_leaves_input_untouched() {
        let policy = NormalizationPolicy {
            max_consecutive_blank_lines: usize::MAX,
            trim_trailing_whitespace: false,
            normalize_line_endings: false,
            collapse_horizontal_rules: false,
            strip_html_comments: false,
            normalize_unicode_whitespace: false,
        };
        let input = "  keep \n\nline  ";
        let normalized = normalize_markdown(input, &policy);
        assert_eq!(normalized, input);
    }
}
