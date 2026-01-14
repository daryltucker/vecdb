use crate::parsers::Parser;
use crate::types::Chunk;
use anyhow::Result;
use serde_yml::Value;
use std::path::Path;
use uuid::Uuid;

pub struct YamlParser;

impl YamlParser {
    pub fn new() -> Self {
        Self
    }


    fn flatten_value(&self, value: &Value, prefix: String, chunks: &mut Vec<String>) {
        match value {
            Value::Mapping(map) => {
                for (k, v) in map {
                    let key_str = match k {
                        Value::String(s) => s.clone(),
                        Value::Number(n) => n.to_string(),
                        Value::Bool(b) => b.to_string(),
                        _ => "complex_key".to_string(),
                    };
                    
                    let new_prefix = if prefix.is_empty() {
                        key_str
                    } else {
                        format!("{}.{}", prefix, key_str)
                    };
                    self.flatten_value(v, new_prefix, chunks);
                }
            }
            Value::Sequence(seq) => {
                for (i, v) in seq.iter().enumerate() {
                    let new_prefix = format!("{}[{}]", prefix, i);
                    self.flatten_value(v, new_prefix, chunks);
                }
            }
            _ => {
                // Leaf node
                if !prefix.is_empty() {
                    chunks.push(format!("{}: {:?}", prefix, value));
                }
            }
        }
    }
}

use async_trait::async_trait;

#[async_trait]
impl Parser for YamlParser {
    async fn parse(&self, content: &str, path: &Path, base_metadata: Option<serde_json::Value>) -> Result<Vec<Chunk>> {
        // serde_yml can define multiple documents in one file
        let docs: Vec<Value> = serde_yml::from_str(content).unwrap_or_else(|_| vec![]);
        
        let mut text_chunks = Vec::new();
        
        for (i, doc) in docs.iter().enumerate() {
            let prefix = if docs.len() > 1 { format!("doc[{}]", i) } else { "".to_string() };
            self.flatten_value(doc, prefix, &mut text_chunks);
        }

        let mut chunks = Vec::new();
        let mut current_chunk_text = String::new();
        let mut start_line = 1;
        
        for text in text_chunks {
            if current_chunk_text.len() + text.len() > 1000 && !current_chunk_text.is_empty() {
                    let mut metadata: std::collections::HashMap<String, serde_json::Value> = match &base_metadata {
                        Some(serde_json::Value::Object(map)) => map.clone().into_iter().collect(),
                        _ => std::collections::HashMap::new(),
                    };

                    metadata.insert("source".to_string(), serde_json::Value::String(path.to_string_lossy().to_string()));

                    chunks.push(Chunk {
                        id: Uuid::new_v4().to_string(),
                        document_id: "".to_string(),
                        content: current_chunk_text.clone(),
                        vector: None,
                        metadata,
                        page_num: None,
                        char_start: 0,
                        char_end: 0,
                        start_line: Some(start_line),
                        end_line: Some(start_line),
                    });
                    current_chunk_text.clear();
                    start_line += 1;
            }
            if !current_chunk_text.is_empty() {
                current_chunk_text.push('\n');
            }
            current_chunk_text.push_str(&text);
        }

        if !current_chunk_text.is_empty() {
            let mut metadata: std::collections::HashMap<String, serde_json::Value> = match &base_metadata {
                Some(serde_json::Value::Object(map)) => map.clone().into_iter().collect(),
                _ => std::collections::HashMap::new(),
            };

            metadata.insert("source".to_string(), serde_json::Value::String(path.to_string_lossy().to_string()));

            chunks.push(Chunk {
                id: Uuid::new_v4().to_string(),
                document_id: "".to_string(),
                content: current_chunk_text,
                vector: None,
                metadata,
                page_num: None,
                char_start: 0,
                char_end: 0,
                start_line: Some(start_line),
                end_line: Some(start_line),
            });
        }

        Ok(chunks)
    }

    fn supported_extensions(&self) -> Vec<&str> {
        vec!["yaml", "yml"]
    }
}

impl Default for YamlParser {
    fn default() -> Self {
        Self::new()
    }
}
