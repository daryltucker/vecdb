// PURPOSE:
//   Bash parser implementation for vecq using tree-sitter-bash.
//   Extracts Bash AST elements (functions, aliases, variables).
//
// RELATED FILES:
//   - src/parsers/c.rs - Reference implementation pattern
//   - src/types.rs - DocumentElement, ElementType definitions

use crate::error::{VecqError, VecqResult};
use crate::parser::{Parser, ParserCapabilities, ParserConfig};
use crate::types::{DocumentElement, DocumentMetadata, ElementType, FileType, ParsedDocument, BashAttributes, ElementAttributes};
use std::collections::HashMap;
use async_trait::async_trait;
use std::path::PathBuf;

/// Bash parser that extracts structural elements from Bash scripts
#[derive(Debug, Clone)]
pub struct BashParser {
    _config: ParserConfig,
}

impl BashParser {
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
            if child.kind() == "function_definition" {
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

        // Function name can be in different positions depending on syntax
        // function name { } or name() { }
        for child in node.children(&mut cursor) {
            if child.kind() == "word" {
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
        ).set_attributes(ElementAttributes::Bash(BashAttributes {
            other: HashMap::new(),
        })))
    }

    fn extract_variables(&self, content: &str, tree: &tree_sitter::Tree) -> VecqResult<Vec<DocumentElement>> {
        let mut variables = Vec::new();
        let root = tree.root_node();
        let mut cursor = root.walk();

        for child in root.children(&mut cursor) {
            if child.kind() == "variable_assignment" {
                if let Some(var) = self.parse_variable(content, child) {
                    variables.push(var);
                }
            }
        }

        Ok(variables)
    }

    fn parse_variable(&self, content: &str, node: tree_sitter::Node) -> Option<DocumentElement> {
        let mut name: Option<String> = None;
        let mut cursor = node.walk();

        for child in node.children(&mut cursor) {
            if child.kind() == "variable_name" {
                name = Some(child.utf8_text(content.as_bytes()).unwrap_or("").to_string());
                break;
            }
        }

        Some(DocumentElement::new(
            ElementType::Variable,
            name,
            node.utf8_text(content.as_bytes()).unwrap_or("").to_string(),
            node.start_position().row + 1,
            node.end_position().row + 1,
        ).set_attributes(ElementAttributes::Bash(BashAttributes {
            other: HashMap::new(),
        })))
    }
}

impl Default for BashParser {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl Parser for BashParser {
    fn file_extensions(&self) -> &[&str] {
        &["sh", "bash"]
    }

    fn language_name(&self) -> &str {
        "Bash"
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
        parser.set_language(&tree_sitter_bash::LANGUAGE.into())
            .map_err(|e| VecqError::ParseError {
                file: PathBuf::from("<string>"),
                line: 0,
                message: format!("Failed to set Bash language: {}", e),
                source: None,
            })?;

        let tree = parser.parse(content, None)
            .ok_or_else(|| VecqError::ParseError {
                file: PathBuf::from("<string>"),
                line: 0,
                message: "Failed to parse Bash content".to_string(),
                source: None,
            })?;

        let functions = self.extract_functions(content, &tree)?;
        let variables = self.extract_variables(content, &tree)?;

        let mut elements = Vec::new();
        elements.extend(functions);
        elements.extend(variables);

        let mut doc = ParsedDocument::new(
            DocumentMetadata::new(PathBuf::from("file.sh"), content.len() as u64)
                .with_line_count(content)
                .with_file_type(FileType::Bash)
        );
        doc.elements = elements;

        Ok(doc)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_parse_function_keyword() {
        let parser = BashParser::new();
        let content = r#"
function hello {
    echo "Hello, World!"
}
"#;
        let result = parser.parse(content).await.unwrap();
        
        let functions: Vec<_> = result.elements.iter()
            .filter(|e| e.element_type == ElementType::Function)
            .collect();
        
        assert_eq!(functions.len ( ), 1);
        assert_eq!(functions[0].name, Some("hello".to_string()));
    }

    #[tokio::test]
    async fn test_parse_function_parentheses() {
        let parser = BashParser::new();
        let content = r#"
greet() {
    echo "Hi!"
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
    async fn test_parse_variables() {
        let parser = BashParser::new();
        let content = r#"
NAME="World"
COUNT=42
"#;
        let result = parser.parse(content).await.unwrap();
        
        let variables: Vec<_> = result.elements.iter()
            .filter(|e| e.element_type == ElementType::Variable)
            .collect();
        
        assert_eq!(variables.len(), 2);
    }

    #[tokio::test]
    async fn test_empty_content() {
        let parser = BashParser::new();
        let result = parser.parse("").await.unwrap();
        assert!(result.elements.is_empty());
    }
}
