pub trait Converter: Send + Sync {
    fn to_markdown(&self, html: &str) -> String;
}

#[derive(Debug, Default, Clone, Copy)]
pub struct Html2MdConverter;

impl Converter for Html2MdConverter {
    fn to_markdown(&self, html: &str) -> String {
        html2md::parse_html(html)
    }
}
