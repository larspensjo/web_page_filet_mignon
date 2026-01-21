use scraper::{Html, Selector};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExtractedContent {
    pub title: Option<String>,
    pub content_html: String,
}

pub trait Extractor: Send + Sync {
    fn extract(&self, html: &str) -> ExtractedContent;
}

/// Lightweight "readability-like" extractor:
/// - pulls `<title>` text if present
/// - returns `<article>` inner_html if present
/// - otherwise returns `<body>` inner_html
/// - fallback to full document HTML.
#[derive(Debug, Default)]
pub struct ReadabilityLikeExtractor;

impl Extractor for ReadabilityLikeExtractor {
    fn extract(&self, html: &str) -> ExtractedContent {
        let doc = Html::parse_document(html);
        let title_sel = Selector::parse("title").ok();
        let article_sel = Selector::parse("article").ok();
        let body_sel = Selector::parse("body").ok();

        let title = title_sel
            .as_ref()
            .and_then(|sel| doc.select(sel).next())
            .map(|t| t.text().collect::<String>().trim().to_string())
            .filter(|t| !t.is_empty());

        let content_html = if let Some(sel) = article_sel.as_ref() {
            if let Some(node) = doc.select(sel).next() {
                node.inner_html()
            } else {
                extract_body(&doc, &body_sel)
            }
        } else {
            extract_body(&doc, &body_sel)
        };

        ExtractedContent {
            title,
            content_html,
        }
    }
}

fn extract_body(doc: &Html, body_sel: &Option<Selector>) -> String {
    if let Some(sel) = body_sel {
        if let Some(node) = doc.select(sel).next() {
            return node.inner_html();
        }
    }
    doc.root_element().html()
}
