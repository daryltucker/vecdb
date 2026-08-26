// PURPOSE:
//   Parser for YAML files (docker-compose, CI workflows, k8s manifests, and
//   most of the configuration in this workspace).
//   Converts YAML into a structured Document representation with line numbers.
//
// RELATED FILES:
//   - src/parsers/toml.rs - the structural pattern this mirrors
//   - src/parsers/json.rs - same data model, different surface syntax
//
// HISTORY:
//   `.yaml` and `.yml` used to resolve to `FileType::Text`, so a YAML file was
//   stored as one undifferentiated blob with no keys, no nesting, and no line
//   attribution. `vecdb-core` did contain a `YamlParser`, but it was registered
//   for `FileType::Toml` — a different format — and reachable only through a
//   factory that no binary ever constructed. Structured YAML support has never
//   actually shipped until now.
//
// LINE ATTRIBUTION: per-key, as of day 238. See `parsers/yaml_spans.rs`.
//   `serde_yaml_ng::Value` carries no source positions, so this parser used to
//   give every element the document's full line range. That was a wide-but-true
//   answer rather than a fabricated one — but it was also 0-indexed
//   (`0..lines-1`), so the honest range still named lines that exist in no
//   editor, and it disagreed with every other parser about what `line_start`
//   means.
//
//   Both are fixed. `yaml_spans` runs yaml-rust's event parser purely for its
//   `Marker` line numbers and hands back a tree matched by key and by index;
//   `serde_yaml_ng` still produces the values. If that scan fails the old
//   document-wide range remains as the fallback, now 1-indexed.

use super::yaml_spans::{self, YamlSpan};
use crate::error::{VecqError, VecqResult};
use crate::parser::Parser;
use crate::types::{
    DocumentElement, DocumentMetadata, ElementAttributes, ElementType, FileType, ParsedDocument,
    YamlAttributes,
};
use async_trait::async_trait;
use std::collections::HashMap;
use std::path::PathBuf;

pub struct YamlParser;

impl Default for YamlParser {
    fn default() -> Self {
        Self::new()
    }
}

impl YamlParser {
    pub fn new() -> Self {
        Self
    }

    /// Build an element tree from a YAML value.
    ///
    /// Mappings and sequences become `Block`s with children; scalars become
    /// `Variable` leaves. This mirrors `TomlParser::process_item` so that a
    /// query like `.entries[]` behaves the same across the two config formats —
    /// having them disagree is the whole class of problem this parser exists
    /// inside of.
    fn process_value(
        &self,
        key: &str,
        value: &serde_yaml_ng::Value,
        span: Option<&YamlSpan>,
        doc_start: usize,
        doc_end: usize,
    ) -> DocumentElement {
        // A real span when yaml-rust could place this node; the whole document
        // otherwise. Matched by key and by index rather than positionally, so
        // neither library's iteration order can mis-assign a line.
        let (line_start, line_end) = match span {
            Some(s) => (s.start_line, s.end_line.max(s.start_line)),
            None => (doc_start, doc_end),
        };
        let json = Self::to_json(value);

        // Content is `key: value`, not the bare scalar.
        //
        // This string is what gets embedded. A chunk reading `localhost` carries
        // no indication of what is at localhost; `host: localhost` is both
        // searchable and the form it takes in the file. The key is also in
        // `name` and the path in `crumbtrail`, but those are payload metadata —
        // they are filterable, not matchable.
        //
        // It is the same failure mode as a Python function indexed as
        // `def alpha(...)`: technically an identifier, useless as a vector.
        let content = match value {
            serde_yaml_ng::Value::Mapping(_) | serde_yaml_ng::Value::Sequence(_) => String::new(),
            scalar => format!("{key}: {}", Self::render_scalar(scalar)),
        };

        let mut element = DocumentElement::new(
            ElementType::Variable,
            Some(key.to_string()),
            content,
            line_start,
            line_end,
        );

        match value {
            serde_yaml_ng::Value::Mapping(map) => {
                element.element_type = ElementType::Block;
                for (k, v) in map {
                    let child_key = Self::key_to_string(k);
                    element.children.push(self.process_value(
                        &child_key,
                        v,
                        span.and_then(|s| s.field(&child_key)),
                        doc_start,
                        doc_end,
                    ));
                }
            }
            serde_yaml_ng::Value::Sequence(seq) => {
                element.element_type = ElementType::Block;
                for (idx, v) in seq.iter().enumerate() {
                    let child_key = format!("{key}[{idx}]");
                    element.children.push(self.process_value(
                        &child_key,
                        v,
                        span.and_then(|s| s.item(idx)),
                        doc_start,
                        doc_end,
                    ));
                }
            }
            _ => {}
        }

        element
            .set_attributes(ElementAttributes::Yaml(YamlAttributes {
                other: HashMap::new(),
            }))
            .with_attribute("value".to_string(), json)
    }

    /// Content for a leaf is its scalar text; for a container it is left empty
    /// rather than re-serialised, so a nested document is not duplicated once
    /// per level of nesting when chunks are built from these elements.
    fn render_scalar(value: &serde_yaml_ng::Value) -> String {
        match value {
            serde_yaml_ng::Value::String(s) => s.clone(),
            serde_yaml_ng::Value::Number(n) => n.to_string(),
            serde_yaml_ng::Value::Bool(b) => b.to_string(),
            serde_yaml_ng::Value::Null => String::new(),
            _ => String::new(),
        }
    }

    fn key_to_string(key: &serde_yaml_ng::Value) -> String {
        match key {
            serde_yaml_ng::Value::String(s) => s.clone(),
            other => Self::render_scalar(other),
        }
    }

    fn to_json(value: &serde_yaml_ng::Value) -> serde_json::Value {
        match value {
            serde_yaml_ng::Value::Null => serde_json::Value::Null,
            serde_yaml_ng::Value::Bool(b) => serde_json::Value::Bool(*b),
            serde_yaml_ng::Value::Number(n) => {
                if let Some(i) = n.as_i64() {
                    serde_json::Value::Number(i.into())
                } else if let Some(f) = n.as_f64() {
                    serde_json::Number::from_f64(f)
                        .map(serde_json::Value::Number)
                        .unwrap_or(serde_json::Value::Null)
                } else {
                    serde_json::Value::Null
                }
            }
            serde_yaml_ng::Value::String(s) => serde_json::Value::String(s.clone()),
            serde_yaml_ng::Value::Sequence(seq) => {
                serde_json::Value::Array(seq.iter().map(Self::to_json).collect())
            }
            serde_yaml_ng::Value::Mapping(map) => {
                let mut out = serde_json::Map::new();
                for (k, v) in map {
                    out.insert(Self::key_to_string(k), Self::to_json(v));
                }
                serde_json::Value::Object(out)
            }
            serde_yaml_ng::Value::Tagged(tagged) => Self::to_json(&tagged.value),
        }
    }
}

#[async_trait]
impl Parser for YamlParser {
    async fn parse(&self, content: &str) -> VecqResult<ParsedDocument> {
        let metadata = DocumentMetadata::new(PathBuf::from("memory"), content.len() as u64)
            .with_file_type(FileType::Yaml)
            .with_line_count(content);
        let mut doc = ParsedDocument::new(metadata).with_source(content);

        // Document-scoped span, 1-INDEXED.
        //
        // `serde_yaml_ng` discards per-node positions, so every element honestly
        // gets the whole document's range rather than a fabricated narrow guess.
        // What is NOT acceptable is doing that 0-indexed: this used to emit
        // `0..lines-1`, making every YAML element in every collection report a
        // line that does not exist in any editor, and disagree with Rust, C and
        // Bash on what `line_start` even means.
        //
        // Narrowing these to real per-key spans needs a position-preserving YAML
        // parser and is tracked separately; the convention is not blocked on it.
        const DOC_FIRST_LINE: usize = 1;
        let total_lines = content.lines().count().max(DOC_FIRST_LINE);

        // Multi-document YAML (`---` separated) is normal in Kubernetes and CI
        // config. Each document becomes its own top-level element rather than
        // being silently reduced to the first one.
        let docs: Vec<serde_yaml_ng::Value> = serde_yaml_ng::Deserializer::from_str(content)
            .map(serde_yaml_ng::Value::deserialize)
            .collect::<Result<Vec<_>, _>>()
            .map_err(|e| VecqError::ParseError {
                file: PathBuf::from("unknown.yaml"),
                line: e.location().map(|l| l.line()).unwrap_or(0),
                message: e.to_string(),
                source: Some(Box::new(e)),
            })?;

        // Real per-node line spans, recovered from a second, position-aware
        // parse. `None` falls back to the document range this used to use for
        // every node unconditionally.
        let spans = yaml_spans::scan(content);

        let multi = docs.len() > 1;
        for (idx, value) in docs.iter().enumerate() {
            let doc_span = spans.as_ref().and_then(|s| s.get(idx));
            match value {
                // A single top-level mapping is the common case; lift its keys
                // to the top so `.entries[]` reaches them without an artificial
                // wrapper, matching TomlParser.
                serde_yaml_ng::Value::Mapping(map) if !multi => {
                    for (k, v) in map {
                        let key = Self::key_to_string(k);
                        doc.elements.push(self.process_value(
                            &key,
                            v,
                            doc_span.and_then(|s| s.field(&key)),
                            DOC_FIRST_LINE,
                            total_lines,
                        ));
                    }
                }
                other => {
                    doc.elements.push(self.process_value(
                        &format!("document[{idx}]"),
                        other,
                        doc_span,
                        DOC_FIRST_LINE,
                        total_lines,
                    ));
                }
            }
        }

        Ok(doc)
    }

    fn file_extensions(&self) -> &[&str] {
        &["yaml", "yml"]
    }

    fn language_name(&self) -> &str {
        "YAML"
    }
}

use serde::Deserialize;
