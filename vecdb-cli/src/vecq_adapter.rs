use anyhow::Result;
use async_trait::async_trait;
use std::path::Path;
use uuid::Uuid;
use vecdb_common::FileType;
use vecdb_core::parsers::ParserFactory;
use vecdb_core::{parsers::Parser, types::Chunk};
use vecq::DocumentElement;

/// Adapter to use vecq parsers within vecdb ingestion
pub struct VecqParserAdapter {
    file_type: FileType,
}

impl VecqParserAdapter {
    pub fn new(file_type: FileType) -> Self {
        Self { file_type }
    }

    /// Recursively flatten vecq elements into chunks
    fn flatten_elements(
        &self,
        elements: &[DocumentElement],
        chunks: &mut Vec<Chunk>,
        path: &Path,
        base_metadata: &serde_json::Value,
        parent_trail: &str,
        doc_id: &str,
    ) {
        for element in elements {
            // Basic metadata
            let mut metadata = base_metadata.as_object().cloned().unwrap_or_default();
            metadata.insert(
                "element_type".to_string(),
                serde_json::Value::String(element.element_type.to_string()),
            );
            if let Some(name) = &element.name {
                metadata.insert("name".to_string(), serde_json::Value::String(name.clone()));
            }
            metadata.insert(
                "line_start".to_string(),
                serde_json::json!(element.line_start),
            );
            metadata.insert("line_end".to_string(), serde_json::json!(element.line_end));
            metadata.insert(
                "source".to_string(),
                serde_json::Value::String(path.to_string_lossy().to_string()),
            );
            metadata.insert(
                "file_type".to_string(),
                serde_json::Value::String(self.file_type.to_string()),
            );

            // Extract Semantic Intent (Phase 3)
            if let Some(doc) = element.attributes.get("docstring") {
                metadata.insert("docstring".to_string(), doc.clone());
                metadata.insert("intent".to_string(), doc.clone()); // Alias for semantic alignment
            }
            if let Some(vis) = element.attributes.get("visibility") {
                metadata.insert("visibility".to_string(), vis.clone());
            }

            // Crumbtrail (Phase 1)
            let current_trail = if parent_trail.is_empty() {
                element
                    .name
                    .clone()
                    .unwrap_or(element.element_type.to_string())
            } else {
                format!(
                    "{}::{}",
                    parent_trail,
                    element
                        .name
                        .as_deref()
                        .unwrap_or(&element.element_type.to_string())
                )
            };
            metadata.insert(
                "crumbtrail".to_string(),
                serde_json::Value::String(current_trail.clone()),
            );

            // Redundancy Check: If it has children, only add it if it has "meat" (unique content)
            let children_len: usize = element.children.iter().map(|c| c.content.len()).sum();
            let is_fully_covered =
                !element.children.is_empty() && (children_len > (element.content.len() * 9 / 10));

            let should_index = if is_fully_covered {
                matches!(
                    element.element_type.to_string().as_str(),
                    "function" | "method" | "class" | "struct" | "interface" | "trait"
                ) || element.attributes.contains_key("docstring")
            } else {
                true
            };

            if should_index {
                // Create deterministic ID based on doc ID + crumbtrail + content hash for maximum stability
                let content_hash = calculate_hash(&element.content);
                let chunk_seed = format!("{}::{}::{}", doc_id, current_trail, content_hash);
                let chunk_id =
                    Uuid::new_v5(&uuid::Uuid::NAMESPACE_URL, chunk_seed.as_bytes()).to_string();

                chunks.push(Chunk {
                    id: chunk_id,
                    document_id: doc_id.to_string(),
                    content: element.content.clone(),
                    vector: None,
                    metadata: metadata.into_iter().collect(),
                    page_num: None,
                    char_start: 0,
                    char_end: element.content.len(),
                    start_line: Some(element.line_start),
                    end_line: Some(element.line_end),
                });
            }

            // Recurse
            if !element.children.is_empty() {
                self.flatten_elements(
                    &element.children,
                    chunks,
                    path,
                    base_metadata,
                    &current_trail,
                    doc_id,
                );
            }
        }
    }
}

fn calculate_hash(content: &str) -> String {
    use sha2::{Digest, Sha256};
    let mut hasher = Sha256::new();
    hasher.update(content.as_bytes());
    let result = hasher.finalize();
    result
        .iter()
        .map(|b| format!("{:02x}", b))
        .collect::<String>()
}

#[async_trait]
impl Parser for VecqParserAdapter {
    async fn parse(
        &self,
        content: &str,
        path: &Path,
        base_metadata: Option<serde_json::Value>,
    ) -> Result<Vec<Chunk>> {
        // Use vecq to parse the file
        // Note: vecq::parse_file takes &str content and FileType
        let parsed_doc = vecq::parse_file(content, self.file_type).await?;

        // Generate a document ID (could be from file path hash)
        // Here we can use file path + optional commit sha from metadata
        let doc_seed = path.to_string_lossy().to_string();
        let doc_id = Uuid::new_v5(&uuid::Uuid::NAMESPACE_URL, doc_seed.as_bytes()).to_string();

        let mut chunks = Vec::new();
        let base_meta = base_metadata.unwrap_or(serde_json::json!({}));

        // Flatten the parsed document hierarchy into a list of chunks
        self.flatten_elements(
            &parsed_doc.elements,
            &mut chunks,
            path,
            &base_meta,
            "",
            &doc_id,
        );

        Ok(chunks)
    }

    fn supported_extensions(&self) -> Vec<&str> {
        // This adapter is generic, supported extensions are handled by the factory
        vec![]
    }
}

/// Factory that produces vecq parsers
pub struct VecqParserFactory;

impl ParserFactory for VecqParserFactory {
    fn get_parser(&self, file_type: FileType) -> Option<Box<dyn Parser>> {
        // Use vecq adapter for supported code types
        // vecdb-core might also have a BuiltinParserFactory for JSON/YAML.
        // We should chain them or this one should handle everything?
        // Since this is injected, it should probably be comprehensive or we compose them in main.
        // For simplicity, let's handle vecq types here and let core handle others?
        // Wait, core uses this factory. If this returns None, core does nothing.
        // So this factory should probably wrap or duplicate core's builtin logic or we use a CompositeFactory.

        if file_type.is_supported() {
            match file_type {
                FileType::Json | FileType::Text | FileType::Toml => {
                    // For now, let's just use vecdb-core's built-ins for these?
                    // But vecdb-core builtin parsers are simple text chunkers.
                    // The requirement is to use vecq for smart ingestion.
                    // vecq also supports JSON, Toml, Text.
                    // Let's defer ALL parsing to vecq adapter if file type is supported!
                    Some(Box::new(VecqParserAdapter::new(file_type)))
                }
                _ => Some(Box::new(VecqParserAdapter::new(file_type))),
            }
        } else {
            None
        }
    }

    fn get_streaming_parser(&self, file_type: FileType) -> Option<Box<dyn Parser>> {
        match file_type {
            FileType::Json => Some(Box::new(
                vecdb_core::parsers::streaming_json::StreamingJsonParser::new(),
            )),
            _ => None,
        }
    }
}
