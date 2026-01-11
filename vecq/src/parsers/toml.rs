
// PURPOSE:
//   Parser for TOML files (commonly used for configuration in Rust/Python projects).
//   Converts TOML content into a structured Document representation.

use crate::error::{VecqError, VecqResult};
use crate::parser::Parser;
use crate::types::{DocumentElement, ElementType, ParsedDocument, FileType, DocumentMetadata, TomlAttributes, ElementAttributes};
use std::collections::HashMap;
use async_trait::async_trait;
use std::path::PathBuf;

pub struct TomlParser;

impl TomlParser {
    pub fn new() -> Self {
        Self
    }

    fn convert_value(value: &toml::Value) -> serde_json::Value {
        match value {
            toml::Value::String(s) => serde_json::Value::String(s.clone()),
            toml::Value::Integer(i) => serde_json::Value::Number((*i).into()),
            toml::Value::Float(f) => {
                if let Some(n) = serde_json::Number::from_f64(*f) {
                    serde_json::Value::Number(n)
                } else {
                    serde_json::Value::Null
                }
            },
            toml::Value::Boolean(b) => serde_json::Value::Bool(*b),
            toml::Value::Datetime(d) => serde_json::Value::String(d.to_string()),
            toml::Value::Array(arr) => {
                serde_json::Value::Array(arr.iter().map(Self::convert_value).collect())
            },
            toml::Value::Table(table) => {
                let map: serde_json::Map<String, serde_json::Value> = table.iter()
                    .map(|(k, v)| (k.clone(), Self::convert_value(v)))
                    .collect();
                serde_json::Value::Object(map)
            },
        }
    }

    // Helper to recursively process TOML structure into DocumentElements
    // TOML is data, so we treat top-level tables as sections/blocks
    fn process_table(table: &toml::Table, _parent_depth: usize) -> Vec<DocumentElement> {
        let mut elements = Vec::new();
        
        for (key, value) in table {
           let mut element = DocumentElement::new(
               ElementType::Variable, // Generic key-value pair
               Some(key.clone()),
               value.to_string(), // Raw representation
               1, 1 // TODO: TOML parser doesn't give line numbers easily. defaulting to 1.
           );
           
           // If it's a table, recurse
           if let toml::Value::Table(inner) = value {
               element.element_type = ElementType::Block; // Or Module/Struct equivalent
               element.children = Self::process_table(inner, _parent_depth + 1);
           }
           
           // Store actual value as attribute for JSON conversion later
           element = element.set_attributes(ElementAttributes::Toml(TomlAttributes { 
               other: HashMap::new() 
           })).with_attribute("value".to_string(), Self::convert_value(value));
           elements.push(element);
        }
        
        elements
    }
}

#[async_trait]
impl Parser for TomlParser {
    async fn parse(&self, content: &str) -> VecqResult<ParsedDocument> {
        // Parse TOML
        let toml_value: toml::Value = toml::from_str(content)
            .map_err(|e| VecqError::ParseError { 
                file: PathBuf::from("unknown.toml"),
                line: 0,
                message: e.to_string(),
                source: Some(Box::new(e))
            })?;

        // Create document
        let metadata = DocumentMetadata::new(PathBuf::from("memory"), content.len() as u64)
            .with_file_type(FileType::Toml)
            .with_line_count(content);
            
        let mut doc = ParsedDocument::new(metadata)
            .with_source(content);
            
        // We fundamentally treat TOML as a data structure. 
        // For vecq, we want to enable `query_json` mostly.
        // The DocumentElement structure is less critical for data formats than code,
        // but we populating it allows for some structural grepping.
        
        if let toml::Value::Table(table) = toml_value {
            doc.elements = Self::process_table(&table, 0);
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
