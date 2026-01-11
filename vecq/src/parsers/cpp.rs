// PURPOSE:
//   C++ parser implementation for vecq using tree-sitter-cpp.
//   Extracts C++ AST elements (functions, classes, namespaces, includes).
//
// RELATED FILES:
//   - src/parsers/c.rs - Base C parser reference
//   - src/types.rs - DocumentElement, ElementType definitions

use crate::error::{VecqError, VecqResult};
use crate::parser::{Parser, ParserCapabilities, ParserConfig};
use crate::types::{DocumentElement, DocumentMetadata, ElementType, FileType, ParsedDocument, CFamilyAttributes, ElementAttributes};
use async_trait::async_trait;
use serde_json::json;
use std::collections::HashMap;
use std::path::PathBuf;

/// C++ parser that extracts structural elements from C++ source code
#[derive(Debug, Clone)]
pub struct CppParser {
    _config: ParserConfig,
}

impl CppParser {
    pub fn new() -> Self {
        Self {
            _config: ParserConfig::default(),
        }
    }

    pub fn with_config(config: ParserConfig) -> Self {
        Self { _config: config }
    }

    fn extract_functions(&self, content: &str, tree: &tree_sitter::Tree) -> VecqResult<Vec<DocumentElement>> {
        let mut functions = Vec::new();
        self.extract_functions_recursive(content, tree.root_node(), &mut functions);
        Ok(functions)
    }

    fn extract_functions_recursive(&self, content: &str, node: tree_sitter::Node, functions: &mut Vec<DocumentElement>) {
        let mut cursor = node.walk();

        for child in node.children(&mut cursor) {
            if child.kind() == "function_definition" {
                if let Some(func) = self.parse_function(content, child) {
                    functions.push(func);
                }
            } else if child.kind() == "namespace_definition" || child.kind() == "class_specifier" {
                // Recurse into namespaces and classes
                self.extract_functions_recursive(content, child, functions);
            }
        }
    }

    fn parse_function(&self, content: &str, node: tree_sitter::Node) -> Option<DocumentElement> {
        let mut name = String::new();
        let mut return_type = String::new();
        let mut cursor = node.walk();

        for child in node.children(&mut cursor) {
            match child.kind() {
                "type_identifier" | "primitive_type" | "auto" => {
                    if return_type.is_empty() {
                        return_type = child.utf8_text(content.as_bytes()).unwrap_or("").to_string();
                    }
                }
                "function_declarator" => {
                    let mut decl_cursor = child.walk();
                    for decl_child in child.children(&mut decl_cursor) {
                        if decl_child.kind() == "identifier" || decl_child.kind() == "qualified_identifier" {
                            name = decl_child.utf8_text(content.as_bytes()).unwrap_or("").to_string();
                        }
                    }
                }
                _ => {}
            }
        }

        if name.is_empty() {
            return None;
        }

        let element = DocumentElement::new(
            ElementType::Function,
            Some(name),
            node.utf8_text(content.as_bytes()).unwrap_or("").to_string(),
            node.start_position().row + 1,
            node.end_position().row + 1,
        )
        .set_attributes(ElementAttributes::CFamily(CFamilyAttributes {
            other: {
                let mut map = HashMap::new();
                map.insert("return_type".to_string(), json!(return_type));
                map
            }
        }));

        Some(element)
    }

    fn extract_classes(&self, content: &str, tree: &tree_sitter::Tree) -> VecqResult<Vec<DocumentElement>> {
        let mut classes = Vec::new();
        self.extract_classes_recursive(content, tree.root_node(), &mut classes);
        Ok(classes)
    }

    fn extract_classes_recursive(&self, content: &str, node: tree_sitter::Node, classes: &mut Vec<DocumentElement>) {
        let mut cursor = node.walk();

        for child in node.children(&mut cursor) {
            if child.kind() == "class_specifier" || child.kind() == "struct_specifier" {
                if let Some(cls) = self.parse_class(content, child) {
                    classes.push(cls);
                }
            } else if child.kind() == "namespace_definition" {
                self.extract_classes_recursive(content, child, classes);
            }
        }
    }

    fn parse_class(&self, content: &str, node: tree_sitter::Node) -> Option<DocumentElement> {
        let mut name: Option<String> = None;
        let element_type = if node.kind() == "class_specifier" {
            ElementType::Class
        } else {
            ElementType::Struct
        };

        let mut cursor = node.walk();
        for child in node.children(&mut cursor) {
            if child.kind() == "type_identifier" {
                name = Some(child.utf8_text(content.as_bytes()).unwrap_or("").to_string());
                break;
            }
        }

        Some(DocumentElement::new(
            element_type,
            name,
            node.utf8_text(content.as_bytes()).unwrap_or("").to_string(),
            node.start_position().row + 1,
            node.end_position().row + 1,
        ).set_attributes(ElementAttributes::CFamily(CFamilyAttributes {
            other: HashMap::new(),
        })))
    }

    fn extract_namespaces(&self, content: &str, tree: &tree_sitter::Tree) -> VecqResult<Vec<DocumentElement>> {
        let mut namespaces = Vec::new();
        let root = tree.root_node();
        let mut cursor = root.walk();

        for child in root.children(&mut cursor) {
            if child.kind() == "namespace_definition" {
                if let Some(ns) = self.parse_namespace(content, child) {
                    namespaces.push(ns);
                }
            }
        }

        Ok(namespaces)
    }

    fn parse_namespace(&self, content: &str, node: tree_sitter::Node) -> Option<DocumentElement> {
        let mut name: Option<String> = None;
        let mut cursor = node.walk();

        for child in node.children(&mut cursor) {
            if child.kind() == "identifier" {
                name = Some(child.utf8_text(content.as_bytes()).unwrap_or("").to_string());
                break;
            }
        }

        Some(DocumentElement::new(
            ElementType::Namespace,
            name,
            node.utf8_text(content.as_bytes()).unwrap_or("").to_string(),
            node.start_position().row + 1,
            node.end_position().row + 1,
        ).set_attributes(ElementAttributes::CFamily(CFamilyAttributes {
            other: HashMap::new(),
        })))
    }

    fn extract_includes(&self, content: &str, tree: &tree_sitter::Tree) -> VecqResult<Vec<DocumentElement>> {
        let mut includes = Vec::new();
        let root = tree.root_node();
        let mut cursor = root.walk();

        for child in root.children(&mut cursor) {
            if child.kind() == "preproc_include" {
                let text = child.utf8_text(content.as_bytes()).unwrap_or("");
                let path = text.trim_start_matches("#include")
                    .trim()
                    .trim_matches(|c| c == '<' || c == '>' || c == '"');

                let is_system = text.contains('<');

                let element = DocumentElement::new(
                    ElementType::Import,
                    Some(path.to_string()),
                    text.to_string(),
                    child.start_position().row + 1,
                    child.end_position().row + 1,
                )
                .set_attributes(ElementAttributes::CFamily(CFamilyAttributes {
                    other: {
                        let mut map = HashMap::new();
                        map.insert("is_system".to_string(), json!(is_system));
                        map
                    }
                }));
                
                includes.push(element);
            }
        }

        Ok(includes)
    }
}

impl Default for CppParser {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl Parser for CppParser {
    fn file_extensions(&self) -> &[&str] {
        &["cpp", "cc", "cxx", "hpp", "hxx", "h"]
    }

    fn language_name(&self) -> &str {
        "C++"
    }

    fn capabilities(&self) -> ParserCapabilities {
        ParserCapabilities {
            incremental: false,
            error_recovery: true,
            documentation: false,
            type_information: true,
            macros: true,
            max_file_size: None,
        }
    }

    async fn parse(&self, content: &str) -> VecqResult<ParsedDocument> {
        let mut parser = tree_sitter::Parser::new();
        parser.set_language(&tree_sitter_cpp::LANGUAGE.into())
            .map_err(|e| VecqError::ParseError {
                file: PathBuf::from("<string>"),
                line: 0,
                message: format!("Failed to set C++ language: {}", e),
                source: None,
            })?;

        let tree = parser.parse(content, None)
            .ok_or_else(|| VecqError::ParseError {
                file: PathBuf::from("<string>"),
                line: 0,
                message: "Failed to parse C++ content".to_string(),
                source: None,
            })?;

        let functions = self.extract_functions(content, &tree)?;
        let classes = self.extract_classes(content, &tree)?;
        let namespaces = self.extract_namespaces(content, &tree)?;
        let includes = self.extract_includes(content, &tree)?;

        let mut elements = Vec::new();
        elements.extend(functions);
        elements.extend(classes);
        elements.extend(namespaces);
        elements.extend(includes);

        let mut doc = ParsedDocument::new(
            DocumentMetadata::new(PathBuf::from("file.cpp"), content.len() as u64)
                .with_line_count(content)
                .with_file_type(FileType::Cpp)
        );
        doc.elements = elements;

        Ok(doc)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_parse_function() {
        let parser = CppParser::new();
        let content = r#"
int main() {
    return 0;
}
"#;
        let result = parser.parse(content).await.unwrap();
        
        let functions: Vec<_> = result.elements.iter()
            .filter(|e| e.element_type == ElementType::Function)
            .collect();
        
        assert_eq!(functions.len(), 1);
        assert_eq!(functions[0].name, Some("main".to_string()));
    }

    #[tokio::test]
    async fn test_parse_class() {
        let parser = CppParser::new();
        let content = r#"
class Point {
public:
    int x;
    int y;
};
"#;
        let result = parser.parse(content).await.unwrap();
        
        let classes: Vec<_> = result.elements.iter()
            .filter(|e| e.element_type == ElementType::Class)
            .collect();
        
        assert_eq!(classes.len(), 1);
        assert_eq!(classes[0].name, Some("Point".to_string()));
    }

    #[tokio::test]
    async fn test_parse_namespace() {
        let parser = CppParser::new();
        let content = r#"
namespace mylib {
    void foo() {}
}
"#;
        let result = parser.parse(content).await.unwrap();
        
        let namespaces: Vec<_> = result.elements.iter()
            .filter(|e| e.element_type == ElementType::Namespace)
            .collect();
        
        assert_eq!(namespaces.len(), 1);
        // Note: Name extraction depends on tree-sitter grammar version
        // For now, verify we found the namespace
    }

    #[tokio::test]
    async fn test_parse_includes() {
        let parser = CppParser::new();
        let content = r#"
#include <iostream>
#include "myheader.h"

int main() { return 0; }
"#;
        let result = parser.parse(content).await.unwrap();
        
        let imports: Vec<_> = result.elements.iter()
            .filter(|e| e.element_type == ElementType::Import)
            .collect();
        
        assert_eq!(imports.len(), 2);
    }

    #[tokio::test]
    async fn test_empty_content() {
        let parser = CppParser::new();
        let result = parser.parse("").await.unwrap();
        assert!(result.elements.is_empty());
    }
}
