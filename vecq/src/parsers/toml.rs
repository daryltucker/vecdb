// PURPOSE:
//   Parser for TOML files (commonly used for configuration in Rust/Python projects).
//   Converts TOML content into a structured Document representation with accurate line numbers.

use crate::error::{VecqError, VecqResult};
use crate::parser::Parser;
use crate::types::{
    DocumentElement, DocumentMetadata, ElementAttributes, ElementType, FileType, ParsedDocument,
    TomlAttributes,
};
use async_trait::async_trait;
use std::collections::HashMap;
use std::path::PathBuf;

use vecdb_common::LineCounter;

pub struct TomlParser;

impl Default for TomlParser {
    fn default() -> Self {
        Self::new()
    }
}

impl TomlParser {
    pub fn new() -> Self {
        Self
    }

    fn convert_value(item: &toml_edit::Item) -> serde_json::Value {
        match item {
            toml_edit::Item::Value(v) => match v {
                toml_edit::Value::String(s) => serde_json::Value::String(s.value().to_string()),
                toml_edit::Value::Integer(i) => serde_json::Value::Number((*i.value()).into()),
                toml_edit::Value::Float(f) => {
                    if let Some(n) = serde_json::Number::from_f64(*f.value()) {
                        serde_json::Value::Number(n)
                    } else {
                        serde_json::Value::Null
                    }
                }
                toml_edit::Value::Boolean(b) => serde_json::Value::Bool(*b.value()),
                toml_edit::Value::Datetime(d) => serde_json::Value::String(d.to_string()),
                toml_edit::Value::Array(arr) => serde_json::Value::Array(
                    arr.iter()
                        .map(|v| Self::convert_value(&toml_edit::Item::Value(v.clone())))
                        .collect(),
                ),
                toml_edit::Value::InlineTable(table) => {
                    let mut map = serde_json::Map::new();
                    for (k, v) in table.iter() {
                        map.insert(
                            k.to_string(),
                            Self::convert_value(&toml_edit::Item::Value(v.clone())),
                        );
                    }
                    serde_json::Value::Object(map)
                }
            },
            toml_edit::Item::Table(table) => {
                let mut map = serde_json::Map::new();
                for (k, v) in table.iter() {
                    map.insert(k.to_string(), Self::convert_value(v));
                }
                serde_json::Value::Object(map)
            }
            toml_edit::Item::ArrayOfTables(arr) => serde_json::Value::Array(
                arr.iter()
                    .map(|t| Self::convert_value(&toml_edit::Item::Table(t.clone())))
                    .collect(),
            ),
            toml_edit::Item::None => serde_json::Value::Null,
        }
    }

    /// A `key = value` entry, spanning from its (first) key to the end of its
    /// value.
    ///
    /// Content is `key = value`, not the bare value — and it
    /// is exactly that text as written (`vecq::parse_file` reads it off the
    /// span).
    ///
    /// This string is what gets embedded, and the bare value for a leaf is just
    /// the value — so `vecdb-core/Cargo.toml` once produced four separate chunks
    /// whose entire content was ` true`. They embed to the same vector, carry no
    /// indication of which key they belong to, and are indistinguishable in a
    /// search result. The key lives in `name` and the path in `crumbtrail`, but
    /// those are payload metadata: filterable, not matchable. `key = value`
    /// matches the source line and keeps TOML and YAML consistent — a query that
    /// works on one should work on the other.
    fn entry(name: String, start: usize, item: &toml_edit::Item) -> Option<DocumentElement> {
        let end = item.span()?.end;
        let element = DocumentElement::new(ElementType::Variable, Some(name), String::new(), 1, 1)
            .with_source_span(start, end)
            .set_attributes(ElementAttributes::Toml(TomlAttributes {
                other: HashMap::new(),
            }))
            .with_attribute("value".to_string(), Self::convert_value(item));
        Some(element)
    }

    /// Walk `table`'s items. Its entries go to `entries`, named relative to the
    /// section (`rel`): a dotted key `metadata.x = 1` is an entry `metadata.x`
    /// of the section it is written in. Every table with a header of its own
    /// becomes a separate section in `out`.
    ///
    /// Sections are FLAT — one per `[header]` / `[[header]]` as written, named
    /// by the full dotted path. A table nested in the tree (`[a.b]` under `[a]`)
    /// is not nested in the text: its bytes come after other sections', so a
    /// parent spanning it would overlap them. One header, one element, one span.
    fn walk(
        table: &toml_edit::Table,
        section_path: &str,
        rel: &str,
        entries: &mut Vec<DocumentElement>,
        out: &mut Sections<'_>,
    ) {
        for (key, item) in table.iter() {
            // An entry starts where its line's text starts: the grammar puts
            // each `key = value` on a line of its own, and a dotted key's
            // shared prefix (`metadata` in `metadata.a` / `metadata.b`) has ONE
            // span in toml_edit — the first occurrence's — so the key's own
            // span cannot say where the second entry begins.
            let start = table
                .key(key)
                .and_then(|k| k.span())
                .map(|s| line_text_start(out.source, s.start));
            let rel_name = if rel.is_empty() {
                key.to_string()
            } else {
                format!("{rel}.{key}")
            };
            let full = if section_path.is_empty() {
                key.to_string()
            } else {
                format!("{section_path}.{key}")
            };
            match item {
                toml_edit::Item::Value(_) => {
                    if let Some(e) = start.and_then(|s| Self::entry(rel_name, s, item)) {
                        entries.push(e);
                    }
                }
                toml_edit::Item::Table(t) if t.is_dotted() => {
                    Self::walk(t, &full, &rel_name, entries, out);
                }
                toml_edit::Item::Table(t) => Self::section(t, &full, item, out),
                toml_edit::Item::ArrayOfTables(arr) => {
                    for (i, t) in arr.iter().enumerate() {
                        let as_item = toml_edit::Item::Table(t.clone());
                        Self::section(t, &format!("{full}[{i}]"), &as_item, out);
                    }
                }
                toml_edit::Item::None => {}
            }
        }
    }

    /// One section: a header (`[path]`) and its entries, spanning from the
    /// header to its last entry. An implicit table — `[a.b]` with no `[a]` —
    /// has no text of its own and is no element; its sub-tables still are.
    fn section(t: &toml_edit::Table, path: &str, item: &toml_edit::Item, out: &mut Sections<'_>) {
        let mut own = Vec::new();
        Self::walk(t, path, "", &mut own, out);
        let Some(header) = t.span().filter(|s| !t.is_implicit() && s.end > s.start) else {
            return;
        };
        out.headers.push((header.start, header.end));
        let end = own
            .iter()
            .filter_map(|e| e.source_span().map(|s| s.1))
            .max()
            .unwrap_or(header.end)
            .max(header.end);
        let element = DocumentElement::new(
            ElementType::Block,
            Some(path.to_string()),
            String::new(),
            1,
            1,
        )
        .with_source_span(header.start, end)
        .set_attributes(ElementAttributes::Toml(TomlAttributes {
            other: HashMap::new(),
        }))
        .with_attribute("value".to_string(), Self::convert_value(item))
        .with_children(own);
        out.sections.push(element);
    }

    /// Byte spans of the `#` comments in the text no entry or header covers.
    /// TOML's grammar allows only whitespace, newlines and comments there
    /// (strings live inside entries and headers), so a `#` begins a comment
    /// that runs to the end of its line.
    fn comments(source: &str, covered: &[(usize, usize)]) -> Vec<(usize, usize)> {
        let b = source.as_bytes();
        let mut out = Vec::new();
        for (from, to) in super::ownership::uncovered((0, source.len()), covered) {
            let mut i = from;
            while i < to {
                if b[i] != b'#' {
                    i += 1;
                    continue;
                }
                let start = i;
                while i < to && b[i] != b'\n' {
                    i += 1;
                }
                let end = if b[i - 1] == b'\r' { i - 1 } else { i };
                out.push((start, end));
            }
        }
        out
    }
}

/// The sections a walk finds, and their header spans (for the comment scan).
struct Sections<'s> {
    source: &'s str,
    sections: Vec<DocumentElement>,
    headers: Vec<(usize, usize)>,
}

/// Offset of the first non-blank byte of the line holding `at`.
fn line_text_start(source: &str, at: usize) -> usize {
    let line = source[..at].rfind('\n').map_or(0, |i| i + 1);
    line + (source[line..at].len() - source[line..at].trim_start().len())
}

#[async_trait]
impl Parser for TomlParser {
    async fn parse(&self, content: &str) -> VecqResult<ParsedDocument> {
        // `Document`, not `DocumentMut`: parsing into the mutable document
        // discards every span (`despan()`), which is why every TOML element
        // reported line 1. The immutable one keeps them.
        let parsed = toml_edit::Document::parse(content).map_err(|e| VecqError::ParseError {
            file: PathBuf::from("unknown.toml"),
            line: 0,
            message: e.to_string(),
            source: Some(Box::new(e)),
        })?;

        let metadata = DocumentMetadata::new(PathBuf::from("memory"), content.len() as u64)
            .with_file_type(FileType::Toml)
            .with_line_count(content);
        let mut doc = ParsedDocument::new(metadata).with_source(content);

        let mut elements = Vec::new();
        let mut found = Sections {
            source: content,
            sections: Vec::new(),
            headers: Vec::new(),
        };
        Self::walk(parsed.as_table(), "", "", &mut elements, &mut found);
        elements.extend(found.sections);
        elements.sort_by_key(|e| e.source_span().map_or(0, |s| s.0));

        // Lossless: blank lines and comments are trivia,
        // owned by the shared rule, at the top level and inside a section.
        let lines = LineCounter::new(content);
        let mut covered = found.headers;
        for e in &elements {
            if e.children.is_empty() {
                covered.extend(e.source_span());
            }
            covered.extend(e.children.iter().filter_map(|c| c.source_span()));
        }
        covered.sort_unstable();
        let comments = Self::comments(content, &covered);
        super::ownership::assign_trivia(&mut elements, content, &lines, &comments, |_| true);
        for e in elements.iter_mut() {
            super::ownership::assign_nested_trivia(e, content, &comments);
            // `content` and lines read off the span here, not only in
            // `vecq::parse_file`: a bare `Parser::parse` result (the enricher's
            // input) must state what its spans say too.
            e.conform_to_span(content, &lines);
        }
        doc.elements = elements;
        Ok(doc)
    }

    fn file_extensions(&self) -> &[&str] {
        &["toml"]
    }

    fn language_name(&self) -> &str {
        "TOML"
    }
}
