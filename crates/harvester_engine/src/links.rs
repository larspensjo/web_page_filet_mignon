use ego_tree::NodeRef;
use scraper::node::Node;
use scraper::{ElementRef, Html};
use url::Url;

const DEFAULT_MAX_LINKS: usize = 5_000;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LinkKind {
    Hyperlink,
    Image,
    Email,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExtractedLink {
    pub url: String,
    pub text: Option<String>,
    pub kind: LinkKind,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConversionOutput {
    pub markdown: String,
    pub links: Vec<ExtractedLink>,
}

pub struct LinkExtractingConverter {
    max_links_per_job: usize,
}

impl LinkExtractingConverter {
    pub fn new() -> Self {
        Self::with_max_links(DEFAULT_MAX_LINKS)
    }

    pub fn with_max_links(max_links_per_job: usize) -> Self {
        Self { max_links_per_job }
    }

    pub fn convert(&self, html: &str, base_url: Option<&str>) -> ConversionOutput {
        let document = Html::parse_document(html);
        let base_url = base_url.and_then(|b| Url::parse(b).ok());
        let mut ctx = ConversionContext::new(base_url, self.max_links_per_job);

        for child in document.root_element().children() {
            self.visit_node(child, &mut ctx);
        }

        let (markdown, links) = ctx.into_output();

        ConversionOutput { markdown, links }
    }

    fn visit_node<'a>(&self, node: NodeRef<'a, Node>, ctx: &mut ConversionContext) {
        match node.value() {
            Node::Text(text) => ctx.append_text(text),
            Node::Element(_) => {
                if let Some(element) = ElementRef::wrap(node) {
                    self.visit_element(element, ctx);
                }
            }
            _ => {
                for child in node.children() {
                    self.visit_node(child, ctx);
                }
            }
        }
    }

    fn visit_element(&self, element: ElementRef, ctx: &mut ConversionContext) {
        let tag = element.value().name().to_ascii_lowercase();
        match tag.as_str() {
            "a" => self.handle_anchor(element, ctx),
            "img" => self.handle_image(element, ctx),
            "br" => ctx.ensure_newline(),
            "hr" => {
                ctx.ensure_newline();
                ctx.append_text("---");
                ctx.ensure_newline();
            }
            "li" => {
                ctx.ensure_newline();
                ctx.append_text("- ");
                self.visit_children(element, ctx);
                ctx.ensure_newline();
            }
            "p" | "div" | "section" | "article" | "header" | "footer" | "nav" | "figure"
            | "figcaption" | "table" | "tr" | "td" | "th" | "blockquote" | "address" => {
                ctx.ensure_newline();
                self.visit_children(element, ctx);
                ctx.ensure_newline();
            }
            "ul" | "ol" => {
                ctx.ensure_newline();
                self.visit_children(element, ctx);
                ctx.ensure_newline();
            }
            "h1" => {
                ctx.ensure_newline();
                ctx.append_text("# ");
                self.visit_children(element, ctx);
                ctx.ensure_newline();
            }
            "h2" => {
                ctx.ensure_newline();
                ctx.append_text("## ");
                self.visit_children(element, ctx);
                ctx.ensure_newline();
            }
            "h3" => {
                ctx.ensure_newline();
                ctx.append_text("### ");
                self.visit_children(element, ctx);
                ctx.ensure_newline();
            }
            "h4" => {
                ctx.ensure_newline();
                ctx.append_text("#### ");
                self.visit_children(element, ctx);
                ctx.ensure_newline();
            }
            "h5" => {
                ctx.ensure_newline();
                ctx.append_text("##### ");
                self.visit_children(element, ctx);
                ctx.ensure_newline();
            }
            "h6" => {
                ctx.ensure_newline();
                ctx.append_text("###### ");
                self.visit_children(element, ctx);
                ctx.ensure_newline();
            }
            "script" | "style" | "noscript" | "iframe" | "template" => {
                // skip scripting and presentation-only sections
            }
            _ => self.visit_children(element, ctx),
        }
    }

    fn visit_children(&self, element: ElementRef, ctx: &mut ConversionContext) {
        for child in element.children() {
            self.visit_node(child, ctx);
        }
    }

    fn handle_anchor(&self, element: ElementRef, ctx: &mut ConversionContext) {
        let href = element.value().attr("href").map(str::trim);
        let start = ctx.builder.len();
        self.visit_children(element, ctx);
        let end = ctx.builder.len();
        if let Some(raw) = href {
            if let Some(url) = resolve_url(raw, ctx.base_url.as_ref()) {
                let text = ctx.extract_substring(start, end);
                let kind = if url.scheme() == "mailto" {
                    LinkKind::Email
                } else {
                    LinkKind::Hyperlink
                };
                ctx.add_link(url.into(), text, kind);
            }
        }
    }

    fn handle_image(&self, element: ElementRef, ctx: &mut ConversionContext) {
        if let Some(src) = element.value().attr("src").map(str::trim) {
            if let Some(url) = resolve_url(src, ctx.base_url.as_ref()) {
                ctx.add_link(url.into(), String::new(), LinkKind::Image);
            }
        }
    }
}

fn resolve_url(reference: &str, base: Option<&Url>) -> Option<Url> {
    let trimmed = reference.trim();
    if trimmed.is_empty() {
        return None;
    }
    let lower = trimmed.to_ascii_lowercase();
    if lower.starts_with('#') || lower.starts_with('?') || lower.starts_with("javascript:") {
        return None;
    }
    if let Ok(url) = Url::parse(trimmed) {
        return Some(url);
    }
    base.and_then(|base| base.join(trimmed).ok())
}

struct ConversionContext {
    builder: String,
    links: Vec<ExtractedLink>,
    base_url: Option<Url>,
    max_links: usize,
    last_char: Option<char>,
}

impl ConversionContext {
    fn new(base_url: Option<Url>, max_links: usize) -> Self {
        Self {
            builder: String::new(),
            links: Vec::new(),
            base_url,
            max_links,
            last_char: None,
        }
    }

    fn into_output(self) -> (String, Vec<ExtractedLink>) {
        (self.builder.trim().to_string(), self.links)
    }

    fn append_text(&mut self, text: &str) {
        for ch in text.chars() {
            if ch.is_whitespace() {
                if self.last_char == Some(' ') || self.last_char == Some('\n') {
                    continue;
                }
                self.push_char(' ');
            } else {
                self.push_char(ch);
            }
        }
    }

    fn ensure_newline(&mut self) {
        if self.last_char == Some('\n') || self.builder.is_empty() {
            return;
        }
        self.push_char('\n');
    }

    fn push_char(&mut self, ch: char) {
        self.builder.push(ch);
        self.last_char = Some(ch);
    }

    fn extract_substring(&self, start: usize, end: usize) -> String {
        self.builder[start..end].trim().to_string()
    }

    fn add_link(&mut self, url: String, text: String, kind: LinkKind) {
        if self.links.len() >= self.max_links {
            return;
        }

        let text = if text.trim().is_empty() {
            None
        } else {
            Some(text)
        };

        self.links.push(ExtractedLink { url, text, kind });
    }
}

impl Default for LinkExtractingConverter {
    fn default() -> Self {
        Self::new()
    }
}
