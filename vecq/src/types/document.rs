use super::attributes::ElementAttributes;
use super::element_type::ElementType;
use super::metadata::DocumentMetadata;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use vecdb_common::FileType;
// removed

/// Attribute keys vecq sets for its consumers (vecdb's chunker, ivaldi).
///
/// The handoff from vecq's AST to everything downstream, named once. These
/// were string literals repeated across vecq's parsers and vecdb-core; a typo
/// in any one of them silently broke the handoff. Positions and
/// trivia are NOT attributes — they live in [`SourceInfo`], behind the scenes.
pub mod attr {
    /// The grammar's own node kind (tree-sitter), for elements the coverage
    /// pass emitted.
    pub const KIND: &str = "kind";
    /// The element is a wrapper's frame: its own text outside its children.
    pub const FRAME: &str = "frame";
    /// Markdown: number of enclosing list / item / blockquote / footnote blocks.
    pub const BLOCK_DEPTH: &str = "block_depth";
    /// Markdown: names of the headings an element lies under, outermost first
    /// (`["X", "Y", "Tasks"]`). A heading's own path is its ancestors'. Lets a
    /// query select "the list under X/Y/Tasks" without rebuilding section
    /// ownership from line numbers (Ecosystem Experts on `b79f78e`). The same
    /// path the chunker names a section by (`parent_trail`).
    pub const HEADING_PATH: &str = "heading_path";
    /// Markdown paragraph: its rendered prose — emphasis markers, backticks
    /// and link URLs removed. `content` is the paragraph as written, the span's
    /// source text (inclusive at both ends, per the source-span contract).
    pub const TEXT: &str = "text";
    /// Markdown link / image: the destination. `content` is the markup as
    /// written, the span's source text (inclusive at both ends).
    pub const URL: &str = "url";
    /// Markdown code block: the code alone — no fences, an indented block's
    /// indent removed, ending in its newline. `content` is the block as
    /// written, fences included (since `a38c660`), so extracting runnable code
    /// reads this: `.code_blocks[] | .attributes.code`.
    pub const CODE: &str = "code";
}

/// Where an element came from, and the trivia it owns — kept BEHIND the AST.
///
/// Positions are "not key information for someone directly wanting AST", but
/// they are what makes a file reconstructible (Python → JSON → Python must not
/// come back reordered or stripped). So they live here, not in [`attributes`],
/// and the converter does not put them in query output.
///
/// Invariant for a parser that fills `leading`/`trailing` (the tree-sitter
/// coverage pass, JSON): concatenating every top-level element's `leading` +
/// source text + `trailing`, in order, reproduces the file byte for byte.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct SourceInfo {
    /// Byte offset where the element's text begins in the source.
    pub start: usize,
    /// Byte offset one past the element's last byte.
    pub end: usize,
    /// Trivia before the element that belongs to it: whitespace, blank lines and
    /// the comments that describe it (`vecq::parsers::ownership`).
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub leading: String,
    /// Trivia after the element that belongs to it: the rest of its line,
    /// including a same-line comment, and the newline.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub trailing: String,
}

/// A structural element within a document
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DocumentElement {
    pub element_type: ElementType,
    pub name: Option<String>,
    pub content: String,
    pub line_start: usize,
    pub line_end: usize,
    pub attributes: ElementAttributes,
    pub children: Vec<DocumentElement>,
    /// Position and owned trivia — see [`SourceInfo`]. Not part of the AST.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source: Option<SourceInfo>,
    /// The calls and string literals directly inside this element, when it is
    /// a scope (function, class, impl, …) of a language that extracts them.
    /// `None` for an element that is not a scope.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub scope: Option<super::refs::Scope>,
}

impl DocumentElement {
    /// Create a new document element
    pub fn new(
        element_type: ElementType,
        name: Option<String>,
        content: String,
        line_start: usize,
        line_end: usize,
    ) -> Self {
        Self {
            element_type,
            name,
            content,
            line_start,
            line_end,
            attributes: ElementAttributes::default(),
            children: Vec::new(),
            source: None,
            scope: None,
        }
    }

    /// Add an attribute to this element
    /// Exact byte span of this element in the source, when the parser knows it.
    pub fn source_span(&self) -> Option<(usize, usize)> {
        self.source.as_ref().map(|s| (s.start, s.end))
    }

    /// Record the exact byte span of this element in the source.
    pub fn set_source_span(&mut self, start: usize, end: usize) {
        let info = self.source.get_or_insert_with(SourceInfo::default);
        info.start = start;
        info.end = end;
    }

    /// Trivia this element owns before it (see [`SourceInfo::leading`]).
    pub fn leading_trivia(&self) -> &str {
        self.source.as_ref().map_or("", |s| s.leading.as_str())
    }

    /// Trivia this element owns after it (see [`SourceInfo::trailing`]).
    pub fn trailing_trivia(&self) -> &str {
        self.source.as_ref().map_or("", |s| s.trailing.as_str())
    }

    /// Give this element the trivia it owns. Requires a span (set it first).
    pub fn set_trivia(&mut self, leading: String, trailing: String) {
        if let Some(info) = self.source.as_mut() {
            info.leading = leading;
            info.trailing = trailing;
        }
    }

    /// Give this element (and its children) its exact span in `source` when the
    /// parser did not set one, by locating `content` from `line_start`.
    ///
    /// Sound for any parser whose `content` is a verbatim source slice starting
    /// on `line_start` (the extractor contract). Content that is rendered rather
    /// than sliced simply is not found and gets no span — a missing span is
    /// honest, a guessed one is not. Called once per parse by
    /// `vecq::parse_file`, so every consumer receives spans instead of
    /// re-deriving them.
    pub fn stamp_source_span(&mut self, source: &str, lines: &vecdb_common::LineCounter) {
        if self.source_span().is_none() {
            let content = self.content.trim();
            let located = (!content.is_empty())
                .then(|| lines.line_offset(self.line_start))
                .flatten()
                .and_then(|from| Some(source.get(from..)?.find(content)? + from));
            if let Some(at) = located {
                self.set_source_span(at, at + content.len());
            }
        }
        for child in self.children.iter_mut() {
            child.stamp_source_span(source, lines);
        }
    }

    /// Narrow this element's span (and its children's) to its first and last
    /// non-whitespace byte. Line breaks and blank lines at either end are the
    /// element's terminator, not its text — they become trivia, owned by the
    /// trivia pass (`parsers::ownership`). Parsers call this BEFORE assigning
    /// trivia, so no byte is lost: what the span gives up, trivia takes.
    ///
    /// pulldown-cmark's range for a list item carries the blank line after it,
    /// and tree-sitter's for a C `#include` its newline. Reported as `content`,
    /// a consumer splicing `content` back over `line_start..=line_end` inserted
    /// or swallowed lines; trimming to non-whitespace bytes prevents that.
    pub fn tighten_span(&mut self, source: &str) {
        if let Some((s, e)) = self.source_span() {
            if let Some(text) = source.get(s..e) {
                let start = s + (text.len() - text.trim_start().len());
                let end = (s + text.trim_end().len()).max(start);
                self.set_source_span(start, end);
            }
        }
        for child in self.children.iter_mut() {
            child.tighten_span(source);
        }
    }

    /// Make this element (and its children) state exactly what its span says:
    /// `content` becomes the source bytes of the span, and `line_start` /
    /// `line_end` the 1-indexed, inclusive lines of its first and last byte
    /// ([`LineCounter::line_range`](vecdb_common::LineCounter::line_range)).
    ///
    /// The span is the primary fact; `content` and lines are derived from it,
    /// once, here. Before this, each parser chose its own
    /// content (Markdown paragraphs rendered, JSON values re-serialised) and its
    /// own line math (Markdown exclusive, code inclusive), and spans were
    /// recovered afterwards by searching for the content — so they existed only
    /// where content happened to be verbatim. An element with no span is left as
    /// the parser made it; the contract gate (`tier1_source_contract`) fails on it.
    pub fn conform_to_span(&mut self, source: &str, lines: &vecdb_common::LineCounter) {
        if let Some((s, e)) = self.source_span() {
            if let Some(text) = source.get(s..e) {
                if self.content != text {
                    self.content = text.to_string();
                }
                let (first, last) = lines.line_range(s, e);
                self.line_start = first;
                self.line_end = last;
            }
        }
        for child in self.children.iter_mut() {
            child.conform_to_span(source, lines);
        }
    }

    /// Builder form of [`set_source_span`](Self::set_source_span).
    pub fn with_source_span(mut self, start: usize, end: usize) -> Self {
        self.set_source_span(start, end);
        self
    }

    /// The element's text as written, taken from `source` by its span.
    pub fn source_text<'s>(&self, source: &'s str) -> Option<&'s str> {
        self.source_span().and_then(|(s, e)| source.get(s..e))
    }

    /// The element's own text outside its children — a container's frame
    /// (`impl Foo {`, a class header, members no child covers) — computed from
    /// exact spans, so it never mistakes repeated text for a child.
    ///
    /// `None` when this element or any child has no span: the caller decides
    /// what to do without exact positions rather than receiving a guess.
    ///
    /// A child's OWNED trivia (its doc comment, its line break — see
    /// `parsers::ownership::assign_nested_trivia`) is the child's, not the
    /// frame's: a method's `/// doc` is embedded with the method, never with
    /// its `impl`.
    pub fn frame(&self, source: &str) -> Option<String> {
        let (start, end) = self.source_span()?;
        let mut spans = self
            .children
            .iter()
            .map(|c| {
                c.source_span().map(|(s, e)| {
                    (
                        s.saturating_sub(c.leading_trivia().len()),
                        e + c.trailing_trivia().len(),
                    )
                })
            })
            .collect::<Option<Vec<_>>>()?;
        spans.sort_unstable();
        let mut out = String::new();
        let mut at = start;
        for (s, e) in spans {
            if s > at {
                out.push_str(source.get(at..s)?);
            }
            at = at.max(e);
        }
        if end > at {
            out.push_str(source.get(at..end)?);
        }
        Some(out)
    }

    pub fn with_attribute<V: Into<serde_json::Value>>(mut self, key: String, value: V) -> Self {
        self.attributes.insert_generic(key, value.into());
        self
    }

    /// Add multiple attributes (legacy support - inserts into current variant)
    /// NOTE: This does NOT upgrade the variant. Use set_attributes for strict upgrading.
    pub fn with_attributes(mut self, attributes: HashMap<String, serde_json::Value>) -> Self {
        for (k, v) in attributes {
            self.attributes.insert_generic(k, v);
        }
        self
    }

    /// Set strict attributes
    pub fn set_attributes(mut self, attributes: ElementAttributes) -> Self {
        self.attributes = attributes;
        self
    }

    /// Add a child element
    pub fn with_child(mut self, child: DocumentElement) -> Self {
        self.children.push(child);
        self
    }

    /// Add multiple children
    pub fn with_children(mut self, children: Vec<DocumentElement>) -> Self {
        self.children.extend(children);
        self
    }

    /// Get the span of lines this element covers
    pub fn line_span(&self) -> std::ops::RangeInclusive<usize> {
        self.line_start..=self.line_end
    }

    /// Check if this element contains the given line number
    pub fn contains_line(&self, line: usize) -> bool {
        self.line_span().contains(&line)
    }

    /// Find all child elements of a specific type
    pub fn find_children(&self, element_type: ElementType) -> Vec<&DocumentElement> {
        let mut results = Vec::new();
        self.find_children_recursive(element_type, &mut results);
        results
    }

    fn find_children_recursive<'a>(
        &'a self,
        element_type: ElementType,
        results: &mut Vec<&'a DocumentElement>,
    ) {
        for child in &self.children {
            if child.element_type == element_type {
                results.push(child);
            }
            child.find_children_recursive(element_type, results);
        }
    }
}

/// Complete parsed document representation
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ParsedDocument {
    pub metadata: DocumentMetadata,
    pub elements: Vec<DocumentElement>,
    #[serde(skip)]
    pub source_lines: Option<Vec<String>>,
    /// The module scope: calls and string literals outside every scope
    /// element — a script's top-level work, `if __name__ == "__main__":`
    /// bodies, a Rust `static`'s initializer. `None` for a
    /// language that does not extract calls.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub module: Option<super::refs::Scope>,
}

impl ParsedDocument {
    /// Create a new parsed document
    pub fn new(metadata: DocumentMetadata) -> Self {
        Self {
            metadata,
            elements: Vec::new(),
            source_lines: None,
            module: None,
        }
    }

    /// Add an element to the document
    pub fn add_element(mut self, element: DocumentElement) -> Self {
        self.elements.push(element);
        self
    }

    /// Add multiple elements to the document
    pub fn add_elements(mut self, elements: Vec<DocumentElement>) -> Self {
        self.elements.extend(elements);
        self
    }

    /// Set source lines for context extraction, and give every element its
    /// exact source span (`DocumentElement::stamp_source_span`) — so the spans
    /// are part of what vecq hands over, for every parser, and no consumer
    /// re-derives positions by searching text.
    ///
    /// Then every element is made to state what its span says
    /// (`DocumentElement::conform_to_span`): `content` is the span's source
    /// text and the lines are the span's, inclusive. This is the finishing step
    /// of `vecq::parse_file`, and the source-span contract is
    /// guaranteed for documents that went through it. The parsers that build
    /// elements from spans (Markdown, JSON, TOML, YAML, HTML) conform their own
    /// output as well, so a bare `Parser::parse` result states what its spans
    /// say; the search-based stamping above is what only this step adds.
    pub fn with_source(mut self, content: &str) -> Self {
        self.source_lines = Some(content.lines().map(|s| s.to_string()).collect());
        let lines = vecdb_common::LineCounter::new(content);
        for element in self.elements.iter_mut() {
            element.stamp_source_span(content, &lines);
            element.conform_to_span(content, &lines);
        }
        self
    }

    /// Rebuild the file from the top-level elements: each one's owned leading
    /// trivia + its source text + its owned trailing trivia, in document order.
    /// For a document from `vecq::parse_file` this is `source`, byte for byte —
    /// the lossless guarantee, and how a consumer that edits
    /// one element's bytes can check it kept the rest of the file.
    ///
    /// Markdown's element list is flat (nested blocks and inline spans are
    /// siblings of their container), so only its depth-0 blocks are top level.
    pub fn reconstruct(&self, source: &str) -> Result<String, String> {
        if self.file_type() == FileType::Markdown {
            crate::parsers::ownership::reconstruct(
                &self.elements,
                source,
                crate::parsers::markdown::is_top_block,
            )
        } else {
            crate::parsers::ownership::reconstruct(&self.elements, source, |_| true)
        }
    }

    /// Get context lines before a given line
    pub fn get_context_before(&self, line: usize, count: usize) -> Vec<String> {
        if let Some(lines) = &self.source_lines {
            // line is 1-indexed
            if line <= 1 {
                return Vec::new();
            }
            let end_idx = line - 1; // 0-indexed index of the line
            let start_idx = end_idx.saturating_sub(count);
            lines[start_idx..end_idx].to_vec()
        } else {
            Vec::new()
        }
    }

    /// Get context lines after a given line
    pub fn get_context_after(&self, line: usize, count: usize) -> Vec<String> {
        if let Some(lines) = &self.source_lines {
            // line is 1-indexed
            let start_idx = line; // 0-indexed index of the next line
            if start_idx >= lines.len() {
                return Vec::new();
            }
            let end_idx = std::cmp::min(start_idx + count, lines.len());
            lines[start_idx..end_idx].to_vec()
        } else {
            Vec::new()
        }
    }

    /// Find all elements of a specific type
    pub fn find_elements(&self, element_type: ElementType) -> Vec<&DocumentElement> {
        let mut results = Vec::new();
        for element in &self.elements {
            if element.element_type == element_type {
                results.push(element);
            }
            element.find_children_recursive(element_type, &mut results);
        }
        results
    }

    /// Get elements by line number
    pub fn elements_at_line(&self, line: usize) -> Vec<&DocumentElement> {
        self.elements
            .iter()
            .filter(|element| element.contains_line(line))
            .collect()
    }

    /// Get total line count
    pub fn line_count(&self) -> usize {
        self.metadata.line_count
    }

    /// Get file type
    pub fn file_type(&self) -> FileType {
        self.metadata.file_type
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    #[test]
    fn test_document_element_creation() {
        let element = DocumentElement::new(
            ElementType::Function,
            Some("main".to_string()),
            "fn main() {}".to_string(),
            1,
            1,
        )
        .with_attribute("visibility".to_string(), "public")
        .with_child(DocumentElement::new(
            ElementType::Variable,
            Some("x".to_string()),
            "let x = 42;".to_string(),
            2,
            2,
        ));

        assert_eq!(element.element_type, ElementType::Function);
        assert_eq!(element.name, Some("main".to_string()));
        assert_eq!(element.line_start, 1);
        assert_eq!(element.line_end, 1);
        assert_eq!(element.children.len(), 1);
        // With generic, this works via helper or check variant
        if let ElementAttributes::Generic(map) = element.attributes {
            assert!(map.contains_key("visibility"));
        } else {
            panic!("Expected Generic attributes");
        }
    }

    #[test]
    fn test_document_element_line_operations() {
        let element =
            DocumentElement::new(ElementType::Function, None, "content".to_string(), 5, 10);

        assert_eq!(element.line_span(), 5..=10);
        assert!(element.contains_line(7));
        assert!(!element.contains_line(3));
        assert!(!element.contains_line(12));
    }

    #[test]
    fn test_parsed_document_operations() {
        let metadata = DocumentMetadata::new(PathBuf::from("test.rs"), 100)
            .with_line_count("line1\nline2\nline3");

        let doc = ParsedDocument::new(metadata)
            .add_element(DocumentElement::new(
                ElementType::Function,
                Some("func1".to_string()),
                "fn func1() {}".to_string(),
                1,
                1,
            ))
            .add_element(DocumentElement::new(
                ElementType::Function,
                Some("func2".to_string()),
                "fn func2() {}".to_string(),
                2,
                2,
            ));

        let functions = doc.find_elements(ElementType::Function);
        assert_eq!(functions.len(), 2);
        assert_eq!(doc.line_count(), 3);
        assert_eq!(doc.file_type(), FileType::Rust);
    }
}
