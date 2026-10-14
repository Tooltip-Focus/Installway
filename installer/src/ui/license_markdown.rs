// SPDX-License-Identifier: MIT
// Copyright (c) 2026 Gaëtan Dezeiraud, Louis Pinaud

//! Markdown parsing shared by the WinUI and Win32 license views.
//!
//! The payload remains a plain UTF-8 string for backward compatibility. Both
//! frontends parse that string through this module, so `.txt` and `.md` files
//! have exactly the same behavior and there is no format flag to keep in sync.

use pulldown_cmark::{Event, HeadingLevel, Parser, Tag, TagEnd};
use std::fmt::Write;

pub(crate) const BUILT_IN_LICENSE: &str = "END USER LICENSE AGREEMENT - SAMPLE\r\n\r\n\
Lorem ipsum dolor sit amet, consectetur adipiscing elit. Sed do eiusmod \
tempor incididunt ut labore et dolore magna aliqua. Ut enim ad minim veniam, \
quis nostrud exercitation ullamco laboris nisi ut aliquip ex ea commodo \
consequat.\r\n\r\n\
Duis aute irure dolor in reprehenderit in voluptate velit esse cillum dolore \
eu fugiat nulla pariatur. Excepteur sint occaecat cupidatat non proident, \
sunt in culpa qui officia deserunt mollit anim id est laborum.\r\n\r\n\
Sed ut perspiciatis unde omnis iste natus error sit voluptatem accusantium \
doloremque laudantium, totam rem aperiam, eaque ipsa quae ab illo inventore \
veritatis et quasi architecto beatae vitae dicta sunt explicabo.\r\n\r\n\
Nemo enim ipsam voluptatem quia voluptas sit aspernatur aut odit aut fugit, \
sed quia consequuntur magni dolores eos qui ratione voluptatem sequi nesciunt.\r\n\r\n\
At vero eos et accusamus et iusto odio dignissimos ducimus qui blanditiis \
praesentium voluptatum deleniti atque corrupti quos dolores et quas molestias \
excepturi sint occaecati cupiditate non provident, similique sunt in culpa \
qui officia deserunt mollitia animi, id est laborum et dolorum fuga.\r\n\r\n\
By clicking 'I accept' you agree to be bound by the terms above.";

#[derive(Debug, Default, Eq, PartialEq)]
pub(crate) struct Document {
    pub blocks: Vec<Block>,
}

#[derive(Debug, Eq, PartialEq)]
pub(crate) struct Block {
    pub kind: BlockKind,
    pub inlines: Vec<Inline>,
    pub quote_depth: usize,
    pub list_depth: usize,
    pub list_prefix: Option<String>,
}

impl Block {
    pub fn heading_level(&self) -> Option<u8> {
        match self.kind {
            BlockKind::Heading(level) => Some(level),
            _ => None,
        }
    }

    pub fn prefix(&self) -> String {
        format!(
            "{}{}",
            "│ ".repeat(self.quote_depth),
            self.list_prefix.as_deref().unwrap_or("")
        )
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum BlockKind {
    Paragraph,
    Heading(u8),
    Code,
    Rule,
}

#[derive(Debug, Eq, PartialEq)]
pub(crate) struct Inline {
    pub text: String,
    pub bold: bool,
    pub italic: bool,
    pub code: bool,
    pub link: Option<String>,
}

#[derive(Default)]
struct Style {
    bold: usize,
    italic: usize,
    code: bool,
    link: Option<String>,
}

struct ListState {
    next: Option<u64>,
}

#[derive(Default)]
struct Builder {
    document: Document,
    current: Option<Block>,
    style: Style,
    quote_depth: usize,
    lists: Vec<ListState>,
    pending_list_prefix: Option<String>,
    image_depth: usize,
}

impl Builder {
    fn start_block(&mut self, kind: BlockKind) {
        self.finish_block();
        self.current = Some(Block {
            kind,
            inlines: Vec::new(),
            quote_depth: self.quote_depth,
            list_depth: self.lists.len(),
            list_prefix: self.pending_list_prefix.take(),
        });
    }

    fn ensure_block(&mut self) -> &mut Block {
        if self.current.is_none() {
            self.start_block(BlockKind::Paragraph);
        }
        self.current.as_mut().expect("block was just created")
    }

    fn finish_block(&mut self) {
        let Some(block) = self.current.take() else {
            return;
        };
        if !block.inlines.is_empty() || matches!(block.kind, BlockKind::Rule) {
            self.document.blocks.push(block);
        }
    }

    fn text(&mut self, text: &str) {
        if self.image_depth > 0 || text.is_empty() {
            return;
        }
        let inline = Inline {
            text: text.to_string(),
            bold: self.style.bold > 0,
            italic: self.style.italic > 0,
            code: self.style.code,
            link: self.style.link.clone(),
        };
        let block = self.ensure_block();
        if let Some(previous) = block.inlines.last_mut()
            && previous.bold == inline.bold
            && previous.italic == inline.italic
            && previous.code == inline.code
            && previous.link == inline.link
        {
            previous.text.push_str(&inline.text);
        } else {
            block.inlines.push(inline);
        }
    }

    fn line_break(&mut self) {
        self.text("\n");
    }

    fn start_item(&mut self) {
        self.finish_block();
        self.pending_list_prefix = self.lists.last_mut().map(|list| match &mut list.next {
            Some(next) => {
                let prefix = format!("{next}. ");
                *next += 1;
                prefix
            }
            None => "• ".to_string(),
        });
    }

    fn rule(&mut self) {
        self.start_block(BlockKind::Rule);
        self.text("────────────────────────");
        self.finish_block();
    }
}

pub(crate) fn parse(source: &str) -> Document {
    let mut builder = Builder::default();

    for event in Parser::new(source) {
        match event {
            Event::Start(tag) => start_tag(&mut builder, tag),
            Event::End(tag) => end_tag(&mut builder, tag),
            Event::Text(text) | Event::Html(text) | Event::InlineHtml(text) => builder.text(&text),
            Event::Code(text) => {
                let was_code = std::mem::replace(&mut builder.style.code, true);
                builder.text(&text);
                builder.style.code = was_code;
            }
            Event::SoftBreak | Event::HardBreak => builder.line_break(),
            Event::Rule => builder.rule(),
            _ => {}
        }
    }
    builder.finish_block();
    builder.document
}

fn start_tag(builder: &mut Builder, tag: Tag<'_>) {
    match tag {
        Tag::Paragraph => builder.start_block(BlockKind::Paragraph),
        Tag::Heading { level, .. } => builder.start_block(BlockKind::Heading(heading(level))),
        Tag::BlockQuote(_) => {
            builder.finish_block();
            builder.quote_depth += 1;
        }
        Tag::CodeBlock(_) => {
            builder.start_block(BlockKind::Code);
            builder.style.code = true;
        }
        Tag::List(first) => builder.lists.push(ListState { next: first }),
        Tag::Item => builder.start_item(),
        Tag::Emphasis => builder.style.italic += 1,
        Tag::Strong => builder.style.bold += 1,
        Tag::Link { dest_url, .. } => {
            builder.style.link = safe_link(&dest_url).map(str::to_owned);
        }
        Tag::Image { .. } => builder.image_depth += 1,
        _ => {}
    }
}

fn end_tag(builder: &mut Builder, tag: TagEnd) {
    match tag {
        TagEnd::Paragraph | TagEnd::Heading(_) => builder.finish_block(),
        TagEnd::BlockQuote(_) => {
            builder.finish_block();
            builder.quote_depth = builder.quote_depth.saturating_sub(1);
        }
        TagEnd::CodeBlock => {
            builder.style.code = false;
            builder.finish_block();
        }
        TagEnd::List(_) => {
            builder.finish_block();
            builder.lists.pop();
        }
        TagEnd::Item => builder.finish_block(),
        TagEnd::Emphasis => builder.style.italic = builder.style.italic.saturating_sub(1),
        TagEnd::Strong => builder.style.bold = builder.style.bold.saturating_sub(1),
        TagEnd::Link => builder.style.link = None,
        TagEnd::Image => builder.image_depth = builder.image_depth.saturating_sub(1),
        _ => {}
    }
}

fn heading(level: HeadingLevel) -> u8 {
    match level {
        HeadingLevel::H1 => 1,
        HeadingLevel::H2 => 2,
        HeadingLevel::H3 => 3,
        HeadingLevel::H4 => 4,
        HeadingLevel::H5 => 5,
        HeadingLevel::H6 => 6,
    }
}

fn safe_link(uri: &str) -> Option<&str> {
    let safe_scheme = ["https://", "http://", "mailto:"].iter().any(|prefix| {
        uri.len() > prefix.len()
            && uri
                .get(..prefix.len())
                .is_some_and(|scheme| scheme.eq_ignore_ascii_case(prefix))
    });
    (safe_scheme && !uri.chars().any(|c| c.is_control() || c == '"')).then_some(uri)
}

#[derive(Debug, Eq, PartialEq)]
pub(crate) struct LinkRange {
    pub start: i32,
    pub end: i32,
    pub uri: String,
}

/// Convert the shared document to Rich Text Format for the native Win32
/// RichEdit control. Formatting is deliberately generated from the parsed
/// model rather than from the source extension.
pub(crate) fn to_rtf(document: &Document) -> (String, Vec<LinkRange>) {
    let mut out = String::from(
        "{\\rtf1\\ansi\\ansicpg1252\\uc1\\deff0{\\fonttbl{\\f0 Segoe UI;}}\
         {\\colortbl;\\red0\\green102\\blue204;}\\viewkind4 ",
    );
    let mut links = Vec::new();
    let mut position = 0i32;

    for (block_index, block) in document.blocks.iter().enumerate() {
        // Six points after each paragraph mirrors the 8-DIP spacing used by
        // the WinUI block stack at the default 96 DPI.
        out.push_str("{\\pard\\sa120");
        // RTF indentation is expressed in twips: 18 DIP = 270 twips at 96 DPI.
        let indent = block.list_depth.saturating_sub(1) * 270;
        if indent > 0 {
            let _ = write!(out, "\\li{indent}");
        }
        // The Win32 chrome uses the standard 9-point Segoe UI dialog font.
        // Keep the Markdown body on that baseline and retain the WinUI heading
        // ratios above it. RTF sizes are half-points.
        let size = match block.kind {
            BlockKind::Heading(1) => 31,
            BlockKind::Heading(2) => 27,
            BlockKind::Heading(3) => 23,
            BlockKind::Heading(4) => 21,
            BlockKind::Heading(5 | 6) => 18,
            BlockKind::Code => 17,
            BlockKind::Rule => 13,
            BlockKind::Paragraph => 18,
            BlockKind::Heading(_) => 18,
        };
        let _ = write!(out, "\\fs{size} ");
        if block.heading_level().is_some() {
            out.push_str("\\b ");
        }
        if block.quote_depth > 0 || matches!(block.kind, BlockKind::Code) {
            out.push_str("\\i ");
        }

        let prefix = block.prefix();
        push_rtf_text(&mut out, &prefix);
        position += utf16_len(&prefix);

        for inline in &block.inlines {
            out.push('{');
            if inline.bold {
                out.push_str("\\b ");
            }
            if inline.italic || inline.code {
                out.push_str("\\i ");
            }
            let link_start = inline.link.is_some().then_some(position);
            if link_start.is_some() {
                // Reactor's native WinUI hyperlink surface does not expose
                // nested run styling, so links intentionally reset emphasis
                // here as well to keep both frontends visually equivalent.
                out.push_str("\\b0\\i0\\ul\\cf1 ");
            }
            push_rtf_text(&mut out, &inline.text);
            position += utf16_len(&inline.text);
            if let (Some(start), Some(uri)) = (link_start, &inline.link) {
                links.push(LinkRange {
                    start,
                    end: position,
                    uri: uri.clone(),
                });
            }
            out.push('}');
        }
        out.push_str("\\par}");
        if block_index + 1 < document.blocks.len() {
            position += 1;
        }
    }
    out.push('}');
    (out, links)
}

fn utf16_len(text: &str) -> i32 {
    text.encode_utf16().count() as i32
}

fn push_rtf_text(out: &mut String, text: &str) {
    for unit in text.encode_utf16() {
        match unit {
            0x7b | 0x7d | 0x5c => {
                out.push('\\');
                out.push(char::from_u32(unit as u32).expect("ASCII punctuation"));
            }
            0x0a => out.push_str("\\line "),
            0x20..=0x7e => out.push(char::from_u32(unit as u32).expect("ASCII character")),
            _ => {
                let _ = write!(out, "\\u{}?", unit as i16);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plain_text_keeps_line_breaks() {
        let document = parse("first line\r\nsecond line");
        assert_eq!(document.blocks.len(), 1);
        assert_eq!(
            document.blocks[0].inlines[0].text,
            "first line\nsecond line"
        );
    }

    #[test]
    fn parses_inline_formatting_and_safe_links() {
        let document = parse("**bold** *italic* [`site`](https://example.com) [bad](file:///c:/x)");
        let inlines = &document.blocks[0].inlines;
        assert!(inlines.iter().any(|i| i.text == "bold" && i.bold));
        assert!(inlines.iter().any(|i| i.text == "italic" && i.italic));
        assert!(inlines.iter().any(|i| {
            i.text == "site" && i.code && i.link.as_deref() == Some("https://example.com")
        }));
        assert!(
            inlines
                .iter()
                .any(|i| i.text.contains("bad") && i.link.is_none())
        );
        assert_eq!(
            safe_link("HTTPS://example.com"),
            Some("HTTPS://example.com")
        );
        assert_eq!(safe_link("https://"), None);
        assert_eq!(safe_link("javascript:alert(1)"), None);
    }

    #[test]
    fn parses_headings_quotes_and_nested_lists() {
        let document = parse("# Terms\n\n> Quote\n\n1. One\n   - Nested\n2. Two");
        assert_eq!(document.blocks[0].kind, BlockKind::Heading(1));
        assert_eq!(document.blocks[1].quote_depth, 1);
        assert_eq!(document.blocks[2].list_prefix.as_deref(), Some("1. "));
        assert_eq!(document.blocks[3].list_depth, 2);
        assert_eq!(document.blocks[3].list_prefix.as_deref(), Some("• "));
        assert_eq!(document.blocks[4].list_prefix.as_deref(), Some("2. "));
    }

    #[test]
    fn ignores_images_but_not_surrounding_text() {
        let document = parse("Before ![secret alt](https://example.com/a.png) after");
        let text: String = document.blocks[0]
            .inlines
            .iter()
            .map(|inline| inline.text.as_str())
            .collect();
        assert_eq!(text, "Before  after");
    }

    #[test]
    fn tables_remain_plain_text() {
        let document = parse("A | B\n--|--\n1 | 2");
        let text: String = document.blocks[0]
            .inlines
            .iter()
            .map(|inline| inline.text.as_str())
            .collect();
        assert_eq!(text, "A | B\n--|--\n1 | 2");
    }

    #[test]
    fn rtf_contains_block_and_inline_formatting() {
        let (rtf, links) = to_rtf(&parse(
            "# Terms\n\n> Read **carefully** and visit [site](https://example.com).",
        ));
        assert!(rtf.starts_with("{\\rtf1"));
        assert!(rtf.contains("\\fs31"));
        assert!(rtf.contains("\\b carefully"));
        assert!(rtf.contains("\\ul\\cf1 site"));
        assert_eq!(links.len(), 1);
        assert_eq!(links[0].uri, "https://example.com");
    }

    #[test]
    fn link_ranges_use_rich_edit_utf16_offsets() {
        let (_, links) = to_rtf(&parse("😀 [site](https://example.com)"));
        assert_eq!((links[0].start, links[0].end), (3, 7));
    }
}
