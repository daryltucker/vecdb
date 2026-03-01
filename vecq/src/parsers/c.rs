// PURPOSE:
//   C parser implementation for vecq using tree-sitter-c.
//   Extracts C AST elements (functions, structs, typedefs, includes) while
//   preserving line numbers and structural relationships.
//
// REQUIREMENTS:
//   1. Parse C source files using tree-sitter-c
//   2. Extract function definitions with parameters and return types
//   3. Extract struct/union/enum definitions
//   4. Extract #include directives
//
// RELATED FILES:
//   - src/parsers/mod.rs - Parser registry
//   - src/types.rs - DocumentElement, ElementType definitions
//   - src/parser.rs - Parser trait, ParserCapabilities
//   - src/error.rs - VecqError definitions

use crate::error::{VecqError, VecqResult};
use crate::parser::{Parser, ParserCapabilities, ParserConfig};
use crate::types::{DocumentElement, DocumentMetadata, ElementType, FileType, ParsedDocument, CFamilyAttributes, ElementAttributes};
use async_trait::async_trait;
use serde_json::json;
use std::collections::HashMap;
use std::path::PathBuf;

/// C parser that extracts structural elements from C source code
#[derive(Debug, Clone)]
pub struct CParser {
    _config: ParserConfig,
}

impl CParser {
    /// Create a new C parser with default configuration
    pub fn new() -> Self {
        Self {
            _config: ParserConfig::default(),
        }
    }

    /// Create a new C parser with custom configuration
    pub fn with_config(config: ParserConfig) -> Self {
        Self { _config: config }
    }

    /// Process nodes recursively to preserve hierarchy
    fn process_nodes(
        &self,
        node: tree_sitter::Node,
        content: &str,
    ) -> Vec<DocumentElement> {
        let mut elements = Vec::new();
        let mut cursor = node.walk();

        for child in node.children(&mut cursor) {
            let kind = child.kind();
            
            match kind {
                "function_definition" => {
                    if let Some(func) = self.parse_function_definition(content, child) {
                        elements.push(func);
                    }
                }
                "struct_specifier" | "union_specifier" | "enum_specifier" => {
                    let element_type = match kind {
                        "union_specifier" => ElementType::Union, // If Union type exists, else Struct
                        "enum_specifier" => ElementType::Enum,   // If Enum type exists, else Struct
                        _ => ElementType::Struct,
                    };
                    
                    // Fallback if types not defined in ElementType
                    let effective_type = match element_type {
                         ElementType::Union | ElementType::Enum | ElementType::Struct => element_type,
                         _ => ElementType::Struct, 
                    };

                    let mut element = self.parse_complex_type(content, child, effective_type);
                    
                    if let Some(body) = child.child_by_field_name("body") {
                        let children = self.process_nodes(body, content);
                        element = element.with_children(children);
                    }
                    elements.push(element);
                }
                "type_definition" => {
                    // Typedefs might wrap structs
                    if let Some(type_decl) = self.parse_typedef(content, child) {
                        elements.push(type_decl);
                    }
                }
                 "preproc_include" => {
                     if let Some(include) = self.parse_include(content, child) {
                         elements.push(include);
                     }
                }
                "declaration" | "field_declaration" => {
                     // Check for fields if we are inside a struct, or global variables
                     if let Some(field) = self.parse_field(content, child) {
                         elements.push(field);
                     }
                     // Recurse to find nested types defined within the declaration (e.g. struct defined in field type)
                     elements.extend(self.process_nodes(child, content));
                }
                _ => {
                    // Recurse into linkage specs or blocks
                    if kind == "linkage_specification" || kind == "compound_statement" {
                        if let Some(body) = child.child_by_field_name("body") {
                             let children = self.process_nodes(body, content);
                             elements.extend(children);
                        }
                    }
                }
            }
        }
        
        elements
    }

    fn parse_function_definition(&self, content: &str, node: tree_sitter::Node) -> Option<DocumentElement> {
        let mut name = String::new();
        let mut return_type = String::new();
        let mut params = Vec::<String>::new();

        let mut cursor = node.walk();
        for child in node.children(&mut cursor) {
            match child.kind() {
                "type_identifier" | "primitive_type" => {
                    return_type = child.utf8_text(content.as_bytes()).unwrap_or("").to_string();
                }
                "function_declarator" => {
                    let mut decl_cursor = child.walk();
                    for decl_child in child.children(&mut decl_cursor) {
                        if decl_child.kind() == "identifier" {
                            name = decl_child.utf8_text(content.as_bytes()).unwrap_or("").to_string();
                        } else if decl_child.kind() == "parameter_list" {
                            let mut param_cursor = decl_child.walk();
                            for param in decl_child.children(&mut param_cursor) {
                                if param.kind() == "parameter_declaration" {
                                    if let Ok(param_text) = param.utf8_text(content.as_bytes()) {
                                        params.push(param_text.to_string());
                                    }
                                }
                            }
                        }
                    }
                }
                _ => {}
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
        )
        .set_attributes(ElementAttributes::CFamily(CFamilyAttributes {
            other: {
                let mut map = HashMap::new();
                map.insert("return_type".to_string(), json!(return_type));
                map.insert("parameters".to_string(), json!(params));
                map
            }
        })))
    }

    fn parse_complex_type(&self, content: &str, node: tree_sitter::Node, element_type: ElementType) -> DocumentElement {
        let mut name: Option<String> = None;
        if let Some(name_node) = node.child_by_field_name("name") {
            name = Some(name_node.utf8_text(content.as_bytes()).unwrap_or("").to_string());
        }

        DocumentElement::new(
            element_type,
            name,
            node.utf8_text(content.as_bytes()).unwrap_or("").to_string(),
            node.start_position().row + 1,
            node.end_position().row + 1,
        ).set_attributes(ElementAttributes::CFamily(CFamilyAttributes {
            other: HashMap::new(),
        }))
    }
    
    fn parse_typedef(&self, content: &str, node: tree_sitter::Node) -> Option<DocumentElement> {
         // simple extraction
         let mut name = None;
         let mut cursor = node.walk();
         
         for child in node.children(&mut cursor) {
             if child.kind() == "type_identifier" {
                 name = Some(child.utf8_text(content.as_bytes()).unwrap_or("").to_string());
             }
         }
         
         Some(DocumentElement::new(
             ElementType::Struct, // Treat typedefs as Structs for now or Variable?
             name,
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
         
         // If node is already field_declaration or declaration, scan children for identifier
         if node.kind() == "field_declaration" || node.kind() == "declaration" {
             let mut cursor = node.walk();
             for child in node.children(&mut cursor) {
                 if child.kind() == "field_identifier" || child.kind() == "identifier" {
                     name = Some(child.utf8_text(content.as_bytes()).unwrap_or("").to_string());
                     break;
                 }
                 // Handle init_declarator / field_declarator wrapper
                 if child.kind() == "init_declarator" || child.kind() == "field_declarator" {
                     let mut d_cursor = child.walk();
                     for d_child in child.children(&mut d_cursor) {
                         if d_child.kind() == "field_identifier" || d_child.kind() == "identifier" {
                             name = Some(d_child.utf8_text(content.as_bytes()).unwrap_or("").to_string());
                             break;
                         }
                     }
                     if name.is_some() { break; }
                 }
             }
         } else {
             // Fallback for wrapped nodes (if any)
              let mut cursor = node.walk();
              for child in node.children(&mut cursor) {
                    if child.kind() == "init_declarator" || child.kind() == "field_declaration" {
                         // Recurse? Or just extract from this one.
                         return self.parse_field(content, child);
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

impl Default for CParser {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl Parser for CParser {
    fn file_extensions(&self) -> &[&str] {
        &["c", "h"]
    }

    fn language_name(&self) -> &str {
        "C"
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
        parser.set_language(&tree_sitter_c::LANGUAGE.into())
            .map_err(|e| VecqError::ParseError {
                file: PathBuf::from("<string>"),
                line: 0,
                message: format!("Failed to set C language: {}", e),
                source: None,
            })?;

        let tree = parser.parse(content, None)
            .ok_or_else(|| VecqError::ParseError {
                file: PathBuf::from("<string>"),
                line: 0,
                message: "Failed to parse C content".to_string(),
                source: None,
            })?;

        let elements = self.process_nodes(tree.root_node(), content);

        let mut doc = ParsedDocument::new(
            DocumentMetadata::new(PathBuf::from("file.c"), content.len() as u64)
                .with_line_count(content)
                .with_file_type(FileType::C)
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
        let parser = CParser::new();
        let content = r#"
int main(int argc, char** argv) {
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
    async fn test_parse_struct_nested() {
        let parser = CParser::new();
        let content = r#"
struct Point {
    int x;
    int y;
    struct Meta {
        int id;
    } meta;
};
"#;
        let result = parser.parse(content).await.unwrap();
        
        let structs: Vec<_> = result.elements.iter()
            .filter(|e| e.element_type == ElementType::Struct)
            .collect();
        
        assert_eq!(structs.len(), 1);
        let point = &structs[0];
        assert_eq!(point.name, Some("Point".to_string()));
         
        // Verify fields
        let has_x = point.children.iter().any(|c| c.name.as_deref() == Some("x") && c.element_type == ElementType::Variable);
        assert!(has_x, "Field x missing");
        
        // Verify nested struct 'Meta'
        let has_meta = point.children.iter().any(|c| c.name.as_deref() == Some("Meta") && c.element_type == ElementType::Struct);
        assert!(has_meta, "Nested struct Meta missing");
    }

    #[tokio::test]
    async fn test_parse_includes() {
        let parser = CParser::new();
        let content = r#"
#include <stdio.h>
#include "myheader.h"

int main() { return 0; }
"#;
        let result = parser.parse(content).await.unwrap();
        
        let imports: Vec<_> = result.elements.iter()
            .filter(|e| e.element_type == ElementType::Import)
            .collect();
        
        assert_eq!(imports.len(), 2);
    }
}
