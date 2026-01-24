use crate::error::{VecqError, VecqResult};
use crate::parser::Parser;
use crate::types::{
    DocumentElement, DocumentMetadata, ElementAttributes, ElementType, ParsedDocument,
    RustAttributes, UsageAttributes,
};
use async_trait::async_trait;
use std::collections::HashMap;
use std::path::PathBuf;

#[derive(Clone)]
pub struct RustParser {
    enable_usages: bool,
}

impl Default for RustParser {
    fn default() -> Self {
        Self::new()
    }
}

impl RustParser {
    pub fn new() -> Self {
        Self {
            enable_usages: false,
        }
    }

    /// Enable or disable usage/reference detection
    pub fn with_usages(mut self, enable: bool) -> Self {
        self.enable_usages = enable;
        self
    }

    fn extract_visibility(&self, node: &tree_sitter::Node, source: &[u8]) -> String {
        for child in node.children(&mut node.walk()) {
            if child.kind() == "visibility_modifier" {
                return child.utf8_text(source).unwrap_or("private").to_string();
            }
        }
        "private".to_string()
    }

    fn extract_use_path(&self, node: &tree_sitter::Node, source: &[u8]) -> Option<String> {
        // Find the path in the use declaration
        // Tree-sitter Rust grammar structure: use_declaration -> argument -> path/identifier
        if let Some(argument) = node.child_by_field_name("argument") {
            return self.extract_full_use_path(&argument, source);
        }
        None
    }

    fn extract_full_use_path(&self, node: &tree_sitter::Node, source: &[u8]) -> Option<String> {
        // Extract the full use path including aliases
        // Handle cases like: std::collections::HashMap, crate::types::{TypeA, TypeB}, std::io::Result as IoResult

        let mut cursor = node.walk();
        let mut path_parts = Vec::new();
        let mut alias_part = None;

        for child in node.children(&mut cursor) {
            match child.kind() {
                "identifier" => {
                    if let Ok(text) = child.utf8_text(source) {
                        if alias_part.is_none() {
                            // This might be an alias (like IoResult)
                            alias_part = Some(text.to_string());
                        }
                        path_parts.push(text.to_string());
                    }
                }
                "scoped_identifier" => {
                    if let Some(sub_path) = self.extract_path_from_node(&child, source) {
                        path_parts.push(sub_path);
                    }
                }
                "use_as_clause" => {
                    // Handle "as Alias" part
                    if let Some(alias_node) = child.child_by_field_name("alias") {
                        if let Ok(alias_text) = alias_node.utf8_text(source) {
                            alias_part = Some(alias_text.to_string());
                        }
                    }
                }
                "use_list" => {
                    // For grouped imports like {TypeA, TypeB}, don't extract a specific name
                    return None;
                }
                _ => {
                    // Continue processing
                }
            }
        }

        // Return the alias if present (for renamed imports), otherwise the last identifier
        if let Some(alias) = alias_part {
            Some(alias)
        } else if !path_parts.is_empty() {
            Some(path_parts.last().unwrap().clone())
        } else {
            None
        }
    }

    fn extract_path_from_node(&self, node: &tree_sitter::Node, source: &[u8]) -> Option<String> {
        // Extract the full path from a path node
        let mut path_parts = Vec::new();
        let mut cursor = node.walk();

        for child in node.children(&mut cursor) {
            match child.kind() {
                "identifier" | "super" | "self" | "crate" => {
                    if let Ok(text) = child.utf8_text(source) {
                        path_parts.push(text.to_string());
                    }
                }
                "path" => {
                    if let Some(sub_path) = self.extract_path_from_node(&child, source) {
                        path_parts.push(sub_path);
                    }
                }
                "scoped_identifier" | "::" => {
                    // Continue processing children
                }
                _ => {}
            }
        }

        if path_parts.is_empty() {
            None
        } else {
            Some(path_parts.join("::"))
        }
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
        String::from_utf8_lossy(text)
            .trim()
            .to_string()
            .split_whitespace()
            .collect::<Vec<_>>()
            .join(" ")
    }
    fn extract_usage_attributes(
        &self,
        node: &tree_sitter::Node,
        source: &[u8],
        usage_type: &str,
        context: &str,
        scope: &str,
    ) -> UsageAttributes {
        let symbol_name = node.utf8_text(source).unwrap_or("").to_string();

        UsageAttributes {
            symbol_name,
            usage_type: usage_type.to_string(),
            context: context.to_string(),
            scope: scope.to_string(),
            other: HashMap::new(),
        }
    }

    fn detect_usages(
        &self,
        node: &tree_sitter::Node,
        source: &[u8],
        current_function: Option<&str>,
        current_scope: &str,
    ) -> Vec<DocumentElement> {
        let mut usages = Vec::new();
        let mut cursor = node.walk();

        for child in node.children(&mut cursor) {
            let kind = child.kind();

            match kind {
                // Function calls
                "call_expression" => {
                    if let Some(function_name) = child.child_by_field_name("function") {
                        let name = function_name.utf8_text(source).unwrap_or("").to_string();
                        let usage_attr = self.extract_usage_attributes(
                            &function_name,
                            source,
                            "call",
                            current_function.unwrap_or("global"),
                            current_scope,
                        );

                        let element = DocumentElement::new(
                            ElementType::FunctionCall,
                            Some(name.clone()),
                            child.utf8_text(source).unwrap_or("").to_string(),
                            child.start_position().row + 1,
                            child.end_position().row + 1,
                        )
                        .set_attributes(ElementAttributes::Usage(usage_attr));

                        usages.push(element);
                    }
                }

                // Method calls
                "method_call_expression" => {
                    if let Some(method_name) = child.child_by_field_name("name") {
                        let name = method_name.utf8_text(source).unwrap_or("").to_string();
                        let usage_attr = self.extract_usage_attributes(
                            &method_name,
                            source,
                            "method_call",
                            current_function.unwrap_or("global"),
                            current_scope,
                        );

                        let element = DocumentElement::new(
                            ElementType::MethodCall,
                            Some(name.clone()),
                            child.utf8_text(source).unwrap_or("").to_string(),
                            child.start_position().row + 1,
                            child.end_position().row + 1,
                        )
                        .set_attributes(ElementAttributes::Usage(usage_attr));

                        usages.push(element);
                    }
                }

                // Variable references
                "identifier" => {
                    // Skip identifiers that are part of declarations
                    let parent = child.parent().unwrap();
                    let parent_kind = parent.kind();

                    if !matches!(
                        parent_kind,
                        "function_item"
                            | "struct_item"
                            | "enum_item"
                            | "trait_item"
                            | "impl_item"
                            | "mod_item"
                            | "type_item"
                            | "const_item"
                            | "static_item"
                    ) {
                        let name = child.utf8_text(source).unwrap_or("").to_string();
                        let usage_attr = self.extract_usage_attributes(
                            &child,
                            source,
                            "reference",
                            current_function.unwrap_or("global"),
                            current_scope,
                        );

                        let element = DocumentElement::new(
                            ElementType::VariableReference,
                            Some(name.clone()),
                            child.utf8_text(source).unwrap_or("").to_string(),
                            child.start_position().row + 1,
                            child.end_position().row + 1,
                        )
                        .set_attributes(ElementAttributes::Usage(usage_attr));

                        usages.push(element);
                    }
                }

                // Type references
                "type_identifier" => {
                    let name = child.utf8_text(source).unwrap_or("").to_string();
                    let usage_attr = self.extract_usage_attributes(
                        &child,
                        source,
                        "type_reference",
                        current_function.unwrap_or("global"),
                        current_scope,
                    );

                    let element = DocumentElement::new(
                        ElementType::TypeReference,
                        Some(name.clone()),
                        child.utf8_text(source).unwrap_or("").to_string(),
                        child.start_position().row + 1,
                        child.end_position().row + 1,
                    )
                    .set_attributes(ElementAttributes::Usage(usage_attr));

                    usages.push(element);
                }

                // Assignments
                "assignment_expression" => {
                    if let Some(left) = child.child_by_field_name("left") {
                        let name = left.utf8_text(source).unwrap_or("").to_string();
                        let usage_attr = self.extract_usage_attributes(
                            &left,
                            source,
                            "assignment",
                            current_function.unwrap_or("global"),
                            current_scope,
                        );

                        let element = DocumentElement::new(
                            ElementType::Assignment,
                            Some(name.clone()),
                            child.utf8_text(source).unwrap_or("").to_string(),
                            child.start_position().row + 1,
                            child.end_position().row + 1,
                        )
                        .set_attributes(ElementAttributes::Usage(usage_attr));

                        usages.push(element);
                    }
                }

                _ => {
                    // Recursively check children for usages
                    usages.extend(self.detect_usages(
                        &child,
                        source,
                        current_function,
                        current_scope,
                    ));
                }
            }
        }

        usages
    }

    fn process_nodes(
        &self,
        node: tree_sitter::Node,
        source: &[u8],
        pending_comments: &mut Vec<String>,
    ) -> Vec<DocumentElement> {
        let mut elements = Vec::new();
        let mut cursor = node.walk();

        // Track current function context for usage detection
        let mut current_function: Option<String> = None;
        let mut _current_scope = "global".to_string();

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
                        name.clone(),
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

                    // Set current function context for usage detection
                    if let Some(func_name) = &name {
                        current_function = Some(func_name.clone());
                        _current_scope = format!("function:{}", func_name);
                    }
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

                    // Update scope for impl block
                    let old_scope = _current_scope.clone();
                    _current_scope = format!("impl:{}", type_name);

                    if let Some(body) = child.child_by_field_name("body") {
                        let mut body_comments = Vec::new();
                        let children = self.process_nodes(body, source, &mut body_comments);
                        element = element.with_children(children);
                    }

                    _current_scope = old_scope;
                    elements.push(element);
                }
                "struct_item" => {
                    let name = child
                        .child_by_field_name("name")
                        .and_then(|n| n.utf8_text(source).ok().map(|s| s.to_string()));

                    let mut element = DocumentElement::new(
                        ElementType::Struct,
                        name.clone(),
                        child.utf8_text(source).unwrap_or("").to_string(),
                        child.start_position().row + 1,
                        child.end_position().row + 1,
                    );

                    let rust_attr = RustAttributes {
                        visibility: self.extract_visibility(&child, source),
                        other: HashMap::new(),
                    };

                    if !pending_comments.is_empty() {
                        element.attributes.insert_generic(
                            "docstring".to_string(),
                            serde_json::Value::String(pending_comments.join("\n")),
                        );
                        pending_comments.clear();
                    }
                    element.attributes = ElementAttributes::Rust(rust_attr);
                    elements.push(element);
                }
                "mod_item" => {
                    let name = child
                        .child_by_field_name("name")
                        .and_then(|n| n.utf8_text(source).ok().map(|s| s.to_string()));

                    let mut element = DocumentElement::new(
                        ElementType::Module,
                        name.clone(),
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

                    // Update scope for module
                    let old_scope = _current_scope.clone();
                    if let Some(module_name) = &name {
                        _current_scope = format!("module:{}", module_name);
                    }

                    if let Some(body) = child.child_by_field_name("body") {
                        let mut body_comments = Vec::new();
                        let children = self.process_nodes(body, source, &mut body_comments);
                        element = element.with_children(children);
                    }

                    _current_scope = old_scope;
                    elements.push(element);
                }
                "use_declaration" => {
                    let text = child.utf8_text(source).unwrap_or("").to_string();
                    let name = self.extract_use_path(&child, source);

                    let element = DocumentElement::new(
                        ElementType::Import,
                        name,
                        text,
                        child.start_position().row + 1,
                        child.end_position().row + 1,
                    );

                    elements.push(element);
                }
                _ => {
                    if child.is_named() && kind != "attribute_item" && kind != "visibility_modifier"
                    {
                        pending_comments.clear();
                    }
                }
            }
        }

        // Detect usages in this node (if enabled)
        if self.enable_usages {
            elements.extend(self.detect_usages(
                &node,
                source,
                current_function.as_deref(),
                &_current_scope,
            ));
        }

        elements
    }
}

#[async_trait]
impl Parser for RustParser {
    async fn parse(&self, content: &str) -> VecqResult<ParsedDocument> {
        let mut parser = tree_sitter::Parser::new();
        parser
            .set_language(&tree_sitter_rust::LANGUAGE.into())
            .map_err(|e| {
                VecqError::parse_error(
                    PathBuf::from("unknown"),
                    0,
                    format!("Failed to load Rust language: {}", e),
                    None::<std::io::Error>,
                )
            })?;

        let tree = parser.parse(content, None).ok_or_else(|| {
            VecqError::parse_error(
                PathBuf::from("unknown"),
                0,
                "Failed to parse content".to_string(),
                None::<std::io::Error>,
            )
        })?;

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
        let imports: Vec<_> = result
            .elements
            .iter()
            .filter(|e| e.element_type == ElementType::Import)
            .collect();

        // Debug: print all imports found
        println!("Found {} imports:", imports.len());
        for imp in &imports {
            println!("  - content: '{}', name: {:?}", imp.content, imp.name);
        }

        // 1. HashMap should be named (what you use in code)
        let hashmap = imports
            .iter()
            .find(|i| i.content.contains("HashMap"))
            .unwrap();
        assert_eq!(hashmap.name, Some("HashMap".to_string()));
    }

    #[tokio::test]
    async fn test_usage_detection() {
        let parser = RustParser::new().with_usages(true);
        let content = r#"
fn greet(name: &str) -> String {
    let result = format!("Hello {}", name);
    println!("{}", result);
    result
}

fn main() {
    greet("world");
}
"#;

        let result = parser.parse(content).await.unwrap();

        // Should detect function calls: println!(), greet()
        let function_calls: Vec<_> = result
            .elements
            .iter()
            .filter(|e| e.element_type == ElementType::FunctionCall)
            .collect();
        assert!(function_calls.len() >= 1); // At least greet

        // Check function call names
        let function_names: Vec<String> = function_calls
            .iter()
            .filter_map(|e| e.name.clone())
            .collect();
        assert!(function_names.contains(&"greet".to_string()));
    }
}
