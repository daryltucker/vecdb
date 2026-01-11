// PURPOSE:
//   JavaScript parser implementation for vecq using tree-sitter-javascript.
//   Extracts JS AST elements (functions, classes, imports, arrow functions).
//
// RELATED FILES:
//   - src/parsers/mod.rs - Parser registry
//   - src/types.rs - DocumentElement, ElementType definitions
//   - docs/ADDING_LANGUAGE_PARSER.md - Tutorial using this as example

use crate::error::{VecqError, VecqResult};
use crate::parser::{Parser, ParserCapabilities, ParserConfig};
use crate::types::{DocumentElement, DocumentMetadata, ElementType, ParsedDocument, JavaScriptAttributes, ElementAttributes};
use async_trait::async_trait;
use std::collections::HashMap;
use std::path::PathBuf;

/// JavaScript parser using tree-sitter
#[derive(Debug, Clone)]
pub struct JavaScriptParser {
    config: ParserConfig,
}

impl JavaScriptParser {
    pub fn new() -> Self {
        Self { config: ParserConfig::default() }
    }

    pub fn with_config(config: ParserConfig) -> Self {
        Self { config }
    }

    fn extract_functions(&self, content: &str, tree: &tree_sitter::Tree) -> VecqResult<Vec<DocumentElement>> {
        let mut functions = Vec::new();
        let root = tree.root_node();
        let mut cursor = root.walk();

        for child in root.children(&mut cursor) {
            // function declarations: function foo() {}
            if child.kind() == "function_declaration" {
                if let Some(func) = self.parse_function(content, child) {
                    functions.push(func);
                }
            }
            // arrow functions in variable declarations: const foo = () => {}
            if child.kind() == "lexical_declaration" || child.kind() == "variable_declaration" {
                if let Some(func) = self.parse_arrow_function(content, child) {
                    functions.push(func);
                }
            }
        }

        Ok(functions)
    }

    fn parse_function(&self, content: &str, node: tree_sitter::Node) -> Option<DocumentElement> {
        let mut name = String::new();
        let mut is_async = false;
        let mut cursor = node.walk();

        for child in node.children(&mut cursor) {
            if child.kind() == "async" {
                is_async = true;
            }
            if child.kind() == "identifier" {
                name = child.utf8_text(content.as_bytes()).unwrap_or("").to_string();
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
        .set_attributes(ElementAttributes::JavaScript(JavaScriptAttributes {
            is_async,
            is_arrow: false,
            other: HashMap::new(),
        }));

        Some(element)
    }

    fn parse_arrow_function(&self, content: &str, node: tree_sitter::Node) -> Option<DocumentElement> {
        let text = node.utf8_text(content.as_bytes()).unwrap_or("");
        
        if !text.contains("=>") {
            return None;
        }

        let mut cursor = node.walk();
        for child in node.children(&mut cursor) {
            if child.kind() == "variable_declarator" {
                let mut decl_cursor = child.walk();
                for decl_child in child.children(&mut decl_cursor) {
                    if decl_child.kind() == "identifier" {
                        let name = decl_child.utf8_text(content.as_bytes()).unwrap_or("").to_string();
                        let is_async = text.contains("async");
                        
                        return Some(DocumentElement::new(
                            ElementType::Function,
                            Some(name),
                            text.to_string(),
                            node.start_position().row + 1,
                            node.end_position().row + 1,
                        )
                        .set_attributes(ElementAttributes::JavaScript(JavaScriptAttributes {
                            is_async,
                            is_arrow: true,
                            other: HashMap::new(),
                        })));
                    }
                }
            }
        }
        None
    }

    fn extract_classes(&self, content: &str, tree: &tree_sitter::Tree) -> VecqResult<Vec<DocumentElement>> {
        let mut classes = Vec::new();
        let root = tree.root_node();
        let mut cursor = root.walk();

        for child in root.children(&mut cursor) {
            if child.kind() == "class_declaration" {
                if let Some(cls) = self.parse_class(content, child) {
                    classes.push(cls);
                }
            }
        }

        Ok(classes)
    }

    fn parse_class(&self, content: &str, node: tree_sitter::Node) -> Option<DocumentElement> {
        let mut name: Option<String> = None;
        let mut cursor = node.walk();

        for child in node.children(&mut cursor) {
            if child.kind() == "identifier" {
                name = Some(child.utf8_text(content.as_bytes()).unwrap_or("").to_string());
                break;
            }
        }

        Some(DocumentElement::new(
            ElementType::Class,
            name,
            node.utf8_text(content.as_bytes()).unwrap_or("").to_string(),
            node.start_position().row + 1,
            node.end_position().row + 1,
        ))
    }

    fn extract_imports(&self, content: &str, tree: &tree_sitter::Tree) -> VecqResult<Vec<DocumentElement>> {
        let mut imports = Vec::new();
        let root = tree.root_node();
        let mut cursor = root.walk();

        for child in root.children(&mut cursor) {
            if child.kind() == "import_statement" {
                let text = child.utf8_text(content.as_bytes()).unwrap_or("");
                
                imports.push(DocumentElement::new(
                    ElementType::Import,
                    None,
                    text.to_string(),
                    child.start_position().row + 1,
                    child.end_position().row + 1,
                ));
            }
        }

        Ok(imports)
    }
}

impl Default for JavaScriptParser {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl Parser for JavaScriptParser {
    fn file_extensions(&self) -> &[&str] {
        &["js", "mjs", "cjs"]
    }

    fn language_name(&self) -> &str {
        "JavaScript"
    }

    fn capabilities(&self) -> ParserCapabilities {
        ParserCapabilities {
            incremental: false,
            error_recovery: true,
            documentation: false,
            type_information: false,
            macros: false,
            max_file_size: None,
        }
    }

    async fn parse(&self, content: &str) -> VecqResult<ParsedDocument> {
        let mut parser = tree_sitter::Parser::new();
        parser.set_language(&tree_sitter_javascript::LANGUAGE.into())
            .map_err(|e| VecqError::ParseError {
                file: PathBuf::from("<string>"),
                line: 0,
                message: format!("Failed to set JavaScript language: {}", e),
                source: None,
            })?;

        let tree = parser.parse(content, None)
            .ok_or_else(|| VecqError::ParseError {
                file: PathBuf::from("<string>"),
                line: 0,
                message: "Failed to parse JavaScript content".to_string(),
                source: None,
            })?;

        let functions = self.extract_functions(content, &tree)?;
        let classes = self.extract_classes(content, &tree)?;
        let imports = self.extract_imports(content, &tree)?;

        let mut elements = Vec::new();
        elements.extend(functions);
        elements.extend(classes);
        elements.extend(imports);

        let mut doc = ParsedDocument::new(DocumentMetadata::new(PathBuf::new(), 0));
        doc.elements = elements;

        Ok(doc)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_parse_function_declaration() {
        let parser = JavaScriptParser::new();
        let content = r#"
function greet(name) {
    return "Hello, " + name;
}
"#;
        let result = parser.parse(content).await.unwrap();
        
        let functions: Vec<_> = result.elements.iter()
            .filter(|e| e.element_type == ElementType::Function)
            .collect();
        
        assert_eq!(functions.len(), 1);
        assert_eq!(functions[0].name, Some("greet".to_string()));
    }

    #[tokio::test]
    async fn test_parse_async_function() {
        let parser = JavaScriptParser::new();
        let content = "async function fetchData() { return await fetch('/api'); }";
        let result = parser.parse(content).await.unwrap();
        
        let functions: Vec<_> = result.elements.iter()
            .filter(|e| e.element_type == ElementType::Function)
            .collect();
        
        assert_eq!(functions.len(), 1);
        if let ElementAttributes::JavaScript(attrs) = &functions[0].attributes {
            assert!(attrs.is_async);
        } else {
            panic!("Expected JavaScriptAttributes");
        }
    }

    #[tokio::test]
    async fn test_parse_arrow_function() {
        let parser = JavaScriptParser::new();
        let content = "const add = (a, b) => a + b;";
        let result = parser.parse(content).await.unwrap();
        
        let functions: Vec<_> = result.elements.iter()
            .filter(|e| e.element_type == ElementType::Function)
            .collect();
        
        assert_eq!(functions.len(), 1);
        assert_eq!(functions[0].name, Some("add".to_string()));
        if let ElementAttributes::JavaScript(attrs) = &functions[0].attributes {
            assert!(attrs.is_arrow);
        } else {
            panic!("Expected JavaScriptAttributes");
        }
    }

    #[tokio::test]
    async fn test_parse_class() {
        let parser = JavaScriptParser::new();
        let content = r#"
class User {
    constructor(name) {
        this.name = name;
    }
}
"#;
        let result = parser.parse(content).await.unwrap();
        
        let classes: Vec<_> = result.elements.iter()
            .filter(|e| e.element_type == ElementType::Class)
            .collect();
        
        assert_eq!(classes.len(), 1);
        assert_eq!(classes[0].name, Some("User".to_string()));
    }

    #[tokio::test]
    async fn test_parse_imports() {
        let parser = JavaScriptParser::new();
        let content = r#"
import { useState } from 'react';
import axios from 'axios';
"#;
        let result = parser.parse(content).await.unwrap();
        
        let imports: Vec<_> = result.elements.iter()
            .filter(|e| e.element_type == ElementType::Import)
            .collect();
        
        assert_eq!(imports.len(), 2);
    }

    #[tokio::test]
    async fn test_empty_content() {
        let parser = JavaScriptParser::new();
        let result = parser.parse("").await.unwrap();
        assert!(result.elements.is_empty());
    }
}
