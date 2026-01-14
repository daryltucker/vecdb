// PURPOSE:
//   Go parser implementation for vecq using tree-sitter-go.
//   Extracts Go AST elements (functions, structs, interfaces, imports).
//
// RELATED FILES:
//   - src/parsers/c.rs - Reference implementation pattern
//   - src/types.rs - DocumentElement, ElementType definitions

use crate::error::{VecqError, VecqResult};
use crate::parser::{Parser, ParserCapabilities, ParserConfig};
use crate::types::{DocumentElement, DocumentMetadata, ElementType, FileType, ParsedDocument, GoAttributes, ElementAttributes};
use std::collections::HashMap;
use async_trait::async_trait;
use std::path::PathBuf;
use serde_json::json;

/// Go parser that extracts structural elements from Go source code
#[derive(Debug, Clone)]
pub struct GoParser {
    _config: ParserConfig,
}

impl GoParser {
    pub fn new() -> Self {
        Self {
            _config: ParserConfig::default(),
        }
    }

    pub fn with_config(config: ParserConfig) -> Self {
        Self { _config: config }
    }

    fn extract_raw_elements(&self, content: &str, tree: &tree_sitter::Tree) -> VecqResult<Vec<DocumentElement>> {
        let mut elements = Vec::new();
        let root = tree.root_node();
        let mut cursor = root.walk();

        for child in root.children(&mut cursor) {
            match child.kind() {
                "function_declaration" | "method_declaration" => {
                    if let Some(func) = self.parse_function(content, child) {
                        elements.push(func);
                    }
                }
                "type_declaration" => {
                     // Check type specs inside
                     let mut t_cursor = child.walk();
                     for t_child in child.children(&mut t_cursor) {
                         if t_child.kind() == "type_spec" {
                             if let Some(s) = self.parse_type_spec(content, t_child) {
                                 elements.push(s);
                             }
                         }
                     }
                }
                "import_declaration" => {
                    elements.extend(self.parse_import_declaration(content, child));
                }
                _ => {}
            }
        }

        Ok(elements)
    }

    fn parse_function(&self, content: &str, node: tree_sitter::Node) -> Option<DocumentElement> {
        let mut name = String::new();
        let mut receiver_type = None;
        let _cursor = node.walk();

        // Extract Name
        if let Some(name_node) = node.child_by_field_name("name") {
            name = name_node.utf8_text(content.as_bytes()).unwrap_or("").to_string();
        }

        // Extract Receiver if method
        if node.kind() == "method_declaration" {
            if let Some(receiver_node) = node.child_by_field_name("receiver") {
                // receiver is parameter_list -> parameter_declaration -> type
                let mut rc = receiver_node.walk();
                for r_child in receiver_node.children(&mut rc) {
                    if r_child.kind() == "parameter_declaration" {
                         if let Some(type_node) = r_child.child_by_field_name("type") {
                             let type_text = type_node.utf8_text(content.as_bytes()).unwrap_or("");
                             // Handle pointers: "*MyStruct" -> "MyStruct"
                             let clean_type = type_text.trim_start_matches('*').trim();
                             receiver_type = Some(clean_type.to_string());
                         }
                    }
                }
            }
        }

        if name.is_empty() { return None; }

        let mut attributes = HashMap::new();
        if let Some(rt) = receiver_type {
            attributes.insert("receiver".to_string(), json!(rt));
        }

        Some(DocumentElement::new(
            ElementType::Function,
            Some(name),
            node.utf8_text(content.as_bytes()).unwrap_or("").to_string(),
            node.start_position().row + 1,
            node.end_position().row + 1,
        ).set_attributes(ElementAttributes::Go(GoAttributes {
            other: attributes,
        })))
    }

    fn parse_type_spec(&self, content: &str, node: tree_sitter::Node) -> Option<DocumentElement> {
        // node is type_spec: name=type_identifier, type=struct_type/interface_type
        let mut name = None;
        let mut element_type = ElementType::Struct;
        
        if let Some(name_node) = node.child_by_field_name("name") {
            name = Some(name_node.utf8_text(content.as_bytes()).unwrap_or("").to_string());
        }
        
        if let Some(type_node) = node.child_by_field_name("type") {
            if type_node.kind() == "interface_type" {
                element_type = ElementType::Interface;
            }
        }

        Some(DocumentElement::new(
            element_type,
            name,
            node.utf8_text(content.as_bytes()).unwrap_or("").to_string(),
            node.start_position().row + 1,
            node.end_position().row + 1,
        ).set_attributes(ElementAttributes::Go(GoAttributes {
            other: HashMap::new(),
        })))
    }

    fn parse_import_declaration(&self, content: &str, node: tree_sitter::Node) -> Vec<DocumentElement> {
        let mut imports = Vec::new();
        let text = node.utf8_text(content.as_bytes()).unwrap_or("");
        
        imports.push(DocumentElement::new(
            ElementType::Import,
            None,
            text.to_string(),
            node.start_position().row + 1,
            node.end_position().row + 1,
        ).set_attributes(ElementAttributes::Go(GoAttributes {
            other: HashMap::new(),
        })));
        
        imports
    }

    /// Link methods to their receiver structs
    fn link_methods(&self, elements: Vec<DocumentElement>) -> Vec<DocumentElement> {
        let mut structs: HashMap<String, (usize, DocumentElement)> = HashMap::new();
        let mut other_elements = Vec::new();
        let mut methods_to_link = Vec::new();
        let mut sort_order = 0;

        // 1. Separate items
        for el in elements {
            if (el.element_type == ElementType::Struct || el.element_type == ElementType::Interface) && el.name.is_some() {
                 let name = el.name.clone().unwrap();
                 structs.insert(name, (sort_order, el));
            } else if el.element_type == ElementType::Function {
                // Check receiver attribute
                let has_receiver = match &el.attributes {
                    ElementAttributes::Go(attrs) => attrs.other.contains_key("receiver"),
                    _ => false,
                };
                if has_receiver {
                    methods_to_link.push(el);
                } else {
                    other_elements.push((sort_order, el));
                }
            } else {
                other_elements.push((sort_order, el));
            }
            sort_order += 1;
        }

        // 2. Link
        for method in methods_to_link {
            let receiver = match &method.attributes {
                ElementAttributes::Go(attrs) => attrs.other.get("receiver").and_then(|v| v.as_str()).map(|s| s.to_string()),
                _ => None,
            };

            if let Some(target) = receiver {
                if let Some((_, struct_el)) = structs.get_mut(&target) {
                    struct_el.children.push(method);
                    continue;
                }
            }
            
            other_elements.push((sort_order, method)); // Orphaned or external receiver
            sort_order += 1;
        }

        // 3. Recombine (Structs + Others)
        let mut final_list = Vec::new();
        for (_, s) in structs.into_values() {
            final_list.push(s);
        }
        for (_, e) in other_elements {
            final_list.push(e);
        }
        
        // Sort by line number to keep roughly in file order
        final_list.sort_by_key(|e| e.line_start);
        
        final_list
    }
}

impl Default for GoParser {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl Parser for GoParser {
    fn file_extensions(&self) -> &[&str] {
        &["go"]
    }

    fn language_name(&self) -> &str {
        "Go"
    }

    fn capabilities(&self) -> ParserCapabilities {
        ParserCapabilities {
            incremental: false,
            error_recovery: true,
            documentation: true,
            type_information: true,
            macros: false,
            max_file_size: None,
        }
    }

    async fn parse(&self, content: &str) -> VecqResult<ParsedDocument> {
        let mut parser = tree_sitter::Parser::new();
        parser.set_language(&tree_sitter_go::LANGUAGE.into())
            .map_err(|e| VecqError::ParseError {
                file: PathBuf::from("<string>"),
                line: 0,
                message: format!("Failed to set Go language: {}", e),
                source: None,
            })?;

        let tree = parser.parse(content, None)
            .ok_or_else(|| VecqError::ParseError {
                file: PathBuf::from("<string>"),
                line: 0,
                message: "Failed to parse Go content".to_string(),
                source: None,
            })?;

        let raw_elements = self.extract_raw_elements(content, &tree)?;
        let linked_elements = self.link_methods(raw_elements);

        let mut doc = ParsedDocument::new(
            DocumentMetadata::new(PathBuf::from("file.go"), content.len() as u64)
                .with_line_count(content)
                .with_file_type(FileType::Go)
        );
        doc.elements = linked_elements;

        Ok(doc)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_parse_function() {
        let parser = GoParser::new();
        let content = r#"
package main
func main() {}
"#;
        let result = parser.parse(content).await.unwrap();
        let functions: Vec<_> = result.elements.iter()
            .filter(|e| e.element_type == ElementType::Function)
            .collect();
        assert_eq!(functions.len(), 1);
        assert_eq!(functions[0].name, Some("main".to_string()));
    }

    #[tokio::test]
    async fn test_parse_struct_method_linking() {
        let parser = GoParser::new();
        let content = r#"
package main

type Point struct {
    X int
    Y int
}

func (p *Point) Move(dx int, dy int) {
    p.X += dx
}

func (p Point) Dist() int {
    return 0
}
"#;
        let result = parser.parse(content).await.unwrap();
        
        let structs: Vec<_> = result.elements.iter()
            .filter(|e| e.element_type == ElementType::Struct)
            .collect();
            
        assert_eq!(structs.len(), 1);
        let point = &structs[0];
        assert_eq!(point.name, Some("Point".to_string()));
        
        // Verify linking
        assert_eq!(point.children.len(), 2, "Both methods (ptr and value receiver) should be linked");
        
        let has_move = point.children.iter().any(|c| c.name.as_deref() == Some("Move") && c.element_type == ElementType::Function);
        let has_dist = point.children.iter().any(|c| c.name.as_deref() == Some("Dist") && c.element_type == ElementType::Function);
        
        assert!(has_move, "Move method missing from children");
        assert!(has_dist, "Dist method missing from children");
    }
    
    #[tokio::test]
    async fn test_parse_interface() {
        let parser = GoParser::new();
        let content = r#"type Reader interface {}"#;
        let result = parser.parse(content).await.unwrap();
        assert!(!result.elements.is_empty());
    }
    
    #[tokio::test]
    async fn test_orphan_method() {
         let parser = GoParser::new();
         let content = r#"func (s *Unknown) Foo() {}"#;
         let result = parser.parse(content).await.unwrap();
         
         // Should act as a top-level function since Struct is missing
         let functions: Vec<_> = result.elements.iter()
            .filter(|e| e.element_type == ElementType::Function)
            .collect();
         assert_eq!(functions.len(), 1);
         assert_eq!(functions[0].name, Some("Foo".to_string()));
    }
}
