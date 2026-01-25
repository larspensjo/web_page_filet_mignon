use harvester_engine::{
    decode_html, Converter, Extractor, Html2MdConverter, ReadabilityLikeExtractor,
};
use pretty_assertions::assert_eq;

#[test]
fn decode_respects_charset_header() {
    let bytes = b"caf\xe9"; // iso-8859-1
    let decoded = decode_html(bytes, Some("text/html; charset=ISO-8859-1")).unwrap();
    assert_eq!(decoded.html, "café");
    assert!(
        decoded.encoding_label.eq_ignore_ascii_case("ISO-8859-1")
            || decoded.encoding_label.eq_ignore_ascii_case("windows-1252")
    );
}

#[test]
fn decode_handles_utf8_bom() {
    let bytes = b"\xEF\xBB\xBFhello";
    let decoded = decode_html(bytes, Some("text/html")).unwrap();
    assert_eq!(decoded.html, "hello");
    assert_eq!(decoded.encoding_label, "UTF-8");
}

#[test]
fn extractor_prefers_article_then_body() {
    let html = r#"
    <html><head><title>Title</title></head>
    <body>
        <article><h1>Heading</h1><p>Body text</p></article>
    </body></html>
    "#;
    let extractor = ReadabilityLikeExtractor;
    let extracted = extractor.extract(html);
    assert_eq!(extracted.title.as_deref(), Some("Title"));
    assert!(extracted.content_html.contains("Heading"));
    assert!(extracted.content_html.contains("Body text"));
}

#[test]
fn converter_turns_html_into_markdown() {
    let html = r#"<h1>Hello</h1><p>world</p>"#;
    let md = Html2MdConverter.to_markdown(html, None);
    let trimmed = md.markdown.trim();
    assert!(
        trimmed.starts_with("# Hello") || trimmed.starts_with("Hello\n=="),
        "unexpected markdown output: {md:?}"
    );
    assert!(trimmed.contains("world"));
}

#[test]
fn pipeline_decode_extract_convert_is_deterministic() {
    let bytes = br#"<html><head><title>X</title></head><body><article><p>A</p><p>B</p></article></body></html>"#;
    let decoded = decode_html(bytes, Some("text/html; charset=utf-8")).unwrap();
    let extractor = ReadabilityLikeExtractor;
    let extracted = extractor.extract(&decoded.html);
    let md = Html2MdConverter.to_markdown(&extracted.content_html, None);
    assert_eq!(md.markdown.trim(), "A\n\nB");
}
