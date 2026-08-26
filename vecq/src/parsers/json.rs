// PURPOSE:
//   Parser for JSON files.
//   Converts JSON content into a structured Document representation.

use super::json_spans::{self, SpanNode};
use crate::error::{VecqError, VecqResult};
use crate::parser::Parser;
use crate::types::ParsedDocument;
use crate::types::{
    DocumentElement, DocumentMetadata, ElementAttributes, ElementType, FileType, JsonAttributes,
};
use async_trait::async_trait;
use std::collections::HashMap;
use std::path::PathBuf;
use vecdb_common::LineCounter;

pub struct JsonParser;

impl Default for JsonParser {
    fn default() -> Self {
        Self::new()
    }
}

impl JsonParser {
    pub fn new() -> Self {
        Self
    }

    // Helper to recursively process JSON structure into DocumentElements
    //
    // `span` is this node's position in the source, threaded down in lockstep
    // with the value tree. It is matched by key and by index rather than
    // positionally: `serde_json::Map` is a BTreeMap without the `preserve_order`
    // feature, so its iteration order is alphabetical, not document order.
    //
    // `None` means the span scan could not place this node. The previous
    // behaviour — hardcoding `1, 1` for every element in every file — is what
    // made AST-addressed edits of JSON destroy line 1 and leave the target
    // intact, so it survives only as the last-resort fallback it always should
    // have been.
    fn process_value(
        key: String,
        value: &serde_json::Value,
        span: Option<&SpanNode>,
        lines: &LineCounter,
    ) -> DocumentElement {
        let (line_start, line_end) = match span {
            Some(s) => {
                let start = lines.get_line_number(s.start);
                // `end` is one past the last byte; step back so a node ending at
                // a newline is not credited with the following line.
                let end = lines.get_line_number(s.end.saturating_sub(1)).max(start);
                (start, end)
            }
            None => (1, 1),
        };

        let mut element = DocumentElement::new(
            ElementType::Variable,
            Some(key),
            value.to_string(),
            line_start,
            line_end,
        );

        match value {
            serde_json::Value::Object(map) => {
                element.element_type = ElementType::Block;
                let mut children = Vec::new();
                for (k, v) in map {
                    children.push(Self::process_value(
                        k.clone(),
                        v,
                        span.and_then(|s| s.field(k)),
                        lines,
                    ));
                }
                element = element.with_children(children);
            }
            serde_json::Value::Array(arr) => {
                element.element_type = ElementType::Block;
                let mut children = Vec::new();
                for (i, v) in arr.iter().enumerate() {
                    children.push(Self::process_value(
                        format!("[{}]", i),
                        v,
                        span.and_then(|s| s.item(i)),
                        lines,
                    ));
                }
                element = element.with_children(children);
            }
            _ => {
                element.element_type = ElementType::Variable;
            }
        }

        element.set_attributes(ElementAttributes::Json(JsonAttributes {
            other: {
                let mut other = HashMap::new();
                other.insert("value".to_string(), value.clone());
                other
            },
        }))
    }
}

#[async_trait]
impl Parser for JsonParser {
    fn file_extensions(&self) -> &[&str] {
        &["json", "ndjson", "jsonl"]
    }

    fn language_name(&self) -> &str {
        "JSON"
    }

    async fn parse(&self, content: &str) -> VecqResult<ParsedDocument> {
        // Streaming deserialization, so that JSONL (one value per line) and
        // concatenated roots parse as several values rather than an error.
        let deserializer = serde_json::Deserializer::from_str(content);
        let mut json_values = Vec::new();
        let mut strict_error = None;
        for item in deserializer.into_iter::<serde_json::Value>() {
            match item {
                Ok(val) => json_values.push(val),
                Err(e) => {
                    strict_error = Some(e);
                    break;
                }
            }
        }

        if let Some(e) = strict_error {
            // Comments and trailing commas are pervasive in real `.json` files —
            // `tsconfig.json`, `.eslintrc.json`, `devcontainer.json`. Strict
            // JSON rejects them.
            //
            // `vecdb-core` carried a JSON parser that already handled this, but
            // nothing constructed it: every binary injects the vecq adapter, so
            // the capable parser was unreachable and these files silently
            // degraded to one unstructured text chunk. The capability belongs
            // here, where the only JSON parser lives.
            match json5::from_str::<serde_json::Value>(content) {
                Ok(value) => {
                    json_values.clear();
                    json_values.push(value);
                }
                Err(_) => {
                    // Report the strict error, not the JSON5 one: for genuinely
                    // malformed JSON it names the real problem and its position.
                    return Err(VecqError::json_error(
                        format!("Failed to parse JSON: {}", e),
                        Some(e),
                    ));
                }
            }
        }

        let metadata = DocumentMetadata::new(PathBuf::from("memory"), content.len() as u64)
            .with_file_type(FileType::Json)
            .with_line_count(content);

        let mut doc = ParsedDocument::new(metadata).with_source(content);

        // Byte spans recovered from the raw text; `None` if it did not scan
        // cleanly, in which case every node falls back to 1..1 as before.
        let lines = LineCounter::new(content);
        let roots = json_spans::scan(content);

        for (root_idx, json_value) in json_values.into_iter().enumerate() {
            let root = roots.as_ref().and_then(|r| r.get(root_idx));
            match json_value {
                serde_json::Value::Object(map) => {
                    for (k, v) in map {
                        doc.elements.push(Self::process_value(
                            k.clone(),
                            &v,
                            root.and_then(|s| s.field(&k)),
                            &lines,
                        ));
                    }
                }
                serde_json::Value::Array(arr) => {
                    for (i, v) in arr.iter().enumerate() {
                        doc.elements.push(Self::process_value(
                            format!("[{}]", i),
                            v,
                            root.and_then(|s| s.item(i)),
                            &lines,
                        ));
                    }
                }
                _ => {
                    doc.elements.push(Self::process_value(
                        "root".to_string(),
                        &json_value,
                        root,
                        &lines,
                    ));
                }
            }
        }

        Ok(doc)
    }
}
