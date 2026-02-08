use harvester_engine::{build_markdown_document, WhitespaceTokenCounter};

fn title_line(doc: &str) -> &str {
    doc.lines()
        .find(|line| line.starts_with("title: "))
        .expect("title line present")
}

fn url_line(doc: &str) -> &str {
    doc.lines()
        .find(|line| line.starts_with("url: "))
        .expect("url line present")
}

#[test]
fn title_with_newlines_does_not_break_frontmatter() {
    let (_count, doc) = build_markdown_document(
        "http://example.com",
        Some("evil\n---\ninjected: true"),
        "utf-8",
        "2026-02-08T00:00:00Z",
        "body",
        &WhitespaceTokenCounter,
    );
    assert_eq!(title_line(&doc), "title: \"evil --- injected: true\"");
}

#[test]
fn title_with_yaml_special_chars_is_escaped() {
    let (_count, doc) = build_markdown_document(
        "http://example.com",
        Some(": \" [] {}"),
        "utf-8",
        "2026-02-08T00:00:00Z",
        "body",
        &WhitespaceTokenCounter,
    );
    assert_eq!(title_line(&doc), "title: \": \\\" [] {}\"");
}

#[test]
fn url_with_newlines_gets_sanitized() {
    let (_count, doc) = build_markdown_document(
        "http://example.com\nfoo",
        Some("Safe title"),
        "utf-8",
        "2026-02-08T00:00:00Z",
        "body",
        &WhitespaceTokenCounter,
    );
    assert_eq!(url_line(&doc), "url: \"http://example.com foo\"");
}

#[test]
fn long_title_truncates_at_char_boundary() {
    let long_title = "😀".repeat(600);
    let (_count, doc) = build_markdown_document(
        "http://example.com",
        Some(&long_title),
        "utf-8",
        "2026-02-08T00:00:00Z",
        "body",
        &WhitespaceTokenCounter,
    );
    let expected = format!("title: \"{}\"", "😀".repeat(500));
    assert_eq!(title_line(&doc), expected);
}

#[test]
fn normal_titles_are_quoted_without_change() {
    let (_count, doc) = build_markdown_document(
        "http://example.com",
        Some("Simple Title"),
        "utf-8",
        "2026-02-08T00:00:00Z",
        "body",
        &WhitespaceTokenCounter,
    );
    assert_eq!(title_line(&doc), "title: \"Simple Title\"");
}
