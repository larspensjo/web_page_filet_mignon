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

#[cfg(test)]
mod tests {
    use super::truncate_to_char_boundary;

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
}
