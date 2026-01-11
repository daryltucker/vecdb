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

    fn extract_functions(&self, content: &str, tree: &tree_sitter::Tree) -> VecqResult<Vec<DocumentElement>> {
        let mut functions = Vec::new();
        let root = tree.root_node();
        let mut cursor = root.walk();

        for child in root.children(&mut cursor) {
            if child.kind() == "function_declaration" || child.kind() == "method_declaration" {
                if let Some(func) = self.parse_function(content, child) {
                    functions.push(func);
                }
            }
        }

        Ok(functions)
    }

    fn parse_function(&self, content: &str, node: tree_sitter::Node) -> Option<DocumentElement> {
        let mut name = String::new();
        let mut cursor = node.walk();

        for child in node.children(&mut cursor) {
            if child.kind() == "identifier" {
                name = child.utf8_text(content.as_bytes()).unwrap_or("").to_string();
                break;
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
        ).set_attributes(ElementAttributes::Go(GoAttributes {
            other: HashMap::new(),
        })))
    }

    fn extract_structs(&self, content: &str, tree: &tree_sitter::Tree) -> VecqResult<Vec<DocumentElement>> {
        let mut structs = Vec::new();
        let root = tree.root_node();
        let mut cursor = root.walk();

        for child in root.children(&mut cursor) {
            if child.kind() == "type_declaration" {
                if let Some(s) = self.parse_type_declaration(content, child) {
                    structs.push(s);
                }
            }
        }

        Ok(structs)
    }

    fn parse_type_declaration(&self, content: &str, node: tree_sitter::Node) -> Option<DocumentElement> {
        let mut name: Option<String> = None;
        let mut element_type = ElementType::Struct;
        let mut cursor = node.walk();

        for child in node.children(&mut cursor) {
            if child.kind() == "type_spec" {
                let mut spec_cursor = child.walk();
                for spec_child in child.children(&mut spec_cursor) {
                    if spec_child.kind() == "type_identifier" {
                        name = Some(spec_child.utf8_text(content.as_bytes()).unwrap_or("").to_string());
                    }
                    if spec_child.kind() == "interface_type" {
                        element_type = ElementType::Interface;
                    }
                }
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

    fn extract_imports(&self, content: &str, tree: &tree_sitter::Tree) -> VecqResult<Vec<DocumentElement>> {
        let mut imports = Vec::new();
        let root = tree.root_node();
        let mut cursor = root.walk();

        for child in root.children(&mut cursor) {
            if child.kind() == "import_declaration" {
                let text = child.utf8_text(content.as_bytes()).unwrap_or("");
                
                imports.push(DocumentElement::new(
                    ElementType::Import,
                    None,
                    text.to_string(),
                    child.start_position().row + 1,
                    child.end_position().row + 1,
                ).set_attributes(ElementAttributes::Go(GoAttributes {
                    other: HashMap::new(),
                })));
            }
        }

        Ok(imports)
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

        let functions = self.extract_functions(content, &tree)?;
        let structs = self.extract_structs(content, &tree)?;
        let imports = self.extract_imports(content, &tree)?;

        let mut elements = Vec::new();
        elements.extend(functions);
        elements.extend(structs);
        elements.extend(imports);

        let mut doc = ParsedDocument::new(
            DocumentMetadata::new(PathBuf::from("file.go"), content.len() as u64)
                .with_line_count(content)
                .with_file_type(FileType::Go)
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
        let parser = GoParser::new();
        let content = r#"
package main

func main() {
    println("Hello")
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
        let parser = GoParser::new();
        let content = r#"
package main

type Point struct {
    X int
    Y int
}
"#;
        let result = parser.parse(content).await.unwrap();
        
        let structs: Vec<_> = result.elements.iter()
            .filter(|e| e.element_type == ElementType::Struct)
            .collect();
        
        assert_eq!(structs.len(), 1);
        assert_eq!(structs[0].name, Some("Point".to_string()));
    }

    #[tokio::test]
    async fn test_parse_interface() {
        let parser = GoParser::new();
        let content = r#"
package main

type Reader interface {
    Read(p []byte) (n int, err error)
}
"#;
        let result = parser.parse(content).await.unwrap();
        
        let interfaces: Vec<_> = result.elements.iter()
            .filter(|e| e.element_type == ElementType::Interface)
            .collect();
        
        assert_eq!(interfaces.len(), 1);
        assert_eq!(interfaces[0].name, Some("Reader".to_string()));
    }

    #[tokio::test]
    async fn test_empty_content() {
        let parser = GoParser::new();
        let result = parser.parse("").await.unwrap();
        assert!(result.elements.is_empty());
    }
}
