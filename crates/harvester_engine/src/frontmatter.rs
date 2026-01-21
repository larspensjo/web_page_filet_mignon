use crate::token::TokenCounter;

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
    let frontmatter = format!(
        "---\nurl: {url}\ntitle: {title}\nfetched_utc: {fetched_utc}\nencoding: {encoding}\ntoken_count: {token_count}\n---\n\n",
        url = url,
        title = title_val,
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
