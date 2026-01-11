use crate::parsers::Parser;
use crate::types::Chunk;
use anyhow::Result;
use serde_json::Value;
use std::path::Path;
use uuid::Uuid;

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

    fn flatten_value(&self, value: &Value, prefix: String, chunks: &mut Vec<String>) {
        match value {
            Value::Object(map) => {
                for (k, v) in map {
                    let new_prefix = if prefix.is_empty() {
                        k.clone()
                    } else {
                        format!("{}.{}", prefix, k)
                    };
                    self.flatten_value(v, new_prefix, chunks);
                }
            }
            Value::Array(arr) => {
                for (i, v) in arr.iter().enumerate() {
                    let new_prefix = format!("{}[{}]", prefix, i);
                    self.flatten_value(v, new_prefix, chunks);
                }
            }
            _ => {
                // Leaf node
                if !prefix.is_empty() {
                    chunks.push(format!("{}: {}", prefix, value));
                }
            }
        }
    }
}

use async_trait::async_trait;

#[async_trait]
impl Parser for JsonParser {
    async fn parse(&self, content: &str, path: &Path, base_metadata: Option<Value>) -> Result<Vec<Chunk>> {
        // Try standard JSON first (fastest path)
        let json: Value = match serde_json::from_str(content) {
            Ok(v) => v,
            Err(_) => {
                // Fallback to JSON5 for files with comments (tsconfig.json, .eslintrc.json, etc.)
                // Fallback to JSON5 for files with comments (tsconfig.json, .eslintrc.json, etc.)
                if crate::output::OUTPUT.is_interactive {
                    eprintln!("Notice: Standard JSON parse failed for '{}' (trailing comma/comments detected). Falling back to JSON5 parser...", path.display());
                }
                json5::from_str(content)
                    .map_err(|e| anyhow::anyhow!("JSON5 parse also failed: {}", e))?
            }
        };
        
        let mut text_chunks = Vec::new();
        self.flatten_value(&json, "".to_string(), &mut text_chunks);

        let mut chunks = Vec::new();
        let mut current_chunk_text = String::new();
        let mut start_line = 1;
        
        for text in text_chunks {
            if current_chunk_text.len() + text.len() > 1000 
                && !current_chunk_text.is_empty() {
                    let mut metadata: std::collections::HashMap<String, serde_json::Value> = match &base_metadata {
                        Some(Value::Object(map)) => map.clone().into_iter().collect(),
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
                Some(Value::Object(map)) => map.clone().into_iter().collect(),
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
        vec!["json"]
    }
}
