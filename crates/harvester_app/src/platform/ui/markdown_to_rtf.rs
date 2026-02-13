use pulldown_cmark::{Event, HeadingLevel, Options, Parser, Tag, TagEnd};

pub const FONT_BODY: usize = 0;
pub const FONT_CODE: usize = 1;
pub const MAX_RTF_NESTING_DEPTH: usize = 20;
pub const RTF_TRUNCATE_MARKER: &str = "[display truncated]";

pub fn convert_markdown_to_rtf(markdown: &str) -> String {
    let mut rtf = String::new();
    rtf.push_str(&format!("{{\\rtf1\\ansi\\deff{FONT_BODY}"));
    rtf.push_str(&format!(
        "{{\\fonttbl{{\\f{FONT_BODY} Segoe UI;}}{{\\f{FONT_CODE} Consolas;}}}}"
    ));
    rtf.push_str("{\\colortbl;");
    rtf.push_str("\\red216\\green222\\blue233;");
    rtf.push_str("\\red26\\green29\\blue34;");
    rtf.push_str("\\red88\\green166\\blue255;}");
    rtf.push_str("\\viewkind4\\uc1\\pard\\cf1\\cb2\\f0\\fs20 ");

    let parser = Parser::new_ext(markdown, Options::all());
    let mut list_stack: Vec<ListState> = Vec::new();
    let mut depth = 0usize;

    for event in parser {
        match event {
            Event::Start(tag) => {
                if depth < MAX_RTF_NESTING_DEPTH {
                    handle_start_tag(&mut rtf, &tag, &mut list_stack);
                }
                depth = depth.saturating_add(1);
            }
            Event::End(tag_end) => {
                depth = depth.saturating_sub(1);
                if depth < MAX_RTF_NESTING_DEPTH {
                    handle_end_tag(&mut rtf, tag_end, &mut list_stack);
                }
            }
            Event::Text(text) | Event::Code(text) => escape_rtf_text(&mut rtf, text.as_ref()),
            Event::SoftBreak => rtf.push(' '),
            Event::HardBreak => rtf.push_str("\\line "),
            Event::Rule => rtf.push_str("\\par "),
            Event::Html(text) | Event::InlineHtml(text) => escape_rtf_text(&mut rtf, text.as_ref()),
            Event::InlineMath(text) | Event::DisplayMath(text) => {
                escape_rtf_text(&mut rtf, text.as_ref())
            }
            Event::FootnoteReference(text) => escape_rtf_text(&mut rtf, text.as_ref()),
            Event::TaskListMarker(checked) => {
                if checked {
                    rtf.push_str("[x] ");
                } else {
                    rtf.push_str("[ ] ");
                }
            }
        }
    }

    rtf.push('}');
    rtf
}

#[derive(Debug, Clone, Copy)]
enum ListState {
    Unordered,
    Ordered { next: u64 },
}

fn handle_start_tag(rtf: &mut String, tag: &Tag<'_>, list_stack: &mut Vec<ListState>) {
    match tag {
        Tag::Heading { level, .. } => {
            let size = match level {
                HeadingLevel::H1 => 36,
                HeadingLevel::H2 => 32,
                _ => 28,
            };
            rtf.push_str(&format!("\\pard\\sa120\\sb60\\b\\fs{size} "));
        }
        Tag::Paragraph => rtf.push_str("\\pard\\sa60\\sb0 "),
        Tag::Strong => rtf.push_str("\\b "),
        Tag::Emphasis => rtf.push_str("\\i "),
        Tag::List(start) => match start {
            Some(start) => list_stack.push(ListState::Ordered { next: *start }),
            None => list_stack.push(ListState::Unordered),
        },
        Tag::Item => {
            rtf.push_str("\\par\\pard\\li360\\fi-180 ");
            match list_stack.last_mut() {
                Some(ListState::Unordered) => rtf.push_str("\\bullet\\tab "),
                Some(ListState::Ordered { next }) => {
                    let current = *next;
                    *next = next.saturating_add(1);
                    rtf.push_str(&format!("{current}.\\tab "));
                }
                None => rtf.push_str("\\bullet\\tab "),
            }
        }
        Tag::Link { .. } => rtf.push_str("\\cf3 "),
        _ => {}
    }
}

fn handle_end_tag(rtf: &mut String, tag_end: TagEnd, list_stack: &mut Vec<ListState>) {
    match tag_end {
        TagEnd::Heading(_) => rtf.push_str("\\b0\\fs20\\par\\pard\\sa60\\sb0 "),
        TagEnd::Paragraph => rtf.push_str("\\par "),
        TagEnd::Strong => rtf.push_str("\\b0 "),
        TagEnd::Emphasis => rtf.push_str("\\i0 "),
        TagEnd::List(_) => {
            let _ = list_stack.pop();
        }
        TagEnd::Item => rtf.push_str("\\pard "),
        TagEnd::Link => rtf.push_str("\\cf1 "),
        _ => {}
    }
}

pub fn escape_rtf_text(buffer: &mut String, text: &str) {
    for ch in text.chars() {
        match ch {
            '\\' => buffer.push_str("\\\\"),
            '{' => buffer.push_str("\\{"),
            '}' => buffer.push_str("\\}"),
            '\n' => buffer.push_str("\\line "),
            c if c.is_ascii() && !c.is_control() => buffer.push(c),
            c => {
                for unit in c.to_string().encode_utf16() {
                    let signed = unit as i16;
                    buffer.push_str(&format!("\\u{signed}?"));
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn escape_rtf_escapes_control_chars() {
        let mut out = String::new();
        escape_rtf_text(&mut out, "\\{}");
        assert_eq!(out, "\\\\\\{\\}");
    }

    #[test]
    fn escape_rtf_writes_bmp_unicode() {
        let mut out = String::new();
        escape_rtf_text(&mut out, "å");
        assert!(out.contains("\\u229?"));
    }

    #[test]
    fn escape_rtf_writes_surrogate_pair() {
        let mut out = String::new();
        escape_rtf_text(&mut out, "😀");
        assert!(out.contains("\\u-10179?\\u-8704?"));
    }

    #[test]
    fn headings_emit_expected_sizes() {
        let h1 = convert_markdown_to_rtf("# H1");
        let h2 = convert_markdown_to_rtf("## H2");
        let h3 = convert_markdown_to_rtf("### H3");
        assert!(h1.contains("\\fs36"));
        assert!(h2.contains("\\fs32"));
        assert!(h3.contains("\\fs28"));
    }

    #[test]
    fn bold_and_italic_emit_tags() {
        let rtf = convert_markdown_to_rtf("**bold** *italic* ***both***");
        assert!(rtf.contains("\\b "));
        assert!(rtf.contains("\\b0 "));
        assert!(rtf.contains("\\i "));
        assert!(rtf.contains("\\i0 "));
    }

    #[test]
    fn unordered_list_uses_bullet_tab() {
        let rtf = convert_markdown_to_rtf("- one\n- two");
        assert!(rtf.contains("\\bullet\\tab "));
    }

    #[test]
    fn breaks_are_mapped() {
        let rtf = convert_markdown_to_rtf("a\nb  \nc");
        assert!(rtf.contains("a b"));
        assert!(rtf.contains("\\line "));
    }

    #[test]
    fn snapshot_sample_briefing() {
        let markdown = "# Header\n\n- **Item**\n\nParagraph";
        let rtf = convert_markdown_to_rtf(markdown);
        assert_eq!(
            rtf,
            "{\\rtf1\\ansi\\deff0{\\fonttbl{\\f0 Segoe UI;}{\\f1 Consolas;}}{\\colortbl;\\red216\\green222\\blue233;\\red26\\green29\\blue34;\\red88\\green166\\blue255;}\\viewkind4\\uc1\\pard\\cf1\\cb2\\f0\\fs20 \\pard\\sa120\\sb60\\b\\fs36 Header\\b0\\fs20\\par\\pard\\sa60\\sb0 \\par\\pard\\li360\\fi-180 \\bullet\\tab \\b Item\\b0 \\pard \\pard\\sa60\\sb0 Paragraph\\par }"
        );
    }

    #[test]
    fn brace_balance_for_varied_inputs() {
        let inputs = [
            "",
            "plain",
            "{\\}",
            "emoji 😀",
            "# h\n\n**b**",
            &"x".repeat(10_000),
        ];
        for input in inputs {
            let rtf = convert_markdown_to_rtf(input);
            let opens = rtf.chars().filter(|c| *c == '{').count();
            let closes = rtf.chars().filter(|c| *c == '}').count();
            assert_eq!(opens, closes);
        }
    }

    #[test]
    fn converter_does_not_panic_for_varied_inputs() {
        let inputs = [
            "",
            "text",
            "```code```",
            &"a".repeat(50_000),
            "{broken} \\ input",
        ];
        for input in inputs {
            let _ = convert_markdown_to_rtf(input);
        }
    }

    #[test]
    fn deep_nesting_stays_bounded_and_does_not_panic() {
        let mut markdown = String::new();
        for _ in 0..500 {
            markdown.push_str("> ");
        }
        markdown.push_str("deep");
        let rtf = convert_markdown_to_rtf(&markdown);
        assert!(rtf.starts_with("{\\rtf"));
        assert!(rtf.ends_with('}'));
    }
}
