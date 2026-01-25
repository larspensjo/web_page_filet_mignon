use harvester_engine::{Converter, ExtractedLink, LinkExtractingConverter, LinkKind};
use pretty_assertions::assert_eq;

fn convert(html: &str, base: Option<&str>) -> harvester_engine::ConversionOutput {
    LinkExtractingConverter::new().to_markdown(html, base)
}

#[test]
fn anchor_links_are_extracted_and_text_preserved() {
    let html = r#"<p>Hello <a href="https://example.com/path">world</a>!</p>"#;
    let output = convert(html, None);

    assert!(output.markdown.contains("Hello world"));
    assert_eq!(
        output.links,
        vec![ExtractedLink {
            url: "https://example.com/path".to_string(),
            text: Some("world".to_string()),
            kind: LinkKind::Hyperlink,
        }]
    );
}

#[test]
fn image_src_urls_are_collected_and_markdown_drops_images() {
    let html = r#"<p>Before<img src="/images/pic.jpg" srcset="foo 1x, bar 2x">After</p>"#;
    let output = convert(html, Some("https://news.example.com/base/"));

    assert!(output.markdown.contains("Before"));
    assert!(output.markdown.contains("After"));
    assert!(!output.markdown.contains("img"));
    assert_eq!(
        output.links,
        vec![ExtractedLink {
            url: "https://news.example.com/images/pic.jpg".to_string(),
            text: None,
            kind: LinkKind::Image,
        }]
    );
}

#[test]
fn mailto_links_are_classified_as_email() {
    let html = r#"<p><a href="mailto:foo@example.com">Ping me</a></p>"#;
    let output = convert(html, None);

    assert_eq!(output.links.len(), 1);
    assert_eq!(output.links[0].kind, LinkKind::Email);
    assert_eq!(output.links[0].url, "mailto:foo@example.com");
}

#[test]
fn relative_urls_resolve_with_base_and_fragments_are_skipped() {
    let html = "<a href=\"./article\">Article</a><a href=\"#top\">Skip</a>";
    let output = convert(html, Some("https://base.example.com/docs/"));

    assert_eq!(output.links.len(), 1);
    assert_eq!(output.links[0].url, "https://base.example.com/docs/article");
}

#[test]
fn link_limit_is_enforced() {
    let html = (0..4)
        .map(|i| format!(r#"<a href="https://ex.com/{i}">link{i}</a>"#))
        .collect::<Vec<_>>()
        .join(" ");
    let converter = LinkExtractingConverter::with_max_links(2);
    let output = converter.to_markdown(&html, None);

    assert_eq!(output.links.len(), 2);
    assert_eq!(output.links[0].url, "https://ex.com/0");
    assert_eq!(output.links[1].url, "https://ex.com/1");
}

#[test]
fn conversion_is_deterministic() {
    let html = r#"<p><a href="https://det.example/page">Det</a></p>"#;
    let converter = LinkExtractingConverter::new();

    let first = converter.to_markdown(html, Some("https://det.example/"));
    let second = converter.to_markdown(html, Some("https://det.example/"));

    assert_eq!(first, second);
}
