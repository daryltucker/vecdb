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

    /// Process nodes recursively to preserve hierarchy
    fn process_nodes(
        &self,
        node: tree_sitter::Node,
        content: &str,
        source: &[u8],
    ) -> Vec<DocumentElement> {
        let mut elements = Vec::new();
        let mut cursor = node.walk();

        for child in node.children(&mut cursor) {
            let kind = child.kind();
            
            match kind {
                "namespace_definition" => {
                    let mut element = self.create_element(child, content, source, ElementType::Namespace);
                    if let Some(body) = child.child_by_field_name("body") {
                        let children = self.process_nodes(body, content, source);
                        element = element.with_children(children);
                    }
                    elements.push(element);
                }
                "class_specifier" | "struct_specifier" => {
                    let element_type = if kind == "class_specifier" { ElementType::Class } else { ElementType::Struct };
                    let mut element = self.create_element(child, content, source, element_type);
                    
                    if let Some(body) = child.child_by_field_name("body") {
                        let children = self.process_nodes(body, content, source);
                        element = element.with_children(children);
                    }
                    elements.push(element);
                }
                "function_definition" => {
                    if let Some(func) = self.parse_function(content, child) {
                        elements.push(func);
                    }
                }
                "preproc_include" => {
                     if let Some(include) = self.parse_include(content, child) {
                         elements.push(include);
                     }
                }
                "declaration" => {
                     if let Some(field) = self.parse_field(content, child) {
                         elements.push(field);
                     }
                }
                _ => {
                    // Recurse into linkage specs (extern "C") or other wrappers
                    if kind == "linkage_specification" {
                        if let Some(body) = child.child_by_field_name("body") {
                             let children = self.process_nodes(body, content, source);
                             elements.extend(children);
                        }
                    }
                }
            }
        }
        
        elements
    }

    fn create_element(&self, node: tree_sitter::Node, _content: &str, source: &[u8], element_type: ElementType) -> DocumentElement {
         let mut name = None;
         if let Some(name_node) = node.child_by_field_name("name") {
             name = Some(name_node.utf8_text(source).unwrap_or("").to_string());
         }
         
         if name.is_none() {
              let mut cursor = node.walk();
              for child in node.children(&mut cursor) {
                  if child.kind() == "type_identifier" {
                       name = Some(child.utf8_text(source).unwrap_or("").to_string());
                       break;
                  }
              }
         }

         DocumentElement::new(
            element_type,
            name,
            node.utf8_text(source).unwrap_or("").to_string(),
            node.start_position().row + 1,
            node.end_position().row + 1,
        ).set_attributes(ElementAttributes::CFamily(CFamilyAttributes {
            other: HashMap::new(),
        }))
    }

    fn parse_function(&self, content: &str, node: tree_sitter::Node) -> Option<DocumentElement> {
        let mut name = String::new();
        let mut cursor = node.walk();

        // Extract name
        if let Some(declarator) = node.child_by_field_name("declarator") {
             name = declarator.utf8_text(content.as_bytes()).unwrap_or("").to_string();
             if declarator.kind() == "function_declarator" {
                 if let Some(id) = declarator.child_by_field_name("declarator") {
                     name = id.utf8_text(content.as_bytes()).unwrap_or("").to_string();
                 }
             }
        }
        
        if name.is_empty() {
             for child in node.children(&mut cursor) {
                if child.kind() == "function_declarator" {
                    let mut d = child.walk();
                     for deep_child in child.children(&mut d) {
                         if deep_child.kind() == "identifier" || deep_child.kind() == "field_identifier" || deep_child.kind() == "qualified_identifier" {
                             name = deep_child.utf8_text(content.as_bytes()).unwrap_or("").to_string();
                             break;
                         }
                     }
                }
            }
        }

        if name.is_empty() {
            return None;
        }

        Some(DocumentElement::new(
            ElementType::Function,
            Some(name),
            node.utf8_text(content.as_bytes()).unwrap_or("").to_string(),
            node.start_position().row + 1,
            node.end_position().row + 1,
        ).set_attributes(ElementAttributes::CFamily(CFamilyAttributes {
            other: HashMap::new(),
        })))
    }

    fn parse_include(&self, content: &str, node: tree_sitter::Node) -> Option<DocumentElement> {
        let text = node.utf8_text(content.as_bytes()).unwrap_or("");
        let path = text.trim_start_matches("#include")
            .trim()
            .trim_matches(|c| c == '<' || c == '>' || c == '"');
        let is_system = text.contains('<');

        Some(DocumentElement::new(
            ElementType::Import,
            Some(path.to_string()),
            text.to_string(),
            node.start_position().row + 1,
            node.end_position().row + 1,
        ).set_attributes(ElementAttributes::CFamily(CFamilyAttributes {
            other: {
                let mut map = HashMap::new();
                map.insert("is_system".to_string(), json!(is_system));
                map
            }
        })))
    }
    
    fn parse_field(&self, content: &str, node: tree_sitter::Node) -> Option<DocumentElement> {
        let mut name = None;
        let mut cursor = node.walk();
        
        for child in node.children(&mut cursor) {
             if child.kind() == "init_declarator" || child.kind() == "field_declaration" {
                  let mut d_cursor = child.walk();
                   for d_child in child.children(&mut d_cursor) {
                        if d_child.kind() == "identifier" || d_child.kind() == "field_identifier" {
                            name = Some(d_child.utf8_text(content.as_bytes()).unwrap_or("").to_string());
                            break;
                        }
                   }
             }
        }
        
        #[allow(clippy::manual_map)]
        if let Some(n) = name {
            Some(DocumentElement::new(
                ElementType::Variable,
                Some(n),
                node.utf8_text(content.as_bytes()).unwrap_or("").to_string(),
                node.start_position().row + 1,
                node.end_position().row + 1,
            ).set_attributes(ElementAttributes::CFamily(CFamilyAttributes {
                other: HashMap::new(),
            })))
        } else {
            None
        }
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
        &["cpp", "cc", "cxx", "hpp", "hxx"]
    }

    fn language_name(&self) -> &str {
        "C++"
    }

    fn capabilities(&self) -> ParserCapabilities {
        ParserCapabilities {
            incremental: false,
            error_recovery: true,
            documentation: false, // Could enable later if we extract comments
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

        let elements = self.process_nodes(tree.root_node(), content, content.as_bytes());

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
    async fn test_parse_top_level_function() {
        let parser = CppParser::new();
        let content = r#"
int main() {
    return 0;
}
"#;
        let result = parser.parse(content).await.unwrap();
        
        // Should be at top level
        let functions: Vec<_> = result.elements.iter()
            .filter(|e| e.element_type == ElementType::Function)
            .collect();
        
        assert_eq!(functions.len(), 1);
        assert_eq!(functions[0].name, Some("main".to_string()));
    }

    #[tokio::test]
    async fn test_parse_nested_class() {
        let parser = CppParser::new();
        let content = r#"
class Verified {
public:
    void method() {}
    int field;
};
"#;
        let result = parser.parse(content).await.unwrap();
        
        let classes: Vec<_> = result.elements.iter()
            .filter(|e| e.element_type == ElementType::Class)
            .collect();
        
        assert_eq!(classes.len(), 1);
        let cls = &classes[0];
        assert_eq!(cls.name, Some("Verified".to_string()));
        
        // VERIFY HIERARCHY: Method should be a child, not a sibling
        let methods: Vec<_> = cls.children.iter()
            .filter(|e| e.element_type == ElementType::Function)
            .collect();
        assert_eq!(methods.len(), 1, "Method should be nested in class");
        assert_eq!(methods[0].name, Some("method".to_string()));
    }

    #[tokio::test]
    async fn test_parse_namespace_hierarchy() {
        let parser = CppParser::new();
        let content = r#"
namespace mylib {
    void foo() {}
    class Internal {};
}
"#;
        let result = parser.parse(content).await.unwrap();
        
        let namespaces: Vec<_> = result.elements.iter()
            .filter(|e| e.element_type == ElementType::Namespace)
            .collect();
        
        assert_eq!(namespaces.len(), 1);
        let ns = &namespaces[0];
        // Note: Name might be "mylib" or empty depending on extraction logic detail, 
        // strictly checking children here.
        
        assert_eq!(ns.children.len(), 2, "Namespace should have 2 children");
        
        let has_foo = ns.children.iter().any(|c| c.name.as_deref() == Some("foo") && c.element_type == ElementType::Function);
        let has_internal = ns.children.iter().any(|c| c.name.as_deref() == Some("Internal") && c.element_type == ElementType::Class);
        
        assert!(has_foo, "Function foo should be nested in namespace");
        assert!(has_internal, "Class Internal should be nested in namespace");
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
        
        // Includes are top level
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
