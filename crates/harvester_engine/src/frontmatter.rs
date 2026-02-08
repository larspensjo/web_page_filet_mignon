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
    let truncated_url = truncate_to_char_boundary(url, FRONTMATTER_VALUE_MAX);
    let truncated_title = truncate_to_char_boundary(title_val, FRONTMATTER_VALUE_MAX);
    let frontmatter = format!(
        "---\nurl: {url}\ntitle: {title}\nfetched_utc: {fetched_utc}\nencoding: {encoding}\ntoken_count: {token_count}\n---\n\n",
        url = truncated_url,
        title = truncated_title,
        fetched_utc = fetched_utc,
        encoding = encoding,
        token_count = token_count,
    );
    let doc = format!(
        "{frontmatter}{body}",
        frontmatter = frontmatter,
        body = body_markdown
    );
    (token_count, doc)
}
