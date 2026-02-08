/// Truncate a string to at most `max_chars` characters, never cutting a character boundary.
pub fn truncate_to_char_boundary(s: &str, max_chars: usize) -> &str {
    if max_chars == 0 {
        return "";
    }
    let mut char_count = 0;
    for (index, _) in s.char_indices() {
        if char_count == max_chars {
            return &s[..index];
        }
        char_count += 1;
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
