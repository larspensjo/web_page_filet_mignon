use pulldown_cmark::{Event, HeadingLevel, Options, Parser, Tag, TagEnd};

pub const FONT_BODY: usize = 0;
pub const FONT_CODE: usize = 1;
pub const MAX_RTF_NESTING_DEPTH: usize = 20;
pub const RTF_TRUNCATE_MARKER: &str = "[display truncated]";
const BODY_FONT_SIZE_HALF_POINTS: usize = 24;
const BODY_INDENT_TWIPS: usize = 520;
const BODY_LINE_SPACING_TWIPS: usize = 360;
const HEADING_SPACING_AFTER_TWIPS: usize = 160;
const HEADING_SPACING_BEFORE_TWIPS: usize = 140;
const BODY_SPACING_AFTER_TWIPS: usize = 140;
const LIST_INDENT_TWIPS: usize = 920;
const LIST_HANGING_INDENT_TWIPS: usize = 360;
const COLOR_BODY_TEXT_RTF: &str = "\\red176\\green174\\blue165;";
const COLOR_HEADING_TEXT_RTF: &str = "\\red250\\green249\\blue245;";
const COLOR_BACKGROUND_RTF: &str = "\\red48\\green48\\blue46;";
const COLOR_LINK_RTF: &str = "\\red217\\green119\\blue87;";
const COLOR_CHIP_BG_RTF: &str = "\\red61\\green61\\blue58;}";

pub fn convert_markdown_to_rtf(markdown: &str) -> String {
    let mut rtf = String::new();
    rtf.push_str(&format!("{{\\rtf1\\ansi\\deff{FONT_BODY}"));
    rtf.push_str(&format!(
        "{{\\fonttbl{{\\f{FONT_BODY} Segoe UI;}}{{\\f{FONT_CODE} Consolas;}}}}"
    ));
    rtf.push_str("{\\colortbl;");
    rtf.push_str(COLOR_BODY_TEXT_RTF);
    rtf.push_str(COLOR_HEADING_TEXT_RTF);
    rtf.push_str(COLOR_BACKGROUND_RTF);
    rtf.push_str(COLOR_LINK_RTF);
    rtf.push_str(COLOR_CHIP_BG_RTF);
    rtf.push_str(&format!(
        "\\viewkind4\\uc1\\pard\\li{BODY_INDENT_TWIPS}\\ri{BODY_INDENT_TWIPS}\\sl{BODY_LINE_SPACING_TWIPS}\\slmult1\\sa{BODY_SPACING_AFTER_TWIPS}\\sb0\\cf1\\cb3\\f0\\fs{BODY_FONT_SIZE_HALF_POINTS} "
    ));

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
            Event::Text(text) => escape_rtf_text(&mut rtf, text.as_ref()),
            Event::Code(text) => {
                // Render inline code as a shaded chip (tag pill style).
                // \chshdng10000 = full character shading pattern; \chcbpat5 = use
                // chip background color (index 5 in the color table).
                rtf.push_str("\\chshdng10000\\chcbpat5 ");
                escape_rtf_text(&mut rtf, text.as_ref());
                rtf.push_str("\\chshdng0\\chcbpat0 ");
            }
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
    Unordered {
        item_paragraph_count: Option<usize>,
    },
    Ordered {
        next: u64,
        item_paragraph_count: Option<usize>,
    },
}

impl ListState {
    fn unordered() -> Self {
        Self::Unordered {
            item_paragraph_count: None,
        }
    }

    fn ordered(next: u64) -> Self {
        Self::Ordered {
            next,
            item_paragraph_count: None,
        }
    }

    fn start_item(&mut self) {
        match self {
            Self::Unordered {
                item_paragraph_count,
            }
            | Self::Ordered {
                item_paragraph_count,
                ..
            } => *item_paragraph_count = Some(0),
        }
    }

    fn finish_item(&mut self) {
        match self {
            Self::Unordered {
                item_paragraph_count,
            }
            | Self::Ordered {
                item_paragraph_count,
                ..
            } => *item_paragraph_count = None,
        }
    }

    fn next_item_paragraph_index(&mut self) -> Option<usize> {
        match self {
            Self::Unordered {
                item_paragraph_count,
            }
            | Self::Ordered {
                item_paragraph_count,
                ..
            } => item_paragraph_count.as_mut().map(|count| {
                let current = *count;
                *count += 1;
                current
            }),
        }
    }
}

fn handle_start_tag(rtf: &mut String, tag: &Tag<'_>, list_stack: &mut Vec<ListState>) {
    match tag {
        Tag::Heading { level, .. } => {
            let size = match level {
                HeadingLevel::H1 => 42,
                HeadingLevel::H2 => 34,
                _ => 28,
            };
            rtf.push_str(&format!(
                "\\pard\\li{BODY_INDENT_TWIPS}\\ri{BODY_INDENT_TWIPS}\\sa{HEADING_SPACING_AFTER_TWIPS}\\sb{HEADING_SPACING_BEFORE_TWIPS}\\cf2\\b\\fs{size} "
            ));
        }
        Tag::Paragraph => match list_stack
            .last_mut()
            .and_then(ListState::next_item_paragraph_index)
        {
            Some(0) => rtf.push_str(&format!(
                "\\sa80\\sb0\\sl{BODY_LINE_SPACING_TWIPS}\\slmult1 "
            )),
            Some(_) => rtf.push_str(&format!(
                "\\pard\\li{LIST_INDENT_TWIPS}\\ri{BODY_INDENT_TWIPS}\\sa80\\sb0\\sl{BODY_LINE_SPACING_TWIPS}\\slmult1 "
            )),
            None => rtf.push_str(&format!(
                "\\pard\\li{BODY_INDENT_TWIPS}\\ri{BODY_INDENT_TWIPS}\\sa{BODY_SPACING_AFTER_TWIPS}\\sb0\\sl{BODY_LINE_SPACING_TWIPS}\\slmult1 "
            )),
        },
        Tag::Strong => rtf.push_str("\\b "),
        Tag::Emphasis => rtf.push_str("\\i "),
        Tag::List(start) => match start {
            Some(start) => list_stack.push(ListState::ordered(*start)),
            None => list_stack.push(ListState::unordered()),
        },
        Tag::Item => {
            rtf.push_str(&format!(
                "\\par\\pard\\li{LIST_INDENT_TWIPS}\\ri{BODY_INDENT_TWIPS}\\fi-{LIST_HANGING_INDENT_TWIPS}\\tx{LIST_INDENT_TWIPS} "
            ));
            match list_stack.last_mut() {
                Some(list_state @ ListState::Unordered { .. }) => {
                    list_state.start_item();
                    rtf.push_str("\\bullet\\tab ");
                }
                Some(ListState::Ordered {
                    next,
                    item_paragraph_count,
                }) => {
                    let current = *next;
                    *next = next.saturating_add(1);
                    *item_paragraph_count = Some(0);
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
        TagEnd::Heading(_) => {
            rtf.push_str(&format!(
                "\\cf1\\b0\\fs{BODY_FONT_SIZE_HALF_POINTS}\\par\\pard\\li{BODY_INDENT_TWIPS}\\ri{BODY_INDENT_TWIPS}\\sa0\\sb0\\sl{BODY_LINE_SPACING_TWIPS}\\slmult1 "
            ));
        }
        TagEnd::Paragraph => rtf.push_str("\\par "),
        TagEnd::Strong => rtf.push_str("\\b0 "),
        TagEnd::Emphasis => rtf.push_str("\\i0 "),
        TagEnd::List(_) => {
            let _ = list_stack.pop();
            rtf.push_str("\\par ");
        }
        TagEnd::Item => {
            if let Some(list_state) = list_stack.last_mut() {
                list_state.finish_item();
            }
            rtf.push_str("\\pard ");
        }
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
    fn escape_rtf_writes_unicode_escape_sequences_for_non_bmp_text() {
        let mut out = String::new();
        escape_rtf_text(&mut out, "😀");
        assert_eq!(out.matches("\\u").count(), 2);
        assert!(out.ends_with('?'));
    }

    #[test]
    fn headings_render_as_distinct_bold_blocks() {
        let rtf = convert_markdown_to_rtf("# H1\n## H2\n### H3");
        assert!(rtf.contains("\\pard\\li520\\ri520\\sa160\\sb140\\cf2\\b\\fs42 H1"));
        assert!(rtf.contains("\\pard\\li520\\ri520\\sa160\\sb140\\cf2\\b\\fs34 H2"));
        assert!(rtf.contains("\\pard\\li520\\ri520\\sa160\\sb140\\cf2\\b\\fs28 H3"));
        assert!(
            rtf.matches("\\pard\\li520\\ri520\\sa160\\sb140\\cf2\\b\\fs")
                .count()
                >= 3
        );
        assert!(rtf.matches("\\par").count() >= 3);
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
        assert!(rtf.contains("\\bullet"));
        assert!(rtf.contains("one"));
        assert!(rtf.contains("two"));
    }

    #[test]
    fn unordered_list_uses_explicit_hanging_indent_and_text_tab() {
        let rtf = convert_markdown_to_rtf(
            "- A long bullet item that should wrap under the first line text column",
        );
        assert!(rtf.contains("\\pard\\li920\\ri520\\fi-360\\tx920 \\bullet\\tab "));
    }

    #[test]
    fn breaks_are_mapped() {
        let rtf = convert_markdown_to_rtf("a\nb  \nc");
        assert!(rtf.contains("a b"));
        assert!(rtf.contains("\\line "));
    }

    #[test]
    fn sample_briefing_preserves_visible_structure() {
        let markdown = "# Header\n\n- **Item**\n\nParagraph";
        let rtf = convert_markdown_to_rtf(markdown);
        assert!(rtf.starts_with("{\\rtf1"));
        assert!(rtf.ends_with('}'));
        assert!(rtf.contains("Header"));
        assert!(rtf.contains("Item"));
        assert!(rtf.contains("Paragraph"));
        assert!(rtf.contains("\\bullet"));
        assert!(rtf.contains("\\b "));
    }

    #[test]
    fn default_palette_and_body_size_match_readable_viewer_theme() {
        let rtf = convert_markdown_to_rtf("Body");
        assert!(rtf.contains(COLOR_BODY_TEXT_RTF));
        assert!(rtf.contains(COLOR_HEADING_TEXT_RTF));
        assert!(rtf.contains(COLOR_BACKGROUND_RTF));
        assert!(rtf.contains("\\fs24 "));
        assert!(rtf.contains("\\li520\\ri520\\sl360\\slmult1"));
    }

    #[test]
    fn loose_ordered_list_item_body_starts_on_new_paragraph() {
        let rtf = convert_markdown_to_rtf("1. **Headline**\n\n   Body text");
        assert!(rtf.contains(
            "\\pard\\li920\\ri520\\fi-360\\tx920 1.\\tab \\sa80\\sb0\\sl360\\slmult1 \\b Headline\\b0 \\par \\pard\\li920\\ri520\\sa80\\sb0\\sl360\\slmult1 Body text"
        ));
    }

    #[test]
    fn heading_after_list_starts_on_new_paragraph() {
        let rtf = convert_markdown_to_rtf("- one\n- two\n\n## Brave");
        assert!(rtf.contains("\\pard \\par \\pard\\li520\\ri520\\sa160\\sb140\\cf2\\b\\fs34 Brave"));
    }

    #[test]
    fn body_paragraphs_use_editorial_measure_and_spacing() {
        let rtf = convert_markdown_to_rtf("Paragraph");
        assert!(rtf.contains("\\pard\\li520\\ri520\\sa140\\sb0\\sl360\\slmult1 Paragraph"));
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

    #[test]
    fn inline_code_gets_character_background_shading() {
        let rtf = convert_markdown_to_rtf("a `chip` b");
        // The new chip background color must be declared in the color table.
        // `\red61\green61\blue58;` corresponds to #3D3D3A (spec's Surface Overlay).
        assert!(
            rtf.contains("\\red61\\green61\\blue58;"),
            "color table should include the chip background color; got: {rtf}"
        );
        // The inline code run must be wrapped in character shading control words.
        // We expect \chshdng10000\chcbpat<N> to enable, and \chshdng0\chcbpat0 to reset.
        assert!(
            rtf.contains("\\chshdng10000"),
            "inline code should enable character shading; got: {rtf}"
        );
        assert!(
            rtf.contains("\\chshdng0\\chcbpat0"),
            "inline code should reset character shading after the run; got: {rtf}"
        );
        // The chip text itself must still appear verbatim.
        assert!(rtf.contains("chip"));
    }
}
