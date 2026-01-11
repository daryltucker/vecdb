use crate::error::{VecqResult, VecqError};
use crate::parser::Parser;
use crate::types::{DocumentElement, ParsedDocument, DocumentMetadata, ElementType};
use async_trait::async_trait;
use std::path::PathBuf;
use vecdb_common::LineCounter;

#[derive(Clone)]
pub struct RustTreeSitterParser;

impl Default for RustTreeSitterParser {
    fn default() -> Self {
        Self::new()
    }
}

impl RustTreeSitterParser {
    pub fn new() -> Self {
        Self
    }

}

#[async_trait]
impl Parser for RustTreeSitterParser {
    async fn parse(&self, content: &str) -> VecqResult<ParsedDocument> {
        let mut parser = tree_sitter::Parser::new();
        parser.set_language(&tree_sitter_rust::LANGUAGE.into())
            .map_err(|e| VecqError::parse_error(PathBuf::from("unknown"), 0, format!("Failed to load Rust language: {}", e), None::<std::io::Error>))?;

        let tree = parser.parse(content, None)
            .ok_or_else(|| VecqError::parse_error(PathBuf::from("unknown"), 0, "Failed to parse content".to_string(), None::<std::io::Error>))?;

        let root_node = tree.root_node();
        let mut elements = Vec::new();
        let source_bytes = content.as_bytes();
        let _line_counter = LineCounter::new(content);

        // State for docstring accumulation
        let mut pending_docs = Vec::new();

        // Cursor to walk the tree
        let mut cursor = root_node.walk();
        for child in root_node.children(&mut cursor) {
             match child.kind() {
                "line_comment" => {
                    let text = child.utf8_text(source_bytes).unwrap_or("");
                    if text.starts_with("///") {
                         pending_docs.push(text.trim_start_matches("///").trim().to_string());
                    } else if text.starts_with("//!") {
                         // Module level docs, ignore for function attachment for now
                    } else {
                        // Regular comment
                    }
                },
                "function_item" => {
                    let name_node = child.child_by_field_name("name");
                    let name = name_node.and_then(|n: tree_sitter::Node| n.utf8_text(source_bytes).ok()).map(|s: &str| s.to_string());
                    
                    // Extract visibility (optional)
                    // Tree-sitter structure: [visibility_modifier] fn name ...
                    // If exists, it's the first named child usually? Or just search children.
                    let mut visibility = "private".to_string();
                    let mut sig_start = child.start_byte();
                    
                    // Iterate children to find visibility and fn keyword
                    let mut walker = child.walk();
                    for grandchild in child.children(&mut walker) {
                        if grandchild.kind() == "visibility_modifier" {
                            visibility = grandchild.utf8_text(source_bytes).unwrap_or("private").to_string();
                            sig_start = grandchild.end_byte(); // Signature starts after visibility
                        } else if grandchild.kind() == "fn" {
                            // If we hit fn and hadn't set sig_start (i.e. private), signature starts here
                            // Actually, if we didn't find visibility, signature starts at child.start_byte() which is 'fn' or 'async' or attributes?
                            // Safest ref: Signature is everything from `fn` (or `async fn`) onwards?
                            // Syn seems to output "fn name(...)". 
                            // Let's refine: Signature = (async) fn ...
                        }
                    }

                    // Signature extraction refined
                    let body_node = child.child_by_field_name("body");
                    let sig_end = body_node.map(|n: tree_sitter::Node| n.start_byte()).unwrap_or(child.end_byte());
                    
                    // Adjust sig_start to skip visibility + whitespace
                    // We need to look at bytes from sig_start to sig_end
                    let raw_sig = &source_bytes[sig_start..sig_end];
                    // Trim leading whitespace (from skipping visibility)
                    let signature = String::from_utf8_lossy(raw_sig).trim().to_string();
                    
                    // Normalize whitespace to match Syn's likely behavior (collapse spaces)
                    // "fn  foo" -> "fn foo"
                    let signature = signature.split_whitespace().collect::<Vec<_>>().join(" ");

                    let start_line = child.start_position().row + 1;
                    let end_line = child.end_position().row + 1;
                    let content_text = child.utf8_text(source_bytes).unwrap_or("").to_string();

                    let mut element = DocumentElement::new(
                        ElementType::Function,
                        name,
                        content_text,
                        start_line,
                        end_line,
                    );

                    // Attach attributes
                    element.attributes.insert_generic("signature".to_string(), serde_json::Value::String(signature));
                    element.attributes.insert_generic("visibility".to_string(), serde_json::Value::String(visibility));
                    
                    if !pending_docs.is_empty() {
                         let doc_text = pending_docs.join("\n");
                         element.attributes.insert_generic("docstring".to_string(), serde_json::Value::String(doc_text));
                         pending_docs.clear();
                    }

                    elements.push(element);
                },
                _ => {
                    // If we hit any other node (struct, impl, whitespace is skipped), 
                    // we should probably clear pending docs unless it's an attribute
                    // But actually, attributes appear inside the item in TS usually? 
                    // Or before?
                    // Safe bet: clear pending docs if it's not a comment or attribute
                    if child.kind() != "attribute_item" {
                        pending_docs.clear();
                    }
                }
             }
        }

        let metadata = DocumentMetadata::new(PathBuf::from("unknown"), content.len() as u64)
            .with_line_count(content)
            .with_file_type(crate::types::FileType::Rust);

        Ok(ParsedDocument::new(metadata).add_elements(elements))
    }

    fn file_extensions(&self) -> &[&str] {
        &["rs"]
    }

    fn language_name(&self) -> &str {
        "Rust (Tree-sitter)"
    }
}
