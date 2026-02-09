/// Truncate a string to at most `max_chars` characters, never cutting a character boundary.
pub fn truncate_to_char_boundary(s: &str, max_chars: usize) -> &str {
    if max_chars == 0 {
        return "";
    }
    for (char_count, (index, _)) in s.char_indices().enumerate() {
        if char_count == max_chars {
            return &s[..index];
        }
    }
    s
}

/// Truncate a string to at most `max_bytes` bytes, never cutting a UTF-8 boundary.
pub fn truncate_to_byte_boundary(s: &str, max_bytes: usize) -> &str {
    if max_bytes == 0 {
        return "";
    }
    if max_bytes >= s.len() {
        return s;
    }
    let mut end = max_bytes;
    while end > 0 && !s.is_char_boundary(end) {
        end -= 1;
    }
    &s[..end]
}

#[cfg(test)]
mod tests {
    use super::{truncate_to_byte_boundary, truncate_to_char_boundary};

    #[test]
    fn ascii_string_truncates_normally() {
        let value = "abcdefghij";
        assert_eq!(truncate_to_char_boundary(value, 4), "abcd");
    }

    #[test]
    fn multi_byte_truncation_keeps_char_boundaries() {
        let emoji = "😀😃😄😁😆";
        assert_eq!(truncate_to_char_boundary(emoji, 2), "😀😃");
    }

    #[test]
    fn empty_string_returns_empty() {
        assert_eq!(truncate_to_char_boundary("", 5), "");
    }

    #[test]
    fn shorter_than_limit_returns_original() {
        let value = "short";
        assert_eq!(truncate_to_char_boundary(value, 10), "short");
    }

    #[test]
    fn byte_truncation_respects_utf8_boundaries() {
        let value = "Emoji ❤️ emoji";
        let truncated = truncate_to_byte_boundary(value, 10);
        assert!(truncated.is_char_boundary(truncated.len()));
        assert!(truncated.len() <= 10);
    }

    #[test]
    fn byte_truncation_handles_zero_limit() {
        let value = "text";
        assert_eq!(truncate_to_byte_boundary(value, 0), "");
    }

    #[test]
    fn byte_truncation_returns_full_string_when_under_limit() {
        let value = "abc";
        assert_eq!(truncate_to_byte_boundary(value, 10), "abc");
    }
}
