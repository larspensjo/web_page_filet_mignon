use url::Url;

pub fn archive_url_key(url: &str) -> String {
    let trimmed = url.trim();
    if trimmed.is_empty() {
        return String::new();
    }
    if let Ok(mut parsed) = Url::parse(trimmed) {
        parsed.set_fragment(None);
        if let Some(port) = parsed.port() {
            let normalized_port = match (parsed.scheme(), port) {
                ("http", 80) | ("https", 443) => None,
                _ => Some(port),
            };
            let _ = parsed.set_port(normalized_port);
        }
        return parsed.into();
    }
    trimmed.to_lowercase()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalises_https_default_port() {
        assert_eq!(
            archive_url_key("https://example.com:443/path"),
            archive_url_key("https://example.com/path"),
        );
    }

    #[test]
    fn normalises_http_default_port() {
        assert_eq!(
            archive_url_key("http://example.com:80/path"),
            archive_url_key("http://example.com/path"),
        );
    }

    #[test]
    fn preserves_non_default_port() {
        assert_ne!(
            archive_url_key("https://example.com:8443/path"),
            archive_url_key("https://example.com/path"),
        );
    }

    #[test]
    fn strips_fragment() {
        assert_eq!(
            archive_url_key("https://example.com/page#section"),
            archive_url_key("https://example.com/page"),
        );
    }

    #[test]
    fn host_is_case_insensitive() {
        assert_eq!(
            archive_url_key("https://EXAMPLE.COM/path"),
            archive_url_key("https://example.com/path"),
        );
    }

    #[test]
    fn http_and_https_are_distinct() {
        assert_ne!(
            archive_url_key("http://example.com/path"),
            archive_url_key("https://example.com/path"),
        );
    }

    #[test]
    fn non_parseable_input_is_lowercased_and_trimmed() {
        assert_eq!(archive_url_key("  NOT-A-URL  "), "not-a-url");
    }

    #[test]
    fn empty_input_returns_empty_string() {
        assert_eq!(archive_url_key("   "), "");
    }
}
