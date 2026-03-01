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

    fn flatten_value_iterative(&self, root: &Value, chunks: &mut Vec<String>) {
        // Use a stack for iterative traversal to avoid recursion limits
        // (prefix, value)
        let mut stack = vec![("".to_string(), root)];

        // Safety limit to prevent memory exhaustion on malicious inputs
        let mut processed_nodes = 0;
        const MAX_NODES: usize = 100_000;

        while let Some((prefix, value)) = stack.pop() {
            processed_nodes += 1;
            if processed_nodes > MAX_NODES {
                chunks.push(format!(
                    "...[TRUNCATED: JSON structure exceeded {} nodes]...",
                    MAX_NODES
                ));
                break;
            }

            match value {
                Value::Object(map) => {
                    // Push in reverse order so they are popped in natural order
                    for (k, v) in map.iter().rev() {
                        let new_prefix = if prefix.is_empty() {
                            k.clone()
                        } else {
                            format!("{}.{}", prefix, k)
                        };
                        stack.push((new_prefix, v));
                    }
                }
                Value::Array(arr) => {
                    // Push in reverse order so they are popped in natural order
                    for (i, v) in arr.iter().enumerate().rev() {
                        let new_prefix = format!("{}[{}]", prefix, i);
                        stack.push((new_prefix, v));
                    }
                }
                _ => {
                    // Leaf node
                    if !prefix.is_empty() {
                        // Avoid deep string cloning if possible, but for now format is fine
                        chunks.push(format!("{}: {}", prefix, value));
                    }
                }
            }
        }
    }
}

use async_trait::async_trait;

#[async_trait]
impl Parser for JsonParser {
    async fn parse(
        &self,
        content: &str,
        path: &Path,
        base_metadata: Option<Value>,
    ) -> Result<Vec<Chunk>> {
        // Try standard JSON first (fastest path)
        let json: Value = match serde_json::from_str(content) {
            Ok(v) => v,
            Err(_) => {
                // Fallback to JSON5 for files with comments (tsconfig.json, .eslintrc.json, etc.)
                if crate::output::OUTPUT.is_interactive {
                    eprintln!("Notice: Standard JSON parse failed for '{}' (trailing comma/comments detected). Falling back to JSON5 parser...", path.display());
                }
                json5::from_str(content)
                    .map_err(|e| anyhow::anyhow!("JSON5 parse also failed: {}", e))?
            }
        };

        // 1. Heuristic: Adaptive Chunking Strategy
        // We want to avoid creating thousands of tiny chunks for large files.
        // Target around 500 chunks per file maximum to keep embedding pipeline healthy.
        // Base chunk size is 1000 chars.
        // If file is 2MB, 2MB / 500 = 4000 chars per chunk.

        let content_len = content.len();
        let target_chunk_count = 500;
        let adaptive_chunk_size = std::cmp::max(1000, content_len / target_chunk_count);

        let mut text_chunks = Vec::new();
        // Use iterative flattening
        self.flatten_value_iterative(&json, &mut text_chunks);

        let mut chunks = Vec::new();
        let mut current_chunk_text = String::new();
        let mut start_line = 1;

        for text in text_chunks {
            // Use adaptive_chunk_size instead of fixed 1000
            if current_chunk_text.len() + text.len() > adaptive_chunk_size
                && !current_chunk_text.is_empty()
            {
                let mut metadata: std::collections::HashMap<String, serde_json::Value> =
                    match &base_metadata {
                        Some(Value::Object(map)) => map.clone().into_iter().collect(),
                        _ => std::collections::HashMap::new(),
                    };

                metadata.insert(
                    "source".to_string(),
                    serde_json::Value::String(path.to_string_lossy().to_string()),
                );
                metadata.insert(
                    "calculated_chunk_size".to_string(),
                    serde_json::json!(adaptive_chunk_size),
                );

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
            let mut metadata: std::collections::HashMap<String, serde_json::Value> =
                match &base_metadata {
                    Some(Value::Object(map)) => map.clone().into_iter().collect(),
                    _ => std::collections::HashMap::new(),
                };

            metadata.insert(
                "source".to_string(),
                serde_json::Value::String(path.to_string_lossy().to_string()),
            );
            metadata.insert(
                "calculated_chunk_size".to_string(),
                serde_json::json!(adaptive_chunk_size),
            );

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
