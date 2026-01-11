// PURPOSE:
//   Parser for JSON files.
//   Converts JSON content into a structured Document representation.

use crate::error::{VecqError, VecqResult};
use crate::parser::Parser;
use crate::types::{DocumentElement, ElementType, FileType, DocumentMetadata, JsonAttributes, ElementAttributes};
use crate::types::ParsedDocument;
use async_trait::async_trait;
use std::collections::HashMap;
use std::path::PathBuf;

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
    fn process_value(key: String, value: &serde_json::Value) -> DocumentElement {
        let mut element = DocumentElement::new(
            ElementType::Variable,
            Some(key),
            value.to_string(),
            1, 1 // JSON standard parser doesn't provide line numbers
        );

        match value {
            serde_json::Value::Object(map) => {
                element.element_type = ElementType::Block;
                let mut children = Vec::new();
                for (k, v) in map {
                    children.push(Self::process_value(k.clone(), v));
                }
                element = element.with_children(children);
            },
            serde_json::Value::Array(arr) => {
                element.element_type = ElementType::Block;
                let mut children = Vec::new();
                for (i, v) in arr.iter().enumerate() {
                    children.push(Self::process_value(format!("[{}]", i), v));
                }
                element = element.with_children(children);
            },
            _ => {
                element.element_type = ElementType::Variable;
            }
        }

        element.set_attributes(ElementAttributes::Json(JsonAttributes {
            other: {
                let mut other = HashMap::new();
                other.insert("value".to_string(), value.clone());
                other
            }
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
        let deserializer = serde_json::Deserializer::from_str(content);
        let mut json_values = Vec::new();
        for item in deserializer.into_iter::<serde_json::Value>() {
            let val = item.map_err(|e| VecqError::json_error(format!("Failed to parse JSON: {}", e), Some(e)))?;
            json_values.push(val);
        }

        let metadata = DocumentMetadata::new(PathBuf::from("memory"), content.len() as u64)
            .with_file_type(FileType::Json)
            .with_line_count(content);

        let mut doc = ParsedDocument::new(metadata).with_source(content);

        for json_value in json_values {
            match json_value {
                serde_json::Value::Object(map) => {
                    for (k, v) in map {
                        doc.elements.push(Self::process_value(k.clone(), &v));
                    }
                },
                serde_json::Value::Array(arr) => {
                    for (i, v) in arr.iter().enumerate() {
                        doc.elements.push(Self::process_value(format!("[{}]", i), v));
                    }
                },
                _ => {
                    doc.elements.push(Self::process_value("root".to_string(), &json_value));
                }
            }
        }

        Ok(doc)
    }
}
