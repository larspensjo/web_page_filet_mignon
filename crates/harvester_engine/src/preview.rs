const TRUNCATED_MARKER: &str = "\n.[truncated]";
pub const MAX_PREVIEW_CONTENT: usize = 40_960;

pub fn prepare_preview_content(markdown: &str) -> String {
    let stripped = strip_frontmatter(markdown);
    if stripped.len() <= MAX_PREVIEW_CONTENT {
        stripped.to_string()
    } else {
        let mut end = MAX_PREVIEW_CONTENT;
        while end > 0 && !stripped.is_char_boundary(end) {
            end -= 1;
        }
        let truncated = &stripped[..end];
        format!("{truncated}{TRUNCATED_MARKER}")
    }
}

fn strip_frontmatter(markdown: &str) -> &str {
    let prefix = "---\n";
    if let Some(rest) = markdown.strip_prefix(prefix) {
        if let Some(idx) = rest.find("\n---") {
            let mut after = &rest[idx + "\n---".len()..];
            if after.starts_with('\n') {
                after = &after[1..];
            }
            return after.trim_start_matches('\n');
        }
    }
    markdown
}

#[cfg(test)]
mod tests {
    use super::{prepare_preview_content, strip_frontmatter, MAX_PREVIEW_CONTENT};

    #[test]
    fn short_content_kept_as_is() {
        let content = "short preview";
        assert_eq!(prepare_preview_content(content), content);
    }

    #[test]
    fn truncated_content_appends_marker() {
        let content: String = "a".repeat(MAX_PREVIEW_CONTENT + 128);
        let preview = prepare_preview_content(&content);
        assert!(preview.ends_with("\n.[truncated]"));
        assert_eq!(preview.len(), MAX_PREVIEW_CONTENT + "\n.[truncated]".len());
        assert!(preview.len() <= MAX_PREVIEW_CONTENT + "\n.[truncated]".len());
    }

    #[test]
    fn strips_frontmatter_and_trims_blank_line() {
        let markdown = "---\nkey: value\n---\n\nbody\n";
        assert_eq!(strip_frontmatter(markdown), "body\n");
    }

    #[test]
    fn malformed_frontmatter_is_ignored() {
        let markdown = "---\nkey: value\nbody\n";
        assert_eq!(strip_frontmatter(markdown), markdown);
    }
}
