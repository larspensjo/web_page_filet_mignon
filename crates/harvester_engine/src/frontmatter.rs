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
