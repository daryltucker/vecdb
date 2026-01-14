use crate::error::{VecqResult, VecqError};
use crate::parser::Parser;
use crate::types::{DocumentElement, ParsedDocument, DocumentMetadata, ElementType, RustAttributes, ElementAttributes};
use async_trait::async_trait;
use std::path::PathBuf;
use std::collections::HashMap;

#[derive(Clone)]
pub struct RustParser;

impl Default for RustParser {
    fn default() -> Self {
        Self::new()
    }
}

impl RustParser {
    pub fn new() -> Self {
        Self
    }

    fn extract_visibility(&self, node: &tree_sitter::Node, source: &[u8]) -> String {
        let mut cursor = node.walk();
        for child in node.children(&mut cursor) {
            if child.kind() == "visibility_modifier" {
                return child.utf8_text(source).unwrap_or("private").to_string();
            }
        }
        "private".to_string()
    }

    fn extract_signature(&self, node: &tree_sitter::Node, source: &[u8]) -> String {
        // Simple signature extraction: first line or up to body
        let mut end_byte = node.end_byte();
        
        if let Some(body) = node.child_by_field_name("body") {
            end_byte = body.start_byte();
        } else if let Some(block) = node.child_by_field_name("block") { 
             end_byte = block.start_byte();
        }

        let text = &source[node.start_byte()..end_byte];
        String::from_utf8_lossy(text).trim().to_string().split_whitespace().collect::<Vec<_>>().join(" ")
    }

    fn process_nodes(
        &self,
        node: tree_sitter::Node,
        source: &[u8],
        pending_comments: &mut Vec<String>,
    ) -> Vec<DocumentElement> {
        let mut elements = Vec::new();
        let mut cursor = node.walk();

        for child in node.children(&mut cursor) {
            let kind = child.kind();
            match kind {
                "line_comment" | "block_comment" => {
                    let text = child.utf8_text(source).unwrap_or("").trim();
                    pending_comments.push(text.to_string());
                }
                "function_item" => {
                    let name = child
                        .child_by_field_name("name")
                        .and_then(|n| n.utf8_text(source).ok().map(|s| s.to_string()));

                    let mut element = DocumentElement::new(
                        ElementType::Function,
                        name,
                        child.utf8_text(source).unwrap_or("").to_string(),
                        child.start_position().row + 1,
                        child.end_position().row + 1,
                    );

                    let mut rust_attr = RustAttributes {
                        visibility: self.extract_visibility(&child, source),
                        other: HashMap::new(),
                    };
                    rust_attr.other.insert(
                        "signature".to_string(),
                        serde_json::Value::String(self.extract_signature(&child, source)),
                    );

                    if !pending_comments.is_empty() {
                        rust_attr.other.insert(
                            "docstring".to_string(),
                            serde_json::Value::String(pending_comments.join("\n")),
                        );
                        pending_comments.clear();
                    }
                    element.attributes = ElementAttributes::Rust(rust_attr);
                    elements.push(element);
                }
                "struct_item" => {
                    let name = child
                        .child_by_field_name("name")
                        .and_then(|n| n.utf8_text(source).ok().map(|s| s.to_string()));

                    let mut element = DocumentElement::new(
                        ElementType::Struct,
                        name,
                        child.utf8_text(source).unwrap_or("").to_string(),
                        child.start_position().row + 1,
                        child.end_position().row + 1,
                    );

                    let mut rust_attr = RustAttributes {
                        visibility: self.extract_visibility(&child, source),
                        other: HashMap::new(),
                    };

                    if !pending_comments.is_empty() {
                        rust_attr.other.insert(
                            "docstring".to_string(),
                            serde_json::Value::String(pending_comments.join("\n")),
                        );
                        pending_comments.clear();
                    }
                    element.attributes = ElementAttributes::Rust(rust_attr);
                    elements.push(element);
                }
                "enum_item" => {
                    let name = child
                        .child_by_field_name("name")
                        .and_then(|n| n.utf8_text(source).ok().map(|s| s.to_string()));

                    let mut element = DocumentElement::new(
                        ElementType::Enum,
                        name,
                        child.utf8_text(source).unwrap_or("").to_string(),
                        child.start_position().row + 1,
                        child.end_position().row + 1,
                    );

                    let mut rust_attr = RustAttributes {
                        visibility: self.extract_visibility(&child, source),
                        other: HashMap::new(),
                    };

                    if !pending_comments.is_empty() {
                        rust_attr.other.insert(
                            "docstring".to_string(),
                            serde_json::Value::String(pending_comments.join("\n")),
                        );
                        pending_comments.clear();
                    }
                    element.attributes = ElementAttributes::Rust(rust_attr);
                    elements.push(element);
                }
                "trait_item" => {
                    let name = child
                        .child_by_field_name("name")
                        .and_then(|n| n.utf8_text(source).ok().map(|s| s.to_string()));

                    let mut element = DocumentElement::new(
                        ElementType::Trait,
                        name,
                        child.utf8_text(source).unwrap_or("").to_string(),
                        child.start_position().row + 1,
                        child.end_position().row + 1,
                    );

                    element.attributes.insert_generic(
                        "visibility".to_string(),
                        serde_json::Value::String(self.extract_visibility(&child, source)),
                    );

                    if !pending_comments.is_empty() {
                        element.attributes.insert_generic(
                            "docstring".to_string(),
                            serde_json::Value::String(pending_comments.join("\n")),
                        );
                        pending_comments.clear();
                    }

                    if let Some(body) = child.child_by_field_name("body") {
                        let mut body_comments = Vec::new();
                        let children = self.process_nodes(body, source, &mut body_comments);
                        element = element.with_children(children);
                    }

                    elements.push(element);
                }
                "impl_item" => {
                    let type_node = child.child_by_field_name("type");
                    let trait_node = child.child_by_field_name("trait");

                    let type_name = type_node
                        .and_then(|n| n.utf8_text(source).ok())
                        .unwrap_or("Unknown")
                        .to_string();

                    let name = if let Some(t) = trait_node {
                        let t_name = t.utf8_text(source).unwrap_or("Unknown");
                        format!("impl {} for {}", t_name, type_name)
                    } else {
                        format!("impl {}", type_name)
                    };

                    let mut element = DocumentElement::new(
                        ElementType::Implementation,
                        Some(name),
                        child.utf8_text(source).unwrap_or("").to_string(),
                        child.start_position().row + 1,
                        child.end_position().row + 1,
                    );

                    if !pending_comments.is_empty() {
                        element.attributes.insert_generic(
                            "docstring".to_string(),
                            serde_json::Value::String(pending_comments.join("\n")),
                        );
                        pending_comments.clear();
                    }

                    if let Some(body) = child.child_by_field_name("body") {
                        let mut body_comments = Vec::new();
                        let children = self.process_nodes(body, source, &mut body_comments);
                        element = element.with_children(children);
                    }

                    elements.push(element);
                }
                "mod_item" => {
                    let name = child
                        .child_by_field_name("name")
                        .and_then(|n| n.utf8_text(source).ok().map(|s| s.to_string()));

                    let mut element = DocumentElement::new(
                        ElementType::Module,
                        name,
                        child.utf8_text(source).unwrap_or("").to_string(),
                        child.start_position().row + 1,
                        child.end_position().row + 1,
                    );

                    element.attributes.insert_generic(
                        "visibility".to_string(),
                        serde_json::Value::String(self.extract_visibility(&child, source)),
                    );

                    if !pending_comments.is_empty() {
                        element.attributes.insert_generic(
                            "docstring".to_string(),
                            serde_json::Value::String(pending_comments.join("\n")),
                        );
                        pending_comments.clear();
                    }

                    if let Some(body) = child.child_by_field_name("body") {
                        let mut body_comments = Vec::new();
                        let children = self.process_nodes(body, source, &mut body_comments);
                        element = element.with_children(children);
                    }

                    elements.push(element);
                }
                "use_declaration" => {
                    let name = child
                        .child_by_field_name("argument")
                        .and_then(|n| n.utf8_text(source).ok().map(|s| s.to_string()));

                    let mut element = DocumentElement::new(
                        ElementType::Import,
                        name,
                        child.utf8_text(source).unwrap_or("").to_string(),
                        child.start_position().row + 1,
                        child.end_position().row + 1,
                    );
                    element.attributes.insert_generic(
                        "visibility".to_string(),
                        serde_json::Value::String(self.extract_visibility(&child, source)),
                    );
                    elements.push(element);
                    pending_comments.clear();
                }
                "type_item" => {
                    let name = child
                        .child_by_field_name("name")
                        .and_then(|n| n.utf8_text(source).ok().map(|s| s.to_string()));

                    let mut element = DocumentElement::new(
                        ElementType::TypeAlias,
                        name,
                        child.utf8_text(source).unwrap_or("").to_string(),
                        child.start_position().row + 1,
                        child.end_position().row + 1,
                    );

                    element.attributes.insert_generic(
                        "visibility".to_string(),
                        serde_json::Value::String(self.extract_visibility(&child, source)),
                    );

                    if !pending_comments.is_empty() {
                        element.attributes.insert_generic(
                            "docstring".to_string(),
                            serde_json::Value::String(pending_comments.join("\n")),
                        );
                        pending_comments.clear();
                    }
                    elements.push(element);
                }
                "const_item" | "static_item" => {
                    let name = child
                        .child_by_field_name("name")
                        .and_then(|n| n.utf8_text(source).ok().map(|s| s.to_string()));
                    let element_type = if kind == "const_item" {
                        ElementType::Constant
                    } else {
                        ElementType::Variable
                    };
                    let mut element = DocumentElement::new(
                        element_type,
                        name,
                        child.utf8_text(source).unwrap_or("").to_string(),
                        child.start_position().row + 1,
                        child.end_position().row + 1,
                    );
                    if !pending_comments.is_empty() {
                        element.attributes.insert_generic(
                            "docstring".to_string(),
                            serde_json::Value::String(pending_comments.join("\n")),
                        );
                        pending_comments.clear();
                    }
                    elements.push(element);
                }
                "declaration_list" => {
                    elements.extend(self.process_nodes(child, source, pending_comments));
                }
                _ => {
                    if child.is_named() && kind != "attribute_item" && kind != "visibility_modifier" {
                        pending_comments.clear();
                    }
                }
            }
        }
        elements
    }
}

#[async_trait]
impl Parser for RustParser {
    async fn parse(&self, content: &str) -> VecqResult<ParsedDocument> {
        let mut parser = tree_sitter::Parser::new();
        parser.set_language(&tree_sitter_rust::LANGUAGE.into())
            .map_err(|e| VecqError::parse_error(PathBuf::from("unknown"), 0, format!("Failed to load Rust language: {}", e), None::<std::io::Error>))?;

        let tree = parser.parse(content, None)
            .ok_or_else(|| VecqError::parse_error(PathBuf::from("unknown"), 0, "Failed to parse content".to_string(), None::<std::io::Error>))?;

        let root_node = tree.root_node();
        let source_bytes = content.as_bytes();
        let mut pending_comments = Vec::new();

        let elements = self.process_nodes(root_node, source_bytes, &mut pending_comments);

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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::ElementType;

    #[tokio::test]
    async fn test_parse_complex_imports() {
        let parser = RustParser::new();
        let content = r#"
        use std::collections::HashMap;
        use crate::types::{TypeA, TypeB};
        use std::io::Result as IoResult;
        "#;

        let result = parser.parse(content).await.unwrap();
        let imports: Vec<_> = result.elements.iter()
            .filter(|e| e.element_type == ElementType::Import)
            .collect();

        // 1. HashMap should be named
        let hashmap = imports.iter().find(|i| i.content.contains("HashMap")).unwrap();
        assert_eq!(hashmap.name, Some("std::collections::HashMap".to_string()));
    }
}
