use crate::parsers::Parser;
use crate::types::Chunk;
use anyhow::{Context, Result};
use async_trait::async_trait;
use serde::de::{SeqAccess, Visitor};
use serde::Deserializer;
use serde_json::Value;
use std::fs::File;
use std::io::BufReader;
use std::path::Path;
use uuid::Uuid;

/// Streaming JSON Parser
///
/// Designed for large files (logs, chat history).
/// Iterates over a root-level JSON array item by item, reducing memory usage from O( FileSize ) to O( ItemSize ).
pub struct StreamingJsonParser;

impl StreamingJsonParser {
    pub fn new() -> Self {
        Self
    }

    fn stream_file(&self, path: &Path, base_metadata: &Option<Value>) -> Result<Vec<Chunk>> {
        let file = File::open(path).context("Failed to open file for streaming")?;
        let reader = BufReader::new(file);

        let mut deserializer = serde_json::Deserializer::from_reader(reader);

        struct ChunkVisitor {
            path: String,
            base_metadata: Option<Value>,
        }

        impl<'de> Visitor<'de> for ChunkVisitor {
            type Value = Vec<Chunk>;

            fn expecting(&self, formatter: &mut std::fmt::Formatter) -> std::fmt::Result {
                formatter.write_str("a JSON array of objects")
            }

            fn visit_seq<A>(self, mut seq: A) -> Result<Self::Value, A::Error>
            where
                A: SeqAccess<'de>,
            {
                let mut chunks = Vec::new();
                let mut item_count = 0;

                while let Some(value) = seq.next_element::<Value>()? {
                    let text = match serde_json::to_string_pretty(&value) {
                        Ok(s) => s,
                        Err(_) => continue,
                    };

                    if text.trim().is_empty() {
                        continue;
                    }

                    let mut metadata: std::collections::HashMap<String, Value> =
                        match &self.base_metadata {
                            Some(Value::Object(map)) => map.clone().into_iter().collect(),
                            _ => std::collections::HashMap::new(),
                        };

                    metadata.insert("source".to_string(), Value::String(self.path.clone()));
                    metadata.insert("stream_index".to_string(), serde_json::json!(item_count));

                    if let Some(ts) = value
                        .get("timestamp")
                        .or(value.get("time"))
                        .or(value.get("created_at"))
                    {
                        if let Some(ts_str) = ts.as_str() {
                            metadata
                                .insert("timestamp".to_string(), Value::String(ts_str.to_string()));
                        }
                    }

                    chunks.push(Chunk {
                        id: Uuid::new_v4().to_string(),
                        document_id: "".to_string(),
                        content: text,
                        vector: None,
                        metadata: metadata.into_iter().collect(),
                        page_num: None,
                        char_start: 0,
                        char_end: 0,
                        start_line: None,
                        end_line: None,
                    });
                    item_count += 1;
                }

                Ok(chunks)
            }
        }

        deserializer
            .deserialize_seq(ChunkVisitor {
                path: path.to_string_lossy().to_string(),
                base_metadata: base_metadata.clone(),
            })
            .map_err(|e| anyhow::anyhow!("Streaming parse failed (expected array): {}", e))
    }
}

#[async_trait]
impl Parser for StreamingJsonParser {
    async fn parse(
        &self,
        _content: &str,
        path: &Path,
        base_metadata: Option<Value>,
    ) -> Result<Vec<Chunk>> {
        let path_buf = path.to_path_buf();
        let meta = base_metadata.clone();
        let myself = Self {};

        let chunks =
            tokio::task::spawn_blocking(move || myself.stream_file(&path_buf, &meta)).await??;

        Ok(chunks)
    }

    fn supported_extensions(&self) -> Vec<&str> {
        vec!["json", "jsonl", "ndjson"]
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use tempfile::NamedTempFile;

    #[tokio::test]
    async fn test_streaming_json_parser() -> Result<()> {
        let mut file = NamedTempFile::new()?;
        let json_content = r#"[
            {"id": 1, "text": "Item 1", "timestamp": "2023-01-01T10:00:00Z"},
            {"id": 2, "text": "Item 2"},
            {"id": 3, "text": "Item 3", "nested": {"foo": "bar"}}
        ]"#;
        write!(file, "{}", json_content)?;

        let parser = StreamingJsonParser::new();
        let path = file.path();

        let chunks = parser.parse("", path, None).await?;

        assert_eq!(chunks.len(), 3);

        let c1 = &chunks[0];
        assert!(c1.content.contains("Item 1"));
        assert_eq!(c1.metadata["stream_index"], 0);
        assert_eq!(c1.metadata["timestamp"], "2023-01-01T10:00:00Z");

        let c3 = &chunks[2];
        assert!(c3.content.contains("Item 3"));
        assert!(c3.content.contains("nested"));

        Ok(())
    }

    #[tokio::test]
    async fn test_streaming_fails_on_non_array() {
        let mut file = NamedTempFile::new().unwrap();
        let json_content = r#"{"root": "object"}"#;
        write!(file, "{}", json_content).unwrap();

        let parser = StreamingJsonParser::new();
        let result = parser.parse("", file.path(), None).await;

        // It should fail now because we strictly demand a Sequence
        assert!(result.is_err());
    }
}
