use crate::text_safety::truncate_to_char_boundary;
use crate::token::TokenCounter;

const FRONTMATTER_VALUE_MAX: usize = 500;

pub fn build_markdown_document(
    url: &str,
    title: Option<&str>,
    encoding: &str,
    fetched_utc: &str,
    body_markdown: &str,
    token_counter: &dyn TokenCounter,
) -> (u32, String) {
    let token_count = token_counter.count(body_markdown);
    let title_val = title.unwrap_or("untitled");
    let sanitized_url = sanitize_yaml_value(url);
    let sanitized_title = sanitize_yaml_value(title_val);
    let sanitized_encoding = sanitize_yaml_value(encoding);
    let sanitized_fetched = sanitize_yaml_value(fetched_utc);
    let frontmatter = format!(
        "---\nurl: {url}\ntitle: {title}\nfetched_utc: {fetched_utc}\nencoding: {encoding}\ntoken_count: {token_count}\n---\n\n",
        url = sanitized_url,
        title = sanitized_title,
        fetched_utc = sanitized_fetched,
        encoding = sanitized_encoding,
        token_count = token_count,
    );
    let doc = format!(
        "{frontmatter}{body}",
        frontmatter = frontmatter,
        body = body_markdown
    );
    (token_count, doc)
}

/// Fields extracted from the generated YAML frontmatter block.
#[derive(Debug, Clone, Default)]
pub struct FrontmatterFields {
    pub url: Option<String>,
    pub title: Option<String>,
    pub fetched_utc: Option<String>,
    pub encoding: Option<String>,
    pub token_count: Option<u32>,
}

/// Parses the frontmatter block at the top of a markdown document produced by `build_markdown_document`.
/// Returns `None` if no valid `---` block is present.
pub fn parse_frontmatter(markdown: &str) -> Option<FrontmatterFields> {
    let mut lines = markdown.lines();
    if lines.next()?.trim() != "---" {
        return None;
    }
    let mut fields = FrontmatterFields::default();
    for line in lines {
        let trimmed = line.trim();
        if trimmed == "---" {
            return Some(fields);
        }
        if trimmed.is_empty() {
            continue;
        }
        if let Some((key, value)) = trimmed.split_once(':') {
            let key = key.trim();
            let value = value.trim();
            match key {
                "url" => fields.url = Some(unescape_yaml_value(value)),
                "title" => fields.title = Some(unescape_yaml_value(value)),
                "fetched_utc" => fields.fetched_utc = Some(unescape_yaml_value(value)),
                "encoding" => fields.encoding = Some(unescape_yaml_value(value)),
                "token_count" => {
                    fields.token_count = value.parse::<u32>().ok();
                }
                _ => {}
            }
        }
    }
    None
}

/// Unescape a YAML value produced by `sanitize_yaml_value` in `build_markdown_document`.
pub fn unescape_yaml_value(value: &str) -> String {
    let trimmed = value.trim();
    if let Some(inner) = trimmed.strip_prefix('"').and_then(|s| s.strip_suffix('"')) {
        let mut result = String::with_capacity(inner.len());
        let mut chars = inner.chars();
        while let Some(ch) = chars.next() {
            if ch == '\\' {
                if let Some(next) = chars.next() {
                    match next {
                        '\\' => result.push('\\'),
                        '"' => result.push('"'),
                        other => {
                            result.push('\\');
                            result.push(other);
                        }
                    }
                } else {
                    result.push('\\');
                }
            } else {
                result.push(ch);
            }
        }
        result
    } else {
        trimmed.to_string()
    }
}

pub(crate) fn strip_frontmatter(markdown: &str) -> &str {
    let rest = if let Some(stripped) = markdown.strip_prefix("---\r\n") {
        stripped
    } else if let Some(stripped) = markdown.strip_prefix("---\n") {
        stripped
    } else {
        return markdown;
    };
    if let Some(idx) = rest.find("\n---") {
        let after = &rest[idx + "\n---".len()..];
        return after.trim_start_matches(['\n', '\r']);
    }
    markdown
}

fn sanitize_yaml_value(value: &str) -> String {
    let single_line = value.replace(&['\n', '\r'][..], " ");
    let truncated = truncate_to_char_boundary(&single_line, FRONTMATTER_VALUE_MAX);
    let escaped = truncated.replace('\\', "\\\\").replace('"', "\\\"");
    format!("\"{escaped}\"")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::token::WhitespaceTokenCounter;

    #[test]
    fn parse_frontmatter_round_trip() {
        let (token_count, doc) = build_markdown_document(
            "https://example.com/doc",
            Some("Document Title"),
            "utf-8",
            "2025-12-31T23:59:59Z",
            "body text",
            &WhitespaceTokenCounter,
        );
        let fields = parse_frontmatter(&doc).expect("frontmatter block missing");
        assert_eq!(fields.url.as_deref(), Some("https://example.com/doc"));
        assert_eq!(fields.title.as_deref(), Some("Document Title"));
        assert_eq!(fields.encoding.as_deref(), Some("utf-8"));
        assert_eq!(fields.fetched_utc.as_deref(), Some("2025-12-31T23:59:59Z"));
        assert_eq!(fields.token_count, Some(token_count));
    }

    #[test]
    fn parse_frontmatter_without_block_returns_none() {
        assert!(parse_frontmatter("no frontmatter here").is_none());
    }

    #[test]
    fn unescape_yaml_value_handles_escapes() {
        let value = "\"some\\\\escaped\\\"value\"";
        assert_eq!(unescape_yaml_value(value), "some\\escaped\"value");
    }
}
