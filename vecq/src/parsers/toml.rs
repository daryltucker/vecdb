
// PURPOSE:
//   Parser for TOML files (commonly used for configuration in Rust/Python projects).
//   Converts TOML content into a structured Document representation with accurate line numbers.

use crate::error::{VecqError, VecqResult};
use crate::parser::Parser;
use crate::types::{DocumentElement, ElementType, ParsedDocument, FileType, DocumentMetadata, TomlAttributes, ElementAttributes};
use std::collections::HashMap;
use async_trait::async_trait;
use std::path::PathBuf;
use toml_edit::DocumentMut;
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
                },
                toml_edit::Value::Boolean(b) => serde_json::Value::Bool(*b.value()),
                toml_edit::Value::Datetime(d) => serde_json::Value::String(d.to_string()),
                toml_edit::Value::Array(arr) => {
                    serde_json::Value::Array(arr.iter().map(|v| Self::convert_value(&toml_edit::Item::Value(v.clone()))).collect())
                },
                toml_edit::Value::InlineTable(table) => {
                    let mut map = serde_json::Map::new();
                    for (k, v) in table.iter() {
                        map.insert(k.to_string(), Self::convert_value(&toml_edit::Item::Value(v.clone())));
                    }
                    serde_json::Value::Object(map)
                },
            },
            toml_edit::Item::Table(table) => {
                let mut map = serde_json::Map::new();
                for (k, v) in table.iter() {
                    map.insert(k.to_string(), Self::convert_value(v));
                }
                serde_json::Value::Object(map)
            },
            toml_edit::Item::ArrayOfTables(arr) => {
                serde_json::Value::Array(arr.iter().map(|t| Self::convert_value(&toml_edit::Item::Table(t.clone()))).collect())
            },
            toml_edit::Item::None => serde_json::Value::Null,
        }
    }

    fn process_item(&self, key: &str, item: &toml_edit::Item, counter: &LineCounter) -> DocumentElement {
        let span = item.span().unwrap_or(0..0);
        let start_line = counter.get_line_number(span.start);
        let end_line = counter.get_line_number(span.end.saturating_sub(1)).max(start_line);

        let mut element = DocumentElement::new(
            ElementType::Variable,
            Some(key.to_string()),
            item.to_string(),
            start_line,
            end_line
        );

        match item {
            toml_edit::Item::Table(table) => {
                element.element_type = ElementType::Block;
                for (k, v) in table.iter() {
                    element.children.push(self.process_item(k, v, counter));
                }
            },
            toml_edit::Item::ArrayOfTables(arr) => {
                element.element_type = ElementType::Block;
                for (idx, table) in arr.iter().enumerate() {
                    let key_with_idx = format!("{}[{}]", key, idx);
                    // Use a temporary Item::Table to avoid cloning if possible, 
                    // but toml_edit's ArrayOfTables returns Table, we need Item.
                    // toml_edit::Item::Table is a wrapper around Table.
                    let table_item = toml_edit::Item::Table(table.clone());
                    element.children.push(self.process_item(&key_with_idx, &table_item, counter));
                }
            },
            _ => {
                // Leaf value
            }
        }

        element = element.set_attributes(ElementAttributes::Toml(TomlAttributes {
            other: HashMap::new()
        })).with_attribute("value".to_string(), Self::convert_value(item));

        element
    }
}

#[async_trait]
impl Parser for TomlParser {
    async fn parse(&self, content: &str) -> VecqResult<ParsedDocument> {
        let doc_mut = content.parse::<DocumentMut>()
            .map_err(|e| VecqError::ParseError {
                file: PathBuf::from("unknown.toml"),
                line: 0,
                message: e.to_string(),
                source: Some(Box::new(e))
            })?;

        let counter = LineCounter::new(content);
        let metadata = DocumentMetadata::new(PathBuf::from("memory"), content.len() as u64)
            .with_file_type(FileType::Toml)
            .with_line_count(content);

        let mut doc = ParsedDocument::new(metadata).with_source(content);
        
        for (key, item) in doc_mut.iter() {
            doc.elements.push(self.process_item(key, item, &counter));
        }

        Ok(doc)
    }

    fn file_extensions(&self) -> &[&str] {
        &["toml"]
    }

    fn language_name(&self) -> &str {
        "TOML"
    }
}
