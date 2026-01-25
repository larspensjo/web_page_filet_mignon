use crate::links::{ConversionOutput, LinkExtractingConverter};

pub trait Converter: Send + Sync {
    fn to_markdown(&self, html: &str, base_url: Option<&str>) -> ConversionOutput;
}

#[derive(Debug, Default, Clone, Copy)]
pub struct Html2MdConverter;

impl Converter for Html2MdConverter {
    fn to_markdown(&self, html: &str, _base_url: Option<&str>) -> ConversionOutput {
        ConversionOutput {
            markdown: html2md::parse_html(html),
            links: Vec::new(),
        }
    }
}

impl Converter for LinkExtractingConverter {
    fn to_markdown(&self, html: &str, base_url: Option<&str>) -> ConversionOutput {
        self.convert(html, base_url)
    }
}
