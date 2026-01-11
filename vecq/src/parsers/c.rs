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

    /// Extract function definitions from the AST
    fn extract_functions(&self, content: &str, tree: &tree_sitter::Tree) -> VecqResult<Vec<DocumentElement>> {
        let mut functions = Vec::new();
        let root = tree.root_node();
        let mut cursor = root.walk();

        for child in root.children(&mut cursor) {
            if child.kind() == "function_definition" {
                if let Some(func) = self.parse_function_definition(content, child) {
                    functions.push(func);
                }
            }
        }

        Ok(functions)
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
                map.insert("parameters".to_string(), json!(params));
                map
            }
        }));

        Some(element)
    }

    fn extract_structs(&self, content: &str, tree: &tree_sitter::Tree) -> VecqResult<Vec<DocumentElement>> {
        let mut structs = Vec::new();
        let root = tree.root_node();
        let mut cursor = root.walk();

        for child in root.children(&mut cursor) {
            if child.kind() == "struct_specifier" || child.kind() == "type_definition" {
                if let Some(s) = self.parse_struct_definition(content, child) {
                    structs.push(s);
                }
            }
        }

        Ok(structs)
    }

    fn parse_struct_definition(&self, content: &str, node: tree_sitter::Node) -> Option<DocumentElement> {
        let mut name: Option<String> = None;
        let mut cursor = node.walk();

        for child in node.children(&mut cursor) {
            if child.kind() == "type_identifier" {
                name = Some(child.utf8_text(content.as_bytes()).unwrap_or("").to_string());
                break;
            }
        }

        Some(DocumentElement::new(
            ElementType::Struct,
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

        let functions = self.extract_functions(content, &tree)?;
        let structs = self.extract_structs(content, &tree)?;
        let includes = self.extract_includes(content, &tree)?;

        let mut elements = Vec::new();
        elements.extend(functions);
        elements.extend(structs);
        elements.extend(includes);

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
    async fn test_parse_struct() {
        let parser = CParser::new();
        let content = r#"
struct Point {
    int x;
    int y;
};
"#;
        let result = parser.parse(content).await.unwrap();
        
        let structs: Vec<_> = result.elements.iter()
            .filter(|e| e.element_type == ElementType::Struct)
            .collect();
        
        assert!(!structs.is_empty());
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
        assert!(imports[0].attributes.get("is_system").unwrap().as_bool().unwrap());
        assert!(!imports[1].attributes.get("is_system").unwrap().as_bool().unwrap());
    }

    #[tokio::test]
    async fn test_empty_content() {
        let parser = CParser::new();
        let result = parser.parse("").await.unwrap();
        assert!(result.elements.is_empty());
    }

    #[tokio::test]
    async fn test_multiple_functions() {
        let parser = CParser::new();
        let content = r#"
void foo() {}
int bar(int x) { return x; }
static void baz() {}
"#;
        let result = parser.parse(content).await.unwrap();
        
        let functions: Vec<_> = result.elements.iter()
            .filter(|e| e.element_type == ElementType::Function)
            .collect();
        
        assert_eq!(functions.len(), 3);
    }
}
