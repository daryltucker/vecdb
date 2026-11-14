// PURPOSE:
//   Parser for HTML (and the XML-ish agent files — `<task>`, `<mcp_servers>` —
//   that ride on the same `FileType`).
//
// WHY tree-sitter-html:
//   An XML reader is not an HTML parser: void tags (`<br>`), text nodes,
//   comments and the doctype fall to a catch-all, and an element's `content`
//   becomes its inner text with no exact span, so the file cannot be rebuilt
//   and a nested element is embedded on top of its whole parent (100%
//   duplication). tree-sitter-html parses real HTML, and the source-span
//   contract makes an exact span per element a requirement, not a preference.
//
// THE SHAPE:
//   - every `element` / `script_element` / `style_element` is an `HtmlElement`
//     named by its tag, spanning its whole markup (`<div class="x">…</div>`),
//     its attributes as `attributes`, its child elements as `children`;
//   - text inside an element is that element's own frame, never a second copy;
//   - at the top level, the doctype, loose text, entities and anything the
//     grammar could not parse are `Block`s carrying their grammar kind;
//   - comments are trivia, owned by the element they describe (`ownership`).
//   Every byte lands in exactly one element or its owned trivia.

use crate::error::{VecqError, VecqResult};
use crate::parser::Parser;
use crate::types::{
    attr, DocumentElement, DocumentMetadata, ElementAttributes, ElementType, FileType,
    HtmlAttributes, ParsedDocument,
};
use async_trait::async_trait;
use std::collections::HashMap;
use std::path::PathBuf;
use tree_sitter::Node;

pub struct HtmlParser;

impl Default for HtmlParser {
    fn default() -> Self {
        Self::new()
    }
}

impl HtmlParser {
    pub fn new() -> Self {
        Self
    }

    pub fn is_void_tag(tag: &str) -> bool {
        let tag = tag.to_lowercase();
        matches!(
            tag.as_str(),
            "area"
                | "base"
                | "br"
                | "col"
                | "embed"
                | "hr"
                | "img"
                | "input"
                | "link"
                | "meta"
                | "param"
                | "source"
                | "track"
                | "wbr"
        )
    }
}

#[async_trait]
impl Parser for HtmlParser {
    async fn parse(&self, content: &str) -> VecqResult<ParsedDocument> {
        let meta = DocumentMetadata::new(PathBuf::from("content.html"), content.len() as u64)
            .with_file_type(FileType::Html)
            .with_line_count(content);
        let mut document = ParsedDocument::new(meta);
        parse_html_internal(&mut document, content)?;
        Ok(document)
    }

    fn file_extensions(&self) -> &[&str] {
        &["html", "htm", "xml", "xhtml"]
    }

    fn language_name(&self) -> &str {
        "HTML"
    }
}

/// Every node kind the grammar can produce directly in the document and in an
/// element, classified — the completeness test (`ts_coverage`) checks this
/// against the grammar's own NODE_TYPES, so a grammar bump that adds a kind
/// fails naming it. `Unit` kinds are elements; `Descend` is the element
/// structure itself (start/end tags are its frame); `Comment` is trivia.
// Read by the completeness test only: this parser walks the tree itself.
#[cfg_attr(not(test), allow(dead_code))]
pub(crate) const COVERAGE: crate::parsers::ts_coverage::Spec = {
    use crate::parsers::ts_coverage::Kind::{Comment, Descend, Unit};
    use crate::types::ElementType as E;
    crate::parsers::ts_coverage::Spec {
        bodies: &["document", "element"],
        kinds: &[
            ("comment", Comment),
            ("doctype", Unit(E::Block)),
            ("element", Unit(E::HtmlElement)),
            ("end_tag", Descend),
            ("entity", Unit(E::Block)),
            ("erroneous_end_tag", Unit(E::Block)),
            ("script_element", Unit(E::HtmlElement)),
            ("self_closing_tag", Descend),
            ("start_tag", Descend),
            ("style_element", Unit(E::HtmlElement)),
            ("text", Unit(E::Block)),
        ],
    }
};

// Structural constraints (D026), kept from the quick-xml parser: a file can
// still be a bomb even when the grammar is sound.
const MAX_STRUCTURAL_DEPTH: usize = 200;
const MAX_ELEMENTS_PER_FILE: usize = 100000;
/// Fail if 100 `<div><div>` in a row with no attributes or content.
const CONSECUTIVE_TAG_LIMIT: usize = 100;

fn is_element(kind: &str) -> bool {
    matches!(kind, "element" | "script_element" | "style_element")
}

/// The tag name and attributes from an element's start (or self-closing) tag.
fn tag_of(node: Node, source: &str) -> (Option<String>, HashMap<String, serde_json::Value>) {
    let text = |n: Node| {
        n.utf8_text(source.as_bytes())
            .unwrap_or_default()
            .to_string()
    };
    let mut cursor = node.walk();
    let tag = node
        .children(&mut cursor)
        .find(|c| matches!(c.kind(), "start_tag" | "self_closing_tag"));
    let mut name = None;
    let mut attributes = HashMap::new();
    if let Some(tag) = tag {
        // The name as written: `<` up to whitespace, `/` or `>`. The grammar's
        // tag-name token stops at `_`, so `<mcp_servers>` came back as the tag
        // `mcp` with an attribute `_servers` — and underscore tags are exactly
        // what agent files (`<mcp_servers>`, `<task_list>`) use.
        let raw = text(tag);
        let written: String = raw
            .trim_start_matches('<')
            .chars()
            .take_while(|c| !c.is_whitespace() && *c != '/' && *c != '>')
            .collect();
        let name_end = tag.start_byte() + 1 + written.len();
        name = (!written.is_empty()).then_some(written);
        let mut c = tag.walk();
        for part in tag.children(&mut c) {
            match part.kind() {
                "tag_name" => {}
                // Pieces of the name the grammar split off are not attributes.
                "attribute" if part.start_byte() < name_end => {}
                "attribute" => {
                    let mut a = part.walk();
                    let mut key = None;
                    let mut value = String::new();
                    for piece in part.children(&mut a) {
                        match piece.kind() {
                            "attribute_name" => key = Some(text(piece)),
                            "attribute_value" => value = text(piece),
                            "quoted_attribute_value" => {
                                let mut q = piece.walk();
                                value = piece
                                    .children(&mut q)
                                    .find(|v| v.kind() == "attribute_value")
                                    .map(text)
                                    .unwrap_or_default();
                            }
                            _ => {}
                        }
                    }
                    if let Some(k) = key {
                        attributes.insert(k, serde_json::Value::String(value));
                    }
                }
                _ => {}
            }
        }
    }
    (name, attributes)
}

struct Walk<'s> {
    source: &'s str,
    count: usize,
}

impl Walk<'_> {
    /// One element, and its child elements. `parent` is what the D026 nesting
    /// check needs from the element above: its tag, whether it has
    /// attributes, whether this is its only child, and the length of the run
    /// of identical bare tags so far.
    fn element(
        &mut self,
        node: Node,
        depth: usize,
        parent: Option<(&Option<String>, bool, usize)>,
    ) -> VecqResult<DocumentElement> {
        self.count += 1;
        if self.count > MAX_ELEMENTS_PER_FILE {
            return Err(VecqError::CircuitBreakerTriggered {
                message: format!(
                    "File exceeded maximum element limit of {}",
                    MAX_ELEMENTS_PER_FILE
                ),
            });
        }
        let (name, attributes) = tag_of(node, self.source);
        // D026: consecutive identical, attribute-less tags nested with nothing
        // else around them — a nesting bomb.
        let chain = match parent {
            Some((parent_name, bare_only_child, run))
                if bare_only_child
                    && name.is_some()
                    && *parent_name == name
                    && attributes.is_empty() =>
            {
                run + 1
            }
            _ => 0,
        };
        if chain > CONSECUTIVE_TAG_LIMIT {
            return Err(VecqError::CircuitBreakerTriggered {
                message: format!(
                    "Structural Error: Consecutive open tags of type '{}' without content or attributes. Rejecting potential bomb.",
                    name.unwrap_or_default()
                ),
            });
        }
        let mut el =
            DocumentElement::new(ElementType::HtmlElement, name.clone(), String::new(), 1, 1)
                .with_source_span(node.start_byte(), node.end_byte());
        // Below the depth limit an element is still one element with its exact
        // span; its descendants simply stay in its text rather than becoming
        // elements of their own.
        if depth < MAX_STRUCTURAL_DEPTH {
            let mut cursor = node.walk();
            let content: Vec<Node> = node
                .named_children(&mut cursor)
                .filter(|n| !matches!(n.kind(), "start_tag" | "end_tag"))
                .collect();
            let bare_only_child = attributes.is_empty() && content.len() == 1;
            for kid in content.into_iter().filter(|c| is_element(c.kind())) {
                let child = self.element(kid, depth + 1, Some((&name, bare_only_child, chain)))?;
                el.children.push(child);
            }
        }
        Ok(el.set_attributes(ElementAttributes::Html(HtmlAttributes {
            other: attributes,
        })))
    }
}

/// Byte spans of every comment node, in order.
fn comment_spans(root: Node) -> Vec<(usize, usize)> {
    let mut out = Vec::new();
    let mut stack = vec![root];
    while let Some(n) = stack.pop() {
        if n.kind() == "comment" {
            out.push((n.start_byte(), n.end_byte()));
            continue;
        }
        let mut c = n.walk();
        stack.extend(n.children(&mut c));
    }
    out.sort_unstable();
    out
}

fn parse_html_internal(document: &mut ParsedDocument, content: &str) -> VecqResult<()> {
    let mut parser = tree_sitter::Parser::new();
    parser
        .set_language(&tree_sitter_html::LANGUAGE.into())
        .map_err(|e| VecqError::ConfigError {
            message: format!("tree-sitter-html: {e}"),
        })?;
    let Some(tree) = parser.parse(content, None) else {
        return Err(VecqError::parse_error(
            document.metadata.path.clone(),
            0,
            "tree-sitter-html produced no tree".to_string(),
            None::<std::io::Error>,
        ));
    };
    let root = tree.root_node();
    let mut walk = Walk {
        source: content,
        count: 0,
    };
    let mut elements = Vec::new();
    let mut cursor = root.walk();
    for child in root.named_children(&mut cursor) {
        match child.kind() {
            k if is_element(k) => elements.push(walk.element(child, 0, None)?),
            "comment" => {}
            // doctype, loose text, entities, an end tag with no element, and
            // whatever the grammar could not parse (`ERROR`, which counts as a
            // syntax error): kept, as what the grammar says they are.
            kind => {
                let el = DocumentElement::new(ElementType::Block, None, String::new(), 1, 1)
                    .with_source_span(child.start_byte(), child.end_byte())
                    .with_attribute(attr::KIND.to_string(), kind);
                elements.push(el);
            }
        }
    }

    let lines = vecdb_common::LineCounter::new(content);
    for e in elements.iter_mut() {
        e.tighten_span(content);
    }
    let comments = comment_spans(root);
    super::ownership::assign_trivia(&mut elements, content, &lines, &comments, |_| true);
    for e in elements.iter_mut() {
        super::ownership::assign_nested_trivia(e, content, &comments);
        // `content` and lines read off the span here, not only in
        // `vecq::parse_file`: a bare `Parser::parse` result (the enricher's
        // input) must state what its spans say too.
        e.conform_to_span(content, &lines);
    }
    document.metadata.syntax_errors = super::ts_coverage::syntax_errors(root);
    document.elements = elements;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::{DocumentMetadata, FileType};
    use std::path::PathBuf;

    fn parse_html(content: &str) -> ParsedDocument {
        let meta = DocumentMetadata::new(PathBuf::from("test.html"), content.len() as u64)
            .with_file_type(FileType::Html);
        let mut doc = ParsedDocument::new(meta);
        parse_html_internal(&mut doc, content).unwrap();
        // Finished as `vecq::parse_file` finishes it: `content` and lines are
        // read off each element's span.
        doc.with_source(content)
    }

    #[test]
    fn test_simple_element() {
        let doc = parse_html("<div class=\"test\">Hello</div>");
        assert_eq!(doc.elements.len(), 1);
        let el = &doc.elements[0];
        assert_eq!(el.name.as_deref(), Some("div"));
        // The element as written; it was the inner text
        // `Hello` until 2026-314, which no span could reproduce.
        assert_eq!(el.content, "<div class=\"test\">Hello</div>");
        assert_eq!(el.attributes.get_text("class").unwrap(), "test");
    }

    #[test]
    fn test_nested_elements() {
        let doc = parse_html("<outer><inner>Text</inner></outer>");
        assert_eq!(doc.elements.len(), 1);
        let outer = &doc.elements[0];
        assert_eq!(outer.name.as_deref(), Some("outer"));
        assert_eq!(outer.children.len(), 1);
        let inner = &outer.children[0];
        assert_eq!(inner.name.as_deref(), Some("inner"));
        assert_eq!(inner.content, "<inner>Text</inner>");
        // The outer element's own text — its frame — is its tags alone: the
        // inner element is never embedded twice.
        let src = "<outer><inner>Text</inner></outer>";
        assert_eq!(outer.frame(src).unwrap(), "<outer></outer>");
    }

    #[test]
    fn test_mixed_content() {
        let html = "<section>\n## Header\n</section>";
        let doc = parse_html(html);
        let el = &doc.elements[0];
        assert!(el.content.contains("## Header"));
    }

    #[test]
    fn test_custom_tags() {
        let doc = parse_html("<mcp_servers>List</mcp_servers>");
        assert_eq!(doc.elements[0].name.as_deref(), Some("mcp_servers"));
    }

    #[test]
    fn test_recursion_safety_performance() {
        let mut deep_html = "Text content\n".to_string();
        for i in 0..150 {
            deep_html = format!("<div id=\"{}\">{}</div>", i, deep_html);
        }

        let start = std::time::Instant::now();
        let doc = parse_html(&deep_html);
        let duration = start.elapsed();

        assert!(!doc.elements.is_empty());
        // Skeleton-first should be extremely fast, even with 150 levels
        assert!(
            duration.as_millis() < 50,
            "Skeleton refactor should be ultra-fast: {:?}",
            duration
        );
    }

    #[test]
    fn test_3kb_fixture() {
        let path = std::path::Path::new("tests/fixtures/perf_3kb.html");
        if !path.exists() {
            return;
        }
        let content = std::fs::read_to_string(path).unwrap();
        let doc = parse_html(&content);
        assert!(!doc.elements.is_empty());
        assert!(doc
            .elements
            .iter()
            .any(|e| e.name.as_deref() == Some("div")));
    }

    #[test]
    fn test_circuit_breaker_skeleton() {
        // Test Element Limit
        let many_tags = "<div></div>".repeat(110000);
        let meta = DocumentMetadata::new(PathBuf::from("bomb.html"), many_tags.len() as u64);
        let mut doc = ParsedDocument::new(meta);
        let result = parse_html_internal(&mut doc, &many_tags);
        assert!(result.is_err());
        match result.unwrap_err() {
            VecqError::CircuitBreakerTriggered { message } => assert!(message.contains("limit")),
            _ => panic!("Expected CircuitBreakerTriggered (Limit)"),
        }

        // Unclosed tags are no longer a nesting bomb: tree-sitter-html's error
        // recovery reads `<div>` × 102 with no end tags as `ERROR` text (a few
        // tags: one ERROR node; many: an ERROR root of loose start tags) —
        // flat, spanned, counted as a syntax error — never 102 nested elements.
        let unclosed = "<div>".repeat(102);
        let meta = DocumentMetadata::new(PathBuf::from("deep.html"), unclosed.len() as u64);
        let mut doc = ParsedDocument::new(meta);
        parse_html_internal(&mut doc, &unclosed).unwrap();
        assert!(doc.elements.iter().all(|e| e.children.is_empty()));
        assert!(doc.metadata.syntax_errors >= 1);
        crate::parsers::ownership::assert_reconstructs(
            "unclosed",
            &doc.elements,
            &unclosed,
            |_| true,
        );

        // Test Consecutive Tag Bomb (D026): the nesting the grammar does build.
        let consecutive_bomb = format!("{}{}", "<div>".repeat(102), "</div>".repeat(102));
        let meta = DocumentMetadata::new(PathBuf::from("deep.html"), consecutive_bomb.len() as u64);
        let mut doc = ParsedDocument::new(meta);
        let result = parse_html_internal(&mut doc, &consecutive_bomb);
        assert!(result.is_err());
        match result.unwrap_err() {
            VecqError::CircuitBreakerTriggered { message } => {
                assert!(message.contains("Consecutive open tags"));
                assert!(message.contains("div"));
            }
            _ => panic!("Expected CircuitBreakerTriggered (Consecutive)"),
        }
    }

    #[test]
    fn test_enrich_markdown() {
        let text = "<task>\n# Header\n- Item 1\n</task>";
        let doc = parse_html(text);

        let task = &doc.elements[0];
        // In Skeleton mode, there should be no Markdown children yet
        assert!(
            task.children.is_empty(),
            "Skeleton should not have markdown children"
        );

        // Enrich it
        let enriched_doc = crate::enrich_document(doc).unwrap();
        let enriched_task = &enriched_doc.elements[0];

        assert_eq!(
            enriched_task.children.len(),
            3,
            "Enriched task should have 3 markdown children (Header, List, and ListItem)"
        );
        let header = &enriched_task.children[0];
        assert_eq!(header.element_type, crate::types::ElementType::Header);
        assert_eq!(header.name.as_deref(), Some("Header"));
        assert_eq!(header.line_start, 2);
    }

    #[test]
    fn test_html_failure_fixture() {
        let path = std::path::Path::new("tests/fixtures/html_failure.html");
        if !path.exists() {
            return;
        }
        let content = std::fs::read_to_string(path).unwrap();

        // This file contains binary junk. While the parser itself might try to find tags,
        // we want to ensure it doesn't hang or crash.
        let doc = parse_html(&content);
        assert!(!doc.elements.is_empty());
    }
}
