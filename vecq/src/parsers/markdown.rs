use crate::error::VecqResult;
use crate::parser::Parser;
use crate::types::{
    attr, DocumentElement, DocumentMetadata, ElementType, FileType, ParsedDocument,
};
use async_trait::async_trait;
use pulldown_cmark::{Event, HeadingLevel, Options, Tag, TagEnd};
use std::path::PathBuf;

#[derive(Clone)]
pub struct MarkdownParser;

impl Default for MarkdownParser {
    fn default() -> Self {
        Self::new()
    }
}

impl MarkdownParser {
    pub fn new() -> Self {
        Self
    }
}

#[async_trait]
impl Parser for MarkdownParser {
    async fn parse(&self, content: &str) -> VecqResult<ParsedDocument> {
        Ok(parse_markdown_document(content))
    }

    fn file_extensions(&self) -> &[&str] {
        &["md", "markdown"]
    }

    fn language_name(&self) -> &str {
        "Markdown"
    }
}

pub fn parse_markdown_document(content: &str) -> ParsedDocument {
    let metadata = DocumentMetadata::new(PathBuf::from(""), content.len() as u64)
        .with_line_count(content)
        .with_file_type(FileType::Markdown);
    let mut doc = ParsedDocument::new(metadata);

    let mut options = Options::empty();
    options.insert(Options::ENABLE_TABLES);
    options.insert(Options::ENABLE_TASKLISTS);
    options.insert(Options::ENABLE_STRIKETHROUGH);
    options.insert(Options::ENABLE_FOOTNOTES);
    // Without these, a leading `---` block parsed as a horizontal rule plus a
    // setext heading made of the metadata's last line.
    options.insert(Options::ENABLE_YAML_STYLE_METADATA_BLOCKS);
    options.insert(Options::ENABLE_PLUSES_DELIMITED_METADATA_BLOCKS);

    let parser = pulldown_cmark::Parser::new_ext(content, options);
    let events = parser.into_offset_iter();

    let lines = vecdb_common::LineCounter::new(content);
    // Every element is made from its byte range, and only from it
    // the span is narrowed to the element's own text —
    // pulldown-cmark's ranges carry the newline and blank lines after a block,
    // which made `line_end` the NEXT block's line and put `\n\n` in `content`
    // — and `content` and the lines are then read off it. Those
    // bytes are not lost: they become trivia below.
    let make = |t: ElementType, name: Option<String>, start: usize, end: usize| {
        let mut el =
            DocumentElement::new(t, name, String::new(), 1, 1).with_source_span(start, end);
        el.tighten_span(content);
        el.conform_to_span(content, &lines);
        el
    };

    // Link reference definitions (`[label]: url "title"`) never arrive as
    // events — pulldown-cmark resolves them in its first pass — so they were
    // silently absent from every consumer. Emitted as elements (MDAST
    // `Definition`), each with its exact span; sorted into place below.
    let mut definitions: Vec<DocumentElement> = events
        .reference_definitions()
        .iter()
        .map(|(label, def)| {
            let mut el = make(
                ElementType::Definition,
                Some(label.to_string()),
                def.span.start,
                def.span.end,
            )
            .with_attribute("url".to_string(), def.dest.to_string())
            .with_attribute(attr::BLOCK_DEPTH.to_string(), 0);
            if let Some(title) = &def.title {
                el = el.with_attribute("title".to_string(), title.to_string());
            }
            el
        })
        .collect();

    // State tracking
    let mut in_header = false;
    let mut header_level = 0;
    let mut header_start = 0;
    let mut header_text = String::new();
    let mut codeblock_lang = String::new();
    let mut codeblock_start = 0;
    // The code itself, as pulldown-cmark delivers it inside the block: no
    // fences, an indented block's indent removed. `None` outside a block.
    let mut codeblock_code: Option<String> = None;
    let mut in_paragraph = false;
    let mut paragraph_start = 0;
    let mut paragraph_text = String::new();
    // Container blocks NEST, so their starts are stacks. These were scalars, so
    // an inner list overwrote the outer list's start and the outer `list`
    // element's content began at the inner list — the outer list's first items
    // were silently missing from it.
    let mut blockquote_start: Vec<usize> = Vec::new();
    let mut blockquote_text = String::new();
    let mut list_start: Vec<(usize, bool)> = Vec::new();
    let mut table_start = 0;
    let mut in_footnote = false;

    // Inline style tracking
    // We need stacks because they can nest (e.g. bold inside italic)
    // BUT, vecq element model is somewhat flat for "find all bold spans".
    // We'll track the start positions.
    let mut emphasis_start: Vec<usize> = Vec::new();
    let mut strong_start: Vec<usize> = Vec::new();
    let mut strikethrough_start: Vec<usize> = Vec::new();

    // List Item tracking
    let mut item_start: Vec<usize> = Vec::new();
    // We need to track if we saw a task list marker for the *current* item
    let mut current_item_task_status: Option<bool> = None;

    // Footnotes
    let mut footnote_def_start = 0;
    let mut footnote_name = String::new();

    // Link state
    let mut in_link = false;
    let mut link_start = 0;
    let mut link_url = String::new();
    let mut link_title = String::new();
    let mut link_text = String::new();

    // How many container blocks (list, list item, blockquote, footnote
    // definition) enclose the current position. Recorded on every block element
    // as `block_depth`: elements here are FLAT siblings, so without it nothing
    // downstream can tell a list item from the list that already contains its
    // bytes, and vecdb embedded every nested block two or three times
    // `0` means the block is not inside another one.
    macro_rules! block_depth {
        () => {
            list_start.len() + item_start.len() + blockquote_start.len() + usize::from(in_footnote)
        };
    }

    for (event, range) in events {
        match event {
            // --- Structural Blocks ---
            Event::Start(Tag::Heading { level, .. }) => {
                in_header = true;
                header_level = match level {
                    HeadingLevel::H1 => 1,
                    HeadingLevel::H2 => 2,
                    HeadingLevel::H3 => 3,
                    HeadingLevel::H4 => 4,
                    HeadingLevel::H5 => 5,
                    HeadingLevel::H6 => 6,
                };
                header_start = range.start;
                header_text.clear();
            }
            Event::End(TagEnd::Heading(_)) => {
                in_header = false;
                let element = make(
                    ElementType::Header,
                    Some(header_text.trim().to_string()),
                    header_start,
                    range.end,
                )
                .with_attribute("level".to_string(), header_level)
                .with_attribute(attr::BLOCK_DEPTH.to_string(), block_depth!());

                doc = doc.add_element(element);
            }
            Event::Start(Tag::Paragraph) => {
                in_paragraph = true;
                paragraph_start = range.start;
                paragraph_text.clear();
            }
            Event::End(TagEnd::Paragraph) => {
                in_paragraph = false;
                // `content` is the paragraph AS WRITTEN, like every other
                // block: `failCgroupV1: false` must not become
                // plain prose for an embedder, and an editor splicing `content`
                // back must not strip the markup. The rendered prose — markers,
                // backticks and link URLs gone — was `content` until 2026-314;
                // it stays available as the `text` attribute, which is what a
                // query for prose wants.
                let element = make(ElementType::Paragraph, None, paragraph_start, range.end)
                    .with_attribute(attr::BLOCK_DEPTH.to_string(), block_depth!())
                    .with_attribute(attr::TEXT.to_string(), paragraph_text.trim());

                doc = doc.add_element(element);
            }
            Event::Start(Tag::BlockQuote(_)) => {
                blockquote_start.push(range.start);
                blockquote_text.clear();
            }
            Event::End(TagEnd::BlockQuote(_)) => {
                let start = blockquote_start.pop().unwrap_or(range.start);
                let element = make(ElementType::Blockquote, None, start, range.end)
                    .with_attribute(attr::BLOCK_DEPTH.to_string(), block_depth!());

                doc = doc.add_element(element);
            }

            // --- Lists & Items ---
            Event::Start(Tag::List(first_item)) => {
                list_start.push((range.start, first_item.is_some()));
            }
            Event::End(TagEnd::List(_)) => {
                let (start, ordered) = list_start.pop().unwrap_or((range.start, false));
                let element = make(ElementType::List, None, start, range.end)
                    .with_attribute("ordered".to_string(), ordered)
                    .with_attribute(attr::BLOCK_DEPTH.to_string(), block_depth!());

                doc = doc.add_element(element);
            }
            Event::Start(Tag::Item) => {
                item_start.push(range.start);
                current_item_task_status = None;
            }
            Event::End(TagEnd::Item) => {
                if let Some(start) = item_start.pop() {
                    let mut element = make(ElementType::ListItem, None, start, range.end)
                        .with_attribute(attr::BLOCK_DEPTH.to_string(), block_depth!());

                    if let Some(checked) = current_item_task_status {
                        element = element
                            .with_attribute("task".to_string(), true)
                            .with_attribute("checked".to_string(), checked);
                    }

                    doc = doc.add_element(element);
                }
            }
            Event::TaskListMarker(checked) => {
                current_item_task_status = Some(checked);
            }

            // --- Tables ---
            Event::Start(Tag::Table(_)) => {
                table_start = range.start;
            }
            Event::End(TagEnd::Table) => {
                let element = make(ElementType::Table, None, table_start, range.end)
                    .with_attribute(attr::BLOCK_DEPTH.to_string(), block_depth!());

                doc = doc.add_element(element);
            }

            // --- Raw HTML blocks and front matter ---
            //
            // Neither was emitted before, so `<details>` sections, HTML
            // comments and a file's metadata block silently vanished from
            // every consumer. An End event carries the whole block's range.
            Event::End(TagEnd::HtmlBlock) => {
                let element = make(ElementType::HtmlBlock, None, range.start, range.end)
                    .with_attribute(attr::BLOCK_DEPTH.to_string(), block_depth!());
                doc = doc.add_element(element);
            }
            Event::End(TagEnd::MetadataBlock(kind)) => {
                let format = match kind {
                    pulldown_cmark::MetadataBlockKind::YamlStyle => "yaml",
                    pulldown_cmark::MetadataBlockKind::PlusesStyle => "toml",
                };
                let element = make(ElementType::FrontMatter, None, range.start, range.end)
                    .with_attribute("format".to_string(), format)
                    .with_attribute(attr::BLOCK_DEPTH.to_string(), block_depth!());
                doc = doc.add_element(element);
            }

            // --- Media & Formatting ---
            //
            // An image's and a link's `content` is the markup as written
            // (`![alt](src)`, `[text](url)`), like every element's; the
            // destination — which WAS `content` until 2026-314 — is the `url`
            // attribute.
            Event::Start(Tag::Image {
                dest_url, title, ..
            }) => {
                let element = make(
                    ElementType::Image,
                    Some(title.to_string()),
                    range.start,
                    range.end,
                )
                .with_attribute(attr::URL.to_string(), dest_url.to_string());

                doc = doc.add_element(element);
            }
            Event::Start(Tag::Link {
                dest_url, title, ..
            }) => {
                in_link = true;
                link_start = range.start;
                link_url = dest_url.to_string();
                link_title = title.to_string();
                link_text.clear();
            }
            Event::End(TagEnd::Link) => {
                in_link = false;
                let mut element = make(
                    ElementType::Link,
                    Some(link_text.trim().to_string()),
                    link_start,
                    range.end,
                )
                .with_attribute(attr::URL.to_string(), link_url.clone());

                if !link_title.is_empty() {
                    element = element.with_attribute("title".to_string(), link_title.clone());
                }

                doc = doc.add_element(element);
            }

            // --- Styles: Emphasis, Strong, Strikethrough ---
            Event::Start(Tag::Emphasis) => {
                emphasis_start.push(range.start);
            }
            Event::End(TagEnd::Emphasis) => {
                if let Some(start) = emphasis_start.pop() {
                    // pulldown_cmark's range includes the markers; the span is
                    // the markup as written, markers and all.
                    let element = make(ElementType::Emphasis, None, start, range.end);
                    doc = doc.add_element(element);
                }
            }
            Event::Start(Tag::Strong) => {
                strong_start.push(range.start);
            }
            Event::End(TagEnd::Strong) => {
                if let Some(start) = strong_start.pop() {
                    let element = make(ElementType::Strong, None, start, range.end);
                    doc = doc.add_element(element);
                }
            }
            Event::Start(Tag::Strikethrough) => {
                strikethrough_start.push(range.start);
            }
            Event::End(TagEnd::Strikethrough) => {
                if let Some(start) = strikethrough_start.pop() {
                    let element = make(ElementType::Strikethrough, None, start, range.end);
                    doc = doc.add_element(element);
                }
            }

            // --- Footnotes ---
            Event::Start(Tag::FootnoteDefinition(name)) => {
                footnote_def_start = range.start;
                footnote_name = name.to_string();
                in_footnote = true;
            }
            Event::End(TagEnd::FootnoteDefinition) => {
                in_footnote = false;
                let element = make(
                    ElementType::FootnoteDefinition,
                    Some(footnote_name.clone()),
                    footnote_def_start,
                    range.end,
                )
                .with_attribute(attr::BLOCK_DEPTH.to_string(), block_depth!());
                doc = doc.add_element(element);
            }

            // --- Misc ---
            Event::Rule => {
                let element = make(ElementType::HorizontalRule, None, range.start, range.end)
                    .with_attribute(attr::BLOCK_DEPTH.to_string(), block_depth!());

                doc = doc.add_element(element);
            }
            Event::Text(text) => {
                if let Some(code) = codeblock_code.as_mut() {
                    code.push_str(&text);
                }
                if in_link {
                    link_text.push_str(&text);
                }

                if in_header {
                    header_text.push_str(&text);
                } else if in_paragraph {
                    paragraph_text.push_str(&text);
                } else if !blockquote_start.is_empty() {
                    blockquote_text.push_str(&text);
                }
            }
            Event::Code(text) => {
                let code_text = text.to_string();

                if in_link {
                    link_text.push_str(&code_text);
                }

                if in_header {
                    header_text.push_str(&code_text);
                } else if in_paragraph {
                    paragraph_text.push_str(&code_text);
                } else if !blockquote_start.is_empty() {
                    blockquote_text.push_str(&code_text);
                }
            }
            Event::SoftBreak | Event::HardBreak => {
                let separator = " ";
                if in_link {
                    link_text.push_str(separator);
                }
                if in_header {
                    header_text.push_str(separator);
                } else if in_paragraph {
                    paragraph_text.push_str(separator);
                } else if !blockquote_start.is_empty() {
                    blockquote_text.push_str(separator);
                }
            }
            Event::Start(Tag::CodeBlock(kind)) => {
                codeblock_start = range.start;
                codeblock_code = Some(String::new());
                codeblock_lang = match kind {
                    pulldown_cmark::CodeBlockKind::Fenced(l) => l.to_string(),
                    pulldown_cmark::CodeBlockKind::Indented => "indented".to_string(),
                };
            }
            Event::End(TagEnd::CodeBlock) => {
                let element = make(ElementType::CodeBlock, None, codeblock_start, range.end)
                    .with_attribute("language".to_string(), codeblock_lang.clone())
                    .with_attribute(
                        attr::CODE.to_string(),
                        codeblock_code.take().unwrap_or_default(),
                    )
                    .with_attribute(attr::BLOCK_DEPTH.to_string(), block_depth!());

                doc = doc.add_element(element);
            }

            // ---- Every remaining event, named: no catch-all. ----
            // A pulldown-cmark upgrade that adds a variant fails to compile here
            // instead of being dropped. Each arm says where its text goes.
            //
            // Emitted at the other end of the pair (the element is built there):
            Event::Start(Tag::HtmlBlock)
            | Event::Start(Tag::MetadataBlock(_))
            | Event::End(TagEnd::Image) => {}
            // Inside an enclosing block whose element is its source slice
            // (table → rows → cells; HTML block → its HTML lines):
            Event::Start(Tag::TableHead | Tag::TableRow | Tag::TableCell)
            | Event::End(TagEnd::TableHead | TagEnd::TableRow | TagEnd::TableCell)
            | Event::Html(_) => {}
            // Inline, inside a paragraph / heading / list item whose element is
            // its source slice (a paragraph is embedded as written):
            Event::InlineHtml(_)
            | Event::FootnoteReference(_)
            | Event::InlineMath(_)
            | Event::DisplayMath(_) => {}
            // Extensions NOT enabled in `options` above — never produced. If one
            // is ever enabled, its content still lies inside the enclosing
            // block's slice, and the reconstruction test would show a gap.
            Event::Start(
                Tag::DefinitionList
                | Tag::DefinitionListTitle
                | Tag::DefinitionListDefinition
                | Tag::Superscript
                | Tag::Subscript,
            )
            | Event::End(
                TagEnd::DefinitionList
                | TagEnd::DefinitionListTitle
                | TagEnd::DefinitionListDefinition
                | TagEnd::Superscript
                | TagEnd::Subscript,
            ) => {}
        }
    }

    // Definitions into document order with everything else.
    doc.elements.append(&mut definitions);
    doc.elements.sort_by_key(|e| e.line_start);

    // Lossless: every byte between depth-0 blocks — blank lines, and anything
    // else — becomes trivia owned by a block, by the one shared rule. Markdown
    // has no comment syntax of its own (an HTML comment is an `html_block`
    // element), so nothing is trivia but whitespace; any other text left
    // between blocks is a construct this parser does not emit, and the
    // safeguard surfaces it rather than hiding it.
    let mut elements = std::mem::take(&mut doc.elements);
    super::ownership::assign_trivia(&mut elements, content, &lines, &[], is_top_block);
    stamp_heading_paths(&mut elements);
    doc.elements = elements;
    doc
}

/// Give every element its `heading_path` (`attr::HEADING_PATH`): the names of
/// the depth-0 headings whose sections contain it, by byte position. A heading
/// pops every open heading of its level or deeper, exactly as the chunker
/// builds `parent_trail`, so the two never disagree.
fn stamp_heading_paths(elements: &mut [DocumentElement]) {
    let span = |e: &DocumentElement| e.source_span().unwrap_or((0, 0));
    let start = |e: &DocumentElement| span(e).0;
    // (start, end, ancestors, ancestors + itself), in document order.
    let mut headings: Vec<(usize, usize, Vec<String>, Vec<String>)> = Vec::new();
    let mut stack: Vec<(u64, String)> = Vec::new();
    let mut order: Vec<&DocumentElement> = elements
        .iter()
        .filter(|e| e.element_type == ElementType::Header && is_top_block(e))
        .collect();
    order.sort_by_key(|e| start(e));
    for h in order {
        let level = h
            .attributes
            .get("level")
            .and_then(|v| v.as_u64())
            .unwrap_or(1);
        while stack.last().is_some_and(|(l, _)| *l >= level) {
            stack.pop();
        }
        let ancestors: Vec<String> = stack.iter().map(|(_, n)| n.clone()).collect();
        stack.push((level, h.name.clone().unwrap_or_default()));
        let inside = stack.iter().map(|(_, n)| n.clone()).collect();
        headings.push((start(h), span(h).1, ancestors, inside));
    }
    for e in elements.iter_mut() {
        let at = start(e);
        // An element inside a heading's own bytes (the heading, or an inline
        // span of its text) lies under the heading's ancestors, not under it.
        let path = match headings.iter().rposition(|(s, _, _, _)| *s <= at) {
            Some(i) if at < headings[i].1 => headings[i].2.clone(),
            Some(i) => headings[i].3.clone(),
            None => Vec::new(),
        };
        e.attributes
            .insert_generic(attr::HEADING_PATH.to_string(), serde_json::json!(path));
    }
}

/// The blocks that own Markdown trivia: depth-0 block elements. Inline spans
/// (emphasis, links, …) live inside a paragraph's slice; nested blocks inside
/// their container's.
pub(crate) fn is_top_block(e: &DocumentElement) -> bool {
    let depth = e
        .attributes
        .get(attr::BLOCK_DEPTH)
        .and_then(|v| v.as_u64())
        .unwrap_or(0);
    depth == 0
        && !matches!(
            e.element_type,
            ElementType::Emphasis
                | ElementType::Strong
                | ElementType::Strikethrough
                | ElementType::Link
                | ElementType::Image
        )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse_md(content: &str) -> ParsedDocument {
        parse_markdown_document(content)
    }

    #[test]
    fn test_header_parsing() {
        let doc = parse_md("# H1\n## H2\n### H3");
        let headers: Vec<_> = doc
            .elements
            .iter()
            .filter(|e| e.element_type == ElementType::Header)
            .collect();
        assert_eq!(headers.len(), 3);
        assert_eq!(headers[0].name, Some("H1".to_string()));
        assert_eq!(headers[1].name, Some("H2".to_string()));
        assert_eq!(headers[2].name, Some("H3".to_string()));
    }

    #[test]
    fn test_paragraph_parsing() {
        let doc = parse_md("# Header\n\nThis is a paragraph.");
        let paragraphs: Vec<_> = doc
            .elements
            .iter()
            .filter(|e| e.element_type == ElementType::Paragraph)
            .collect();
        assert_eq!(paragraphs.len(), 1);
        assert_eq!(paragraphs[0].content, "This is a paragraph.");
    }

    #[test]
    fn test_blockquote_parsing() {
        let doc = parse_md("> Quoted text");
        let blockquotes: Vec<_> = doc
            .elements
            .iter()
            .filter(|e| e.element_type == ElementType::Blockquote)
            .collect();
        assert_eq!(blockquotes.len(), 1);
        assert!(blockquotes[0].content.contains("Quoted text"));
    }

    #[test]
    fn test_list_parsing() {
        let doc = parse_md("- Item 1\n- Item 2");
        let lists: Vec<_> = doc
            .elements
            .iter()
            .filter(|e| e.element_type == ElementType::List)
            .collect();
        assert_eq!(lists.len(), 1);
        assert!(lists[0].content.contains("Item 1"));
    }

    /// REGRESSION: a nested list must not truncate its parent.
    ///
    /// The list start was one scalar, so the inner `Start(List)` overwrote it
    /// and the outer list's content began at `- nested`, losing `- outer one`.
    #[test]
    fn nested_list_keeps_outer_content() {
        let doc = parse_md("- outer one\n- outer two\n  - nested\n");
        let lists: Vec<_> = doc
            .elements
            .iter()
            .filter(|e| e.element_type == ElementType::List)
            .collect();
        assert_eq!(lists.len(), 2);
        // The outer list is the one nothing encloses — chosen by depth, not by
        // position: elements are in document order since 2026-300 (the outer
        // list used to be emitted last, at its End event).
        let outer = lists
            .iter()
            .find(|e| e.attributes.get(attr::BLOCK_DEPTH).and_then(|v| v.as_u64()) == Some(0))
            .expect("an outer list");
        assert!(
            outer.content.starts_with("- outer one"),
            "outer list content lost its first item: {:?}",
            outer.content
        );
        assert_eq!(outer.line_start, 1);
    }

    /// `block_depth` is what lets a consumer keep each byte once: 0 for a block
    /// that nothing encloses, more for every enclosing list, item, blockquote
    /// or footnote.
    #[test]
    fn block_depth_counts_enclosing_containers() {
        let doc = parse_md("# H\n\n- item\n\n  para in item\n\n> quoted\n\nplain\n");
        let depth =
            |e: &DocumentElement| e.attributes.get(attr::BLOCK_DEPTH).and_then(|v| v.as_u64());
        for e in &doc.elements {
            let expected = match (e.element_type, e.content.as_str()) {
                (ElementType::Header, _)
                | (ElementType::List, _)
                | (ElementType::Blockquote, _) => 0,
                (ElementType::Paragraph, "plain") => 0,
                (ElementType::ListItem, _) | (ElementType::Paragraph, "quoted") => 1,
                (ElementType::Paragraph, _) => 2,
                _ => continue,
            };
            assert_eq!(
                depth(e),
                Some(expected),
                "{:?} {:?}",
                e.element_type,
                e.content
            );
        }
    }

    #[test]
    fn paragraph_source_span_is_the_text_as_written() {
        // A list interrupting a paragraph with no blank line: the case where a
        // line-number slice would wrongly take the list's first line too.
        let src = "Set `failCgroupV1: false` **before** [upgrading](https://k8s.io/x).\n- next\n";
        let doc = parse_md(src);
        let p = doc
            .elements
            .iter()
            .find(|e| e.element_type == ElementType::Paragraph)
            .unwrap();
        // Since `content` IS the source text of the span — no
        // trailing newline, no list line — and the rendered prose that was
        // `content` before is the `text` attribute.
        let written = "Set `failCgroupV1: false` **before** [upgrading](https://k8s.io/x).";
        assert_eq!(
            p.source_text(src).expect("paragraph carries its span"),
            written
        );
        assert_eq!(p.content, written);
        assert_eq!(
            p.attributes.get(attr::TEXT).and_then(|v| v.as_str()),
            Some("Set failCgroupV1: false before upgrading.")
        );
    }

    #[test]
    fn html_blocks_and_front_matter_are_emitted() {
        let src = "---\ntitle: Eras\nversion: 1.35\n---\n\n# Eras\n\n<details>\n<summary>Why</summary>\n\nhidden reason\n\n</details>\n\n<!-- reviewer: keep -->\n";
        let doc = parse_md(src);
        let of = |t: ElementType| -> Vec<&DocumentElement> {
            doc.elements
                .iter()
                .filter(|e| e.element_type == t)
                .collect()
        };
        let fm = of(ElementType::FrontMatter);
        assert_eq!(fm.len(), 1);
        assert!(
            fm[0].content.contains("version: 1.35"),
            "{:?}",
            fm[0].content
        );
        // Previously the metadata's last line became a setext heading.
        let headers: Vec<_> = of(ElementType::Header)
            .iter()
            .map(|e| e.content.trim())
            .collect();
        assert_eq!(headers, ["# Eras"]);
        let html: Vec<_> = of(ElementType::HtmlBlock)
            .iter()
            .map(|e| e.content.trim())
            .collect();
        assert!(html.iter().any(|h| h.starts_with("<details>")), "{html:?}");
        assert!(html.contains(&"<!-- reviewer: keep -->"), "{html:?}");
    }

    #[test]
    fn test_horizontal_rule_parsing() {
        let doc = parse_md("---\n***\n___");
        let rules: Vec<_> = doc
            .elements
            .iter()
            .filter(|e| e.element_type == ElementType::HorizontalRule)
            .collect();
        assert_eq!(rules.len(), 3);
    }

    #[test]
    fn test_3kb_fixture() {
        let path = std::path::Path::new("tests/fixtures/perf_3kb.md");
        if !path.exists() {
            return;
        }
        let content = std::fs::read_to_string(path).unwrap();
        let doc = parse_markdown_document(&content);
        assert!(!doc.elements.is_empty());
        // Markdown should have headers or paragraphs
        assert!(doc
            .elements
            .iter()
            .any(|e| e.element_type == ElementType::Header));
    }
    /// REGRESSION (2026-314): since Markdown became "as written" (`a38c660`) a
    /// code block's `content` carries its fences, so the documented recipe
    /// "extract bash code: `.code_blocks[] | .content -r`" printed the ```
    /// lines too. `attributes.code` is the code alone — fenced, indented, and
    /// inside a list item (indent removed) alike.
    #[test]
    fn code_block_carries_its_code_without_fences() {
        let src = "```bash\nmake\nmake test\n```\n\n    indented()\n\n- item\n\n  ```py\n  x = 1\n  ```\n";
        let doc = parse_md(src);
        let code: Vec<(String, String)> = doc
            .elements
            .iter()
            .filter(|e| e.element_type == ElementType::CodeBlock)
            .map(|e| {
                (
                    e.content.clone(),
                    e.attributes
                        .get(attr::CODE)
                        .and_then(|v| v.as_str())
                        .unwrap()
                        .to_string(),
                )
            })
            .collect();
        assert_eq!(
            code,
            [
                (
                    "```bash\nmake\nmake test\n```".to_string(),
                    "make\nmake test\n".to_string()
                ),
                // The span starts at the code; the indent is leading trivia.
                ("indented()".to_string(), "indented()\n".to_string()),
                ("```py\n  x = 1\n  ```".to_string(), "x = 1\n".to_string()),
            ]
        );
    }

    #[test]
    fn test_code_block_parsing() {
        let doc = parse_md("```rust\nfn main() {}\n```");
        let blocks: Vec<_> = doc
            .elements
            .iter()
            .filter(|e| e.element_type == ElementType::CodeBlock)
            .collect();
        assert_eq!(blocks.len(), 1);
        assert_eq!(
            blocks[0]
                .attributes
                .get("language")
                .and_then(|v| v.as_str()),
            Some("rust")
        );
    }

    #[test]
    fn test_link_parsing() {
        let doc = parse_md("[Link Text](https://example.com)");
        let links: Vec<_> = doc
            .elements
            .iter()
            .filter(|e| e.element_type == ElementType::Link)
            .collect();
        assert_eq!(links.len(), 1);
        // `content` is the link as written; the destination,
        // which was `content` before, is the `url` attribute.
        assert_eq!(links[0].content, "[Link Text](https://example.com)");
        assert_eq!(links[0].name.as_deref(), Some("Link Text"));
        assert_eq!(
            links[0].attributes.get(attr::URL).and_then(|v| v.as_str()),
            Some("https://example.com")
        );
    }

    #[test]
    fn test_image_parsing() {
        let doc = parse_md("![Alt](image.png)"); // path-ok: markdown sample
        let images: Vec<_> = doc
            .elements
            .iter()
            .filter(|e| e.element_type == ElementType::Image)
            .collect();
        assert_eq!(images.len(), 1);
        // As for links: the markup as written, the destination as `url`.
        assert_eq!(images[0].content, "![Alt](image.png)"); // path-ok: sample Markdown
        assert_eq!(
            images[0].attributes.get(attr::URL).and_then(|v| v.as_str()),
            Some("image.png")
        );
    }

    #[test]
    fn test_table_parsing() {
        let doc = parse_md("| A | B |\n|---|---|\n| 1 | 2 |");
        let tables: Vec<_> = doc
            .elements
            .iter()
            .filter(|e| e.element_type == ElementType::Table)
            .collect();
        assert_eq!(tables.len(), 1);
    }

    #[test]
    fn test_antigravity_chat_format() {
        // Simulates the format from antigravity_chat.jq
        let content = "### 2024-01-01T12:00:00Z\n\nMessage content here.\n\n---\n\n### 2024-01-01T12:01:00Z\n\nAnother message.\n\n---";
        let doc = parse_md(content);

        let headers: Vec<_> = doc
            .elements
            .iter()
            .filter(|e| e.element_type == ElementType::Header)
            .collect();
        let paragraphs: Vec<_> = doc
            .elements
            .iter()
            .filter(|e| e.element_type == ElementType::Paragraph)
            .collect();
        let rules: Vec<_> = doc
            .elements
            .iter()
            .filter(|e| e.element_type == ElementType::HorizontalRule)
            .collect();

        assert_eq!(headers.len(), 2, "Should have 2 headers (timestamps)");
        assert_eq!(paragraphs.len(), 2, "Should have 2 paragraphs (messages)");
        assert_eq!(
            rules.len(),
            2,
            "Should have 2 horizontal rules (separators)"
        );
    }

    /// LOSSLESS: every Markdown fixture rebuilds byte for byte from its
    /// depth-0 blocks and their owned trivia. Any text the parser does not
    /// emit (link reference definitions once did not) makes this fail — or,
    /// between blocks, panics the safeguard.
    #[test]
    fn markdown_fixtures_reconstruct_byte_for_byte() {
        for (name, src) in [
            (
                "all_elements.md",
                include_str!("../../tests/fixtures/markdown/all_elements.md"),
            ),
            (
                "inline_elements.md",
                include_str!("../../tests/fixtures/markdown/inline_elements.md"),
            ),
            (
                "front_matter_and_html.md",
                include_str!("../../tests/fixtures/markdown/front_matter_and_html.md"),
            ),
        ] {
            let doc = parse_md(src);
            crate::parsers::ownership::assert_reconstructs(name, &doc.elements, src, is_top_block);
        }
    }

    /// REGRESSION (Ecosystem Experts on `b79f78e`): "list item 1 under
    /// X/Y/Tasks" must be a path query, not a rebuild of section ownership
    /// from line numbers. Every element carries the headings it lies under;
    /// a heading — and an inline span of its text — carries its ancestors.
    #[test]
    fn every_element_carries_its_heading_path() {
        let doc = parse_md(
            "# X\n\n## Y\n\n### Tasks *now*\n\n- [ ] item one\n- [x] item two\n\n## Z\n\nzed para\n",
        );
        let path_of = |pred: &dyn Fn(&DocumentElement) -> bool| -> Vec<String> {
            let e = doc
                .elements
                .iter()
                .find(|e| pred(e))
                .expect("element present");
            serde_json::from_value(e.attributes.get(attr::HEADING_PATH).unwrap().clone()).unwrap()
        };
        let is = |t: ElementType, text: &'static str| {
            move |e: &DocumentElement| e.element_type == t && e.content.contains(text)
        };
        assert_eq!(
            path_of(&is(ElementType::Header, "# X")),
            Vec::<String>::new()
        );
        assert_eq!(path_of(&is(ElementType::Header, "Tasks")), ["X", "Y"]);
        assert_eq!(path_of(&is(ElementType::Emphasis, "now")), ["X", "Y"]);
        assert_eq!(
            path_of(&is(ElementType::List, "item one")),
            ["X", "Y", "Tasks now"]
        );
        assert_eq!(
            path_of(&is(ElementType::ListItem, "item one")),
            ["X", "Y", "Tasks now"]
        );
        assert_eq!(path_of(&is(ElementType::Paragraph, "zed")), ["X", "Z"]);
        assert!(
            doc.elements
                .iter()
                .all(|e| e.attributes.contains_key(attr::HEADING_PATH)),
            "every element carries a heading path"
        );
    }

    /// REGRESSION: a one-line item on line 8 reports 8–8, not
    /// 8–10, and its content is the item — the blank line after it is trivia.
    /// An editor replacing `line_start..=line_end` with new text deleted the
    /// paragraph after the item.
    #[test]
    fn line_end_is_inclusive_and_content_carries_no_line_break() {
        let src = include_str!("../../tests/fixtures/source_contract/probe.md");
        let doc = parse_md(src);
        let item = doc
            .elements
            .iter()
            .find(|e| e.element_type == ElementType::ListItem && e.content.contains("second"))
            .unwrap();
        assert_eq!(item.content, "- [x] second item");
        assert_eq!((item.line_start, item.line_end), (8, 8));
        assert_eq!(item.source_span(), Some((54, 71)));
        let title = &doc.elements[0];
        assert_eq!(
            (title.content.as_str(), title.line_start, title.line_end),
            ("# Title", 1, 1)
        );
        let last = src.lines().count();
        for e in &doc.elements {
            assert!(
                !e.content.ends_with('\n'),
                "{:?} ends in a newline",
                e.content
            );
            assert!(e.line_end <= last, "{:?} ends past the file", e.content);
        }
        crate::parsers::ownership::assert_reconstructs(
            "probe.md",
            &doc.elements,
            src,
            is_top_block,
        );
    }

    #[test]
    fn link_reference_definitions_are_emitted() {
        let src = "See [the guide][g].\n\n[g]: https://example.com/guide \"Guide\"\n";
        let doc = parse_md(src);
        let def = doc
            .elements
            .iter()
            .find(|e| e.element_type == ElementType::Definition)
            .expect("definition emitted");
        assert_eq!(def.content, "[g]: https://example.com/guide \"Guide\"");
        assert_eq!(
            def.attributes.get("url").and_then(|v| v.as_str()),
            Some("https://example.com/guide")
        );
        crate::parsers::ownership::assert_reconstructs("refdef", &doc.elements, src, is_top_block);
    }
}
