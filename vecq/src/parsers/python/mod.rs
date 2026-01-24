// Python parser module - public interface and Parser trait implementation

use crate::error::{VecqError, VecqResult};
use crate::parser::{Parser, ParserCapabilities, ParserConfig};
use crate::types::{DocumentMetadata, ElementType, FileType, ParsedDocument};
use async_trait::async_trait;
use rustpython_parser::{ast, Parse};
use std::path::PathBuf;

pub mod visitor;
pub mod usage;

/// Python parser that extracts structural elements from Python source code
#[derive(Debug, Clone)]
pub struct PythonParser {
    enable_usages: bool,
    current_scope: String,
    _config: ParserConfig,
}

impl PythonParser {
    /// Create a new Python parser with default configuration
    pub fn new() -> Self {
        Self {
            enable_usages: false,
            current_scope: "global".to_string(),
            _config: ParserConfig::default(),
        }
    }

    /// Enable or disable usage/reference detection
    pub fn with_usages(mut self, enable: bool) -> Self {
        self.enable_usages = enable;
        self
    }

    /// Create a new Python parser with custom configuration
    pub fn with_config(config: ParserConfig) -> Self {
        Self {
            enable_usages: false,
            current_scope: "global".to_string(),
            _config: config,
        }
    }

    /// Helper to convert byte offset to line number (0-indexed to match core)
    pub fn byte_offset_to_line_number(&self, content: &str, offset: usize) -> usize {
        content[..offset.min(content.len())]
            .chars()
            .filter(|&c| c == '\n')
            .count()
    }
}

impl Default for PythonParser {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl Parser for PythonParser {
    fn file_extensions(&self) -> &[&str] {
        &["py", "pyw", "pyi"]
    }

    fn language_name(&self) -> &str {
        "Python"
    }

    fn capabilities(&self) -> ParserCapabilities {
        ParserCapabilities {
            incremental: false,
            error_recovery: false,
            documentation: true,
            type_information: true,
            macros: false,
            max_file_size: None,
        }
    }

    async fn parse(&self, content: &str) -> VecqResult<ParsedDocument> {
        // Parse Python AST
        let ast = ast::Suite::parse(content, "<string>")
            .map_err(|e| VecqError::parse_error(
                PathBuf::from("<string>"),
                0,
                format!("Python parsing failed: {}", e),
                None::<std::io::Error>,
            ))?;

        let mut elements = Vec::new();

        // Extract all structural elements
        elements.extend(self.extract_functions(content, &ast)?);
        elements.extend(self.extract_classes(content, &ast)?);
        elements.extend(self.extract_imports(content, &ast)?);
        elements.extend(self.extract_variables(content, &ast)?);

        // Extract usage/reference elements if enabled
        if self.enable_usages {
            elements.extend(self.detect_usages(content, &ast, None, &self.current_scope)?);
        }

        // Create metadata
        let mut metadata = DocumentMetadata::new(PathBuf::from("<string>"), content.len() as u64)
            .with_line_count(content);
        metadata.file_type = FileType::Python;

        Ok(ParsedDocument::new(metadata).add_elements(elements))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_parse_function() {
        let parser = PythonParser::new();
        let content = r#"
def greet(name: str) -> str:
    """Greet someone by name."""
    return f"Hello {name}"
"#;
        let result = parser.parse(content).await.unwrap();

        let functions: Vec<_> = result.elements.iter()
            .filter(|e| e.element_type == ElementType::Function)
            .collect();

        assert_eq!(functions.len(), 1);
        assert_eq!(functions[0].name, Some("greet".to_string()));
    }

    #[tokio::test]
    async fn test_parse_class() {
        let parser = PythonParser::new();
        let content = r#"
class User:
    """A user class."""

    def __init__(self, name: str):
        self.name = name

    def get_name(self) -> str:
        return self.name
"#;
        let result = parser.parse(content).await.unwrap();

        let classes: Vec<_> = result.elements.iter()
            .filter(|e| e.element_type == ElementType::Class)
            .collect();
        let functions: Vec<_> = result.elements.iter()
            .filter(|e| e.element_type == ElementType::Function)
            .collect();

        assert_eq!(classes.len(), 1);
        assert_eq!(classes[0].name, Some("User".to_string()));
        assert_eq!(functions.len(), 0); // Class methods are children of the class, not top-level elements

        // Check that the class has methods as children
        let method_count = classes[0].children.iter()
            .filter(|c| c.element_type == ElementType::Function)
            .count();
        assert_eq!(method_count, 2); // __init__ and get_name
    }

    #[tokio::test]
    async fn test_parse_imports() {
        let parser = PythonParser::new();
        let content = r#"
import os
from typing import List, Optional
import json as j
"#;
        let result = parser.parse(content).await.unwrap();

        let imports: Vec<_> = result.elements.iter()
            .filter(|e| e.element_type == ElementType::Import)
            .collect();

        assert_eq!(imports.len(), 4); // os, List, Optional, json as j
    }

    #[tokio::test]
    async fn test_parse_variables() {
        let parser = PythonParser::new();
        let content = r#"
x = 42
name: str = "world"
PI = 3.14159
"#;
        let result = parser.parse(content).await.unwrap();

        let variables: Vec<_> = result.elements.iter()
            .filter(|e| e.element_type == ElementType::Variable)
            .collect();

        assert_eq!(variables.len(), 3);
    }

    #[tokio::test]
    async fn test_usage_detection_function_calls() {
        let parser = PythonParser::new().with_usages(true);
        let content = r#"
def greet(name):
    result = "Hello " + name
    print(result)
    return len(result)

def main():
    greet("world")
"#;

        let result = parser.parse(content).await.unwrap();

        // Should detect function calls: print(), len(), greet()
        let function_calls: Vec<_> = result
            .elements
            .iter()
            .filter(|e| e.element_type == ElementType::FunctionCall)
            .collect();
        assert_eq!(function_calls.len(), 3);

        // Should detect method calls: str.__add__() (implicit in "Hello " + name)
        let _method_calls: Vec<_> = result
            .elements
            .iter()
            .filter(|e| e.element_type == ElementType::MethodCall)
            .collect();
        // Note: Python's + operator doesn't create explicit method calls in AST

        // Check function call names
        let function_names: Vec<String> = function_calls
            .iter()
            .filter_map(|e| e.name.clone())
            .collect();
        assert!(function_names.contains(&"print".to_string()));
        assert!(function_names.contains(&"len".to_string()));
        assert!(function_names.contains(&"greet".to_string()));
    }

    #[tokio::test]
    async fn test_usage_detection_assignments() {
        let parser = PythonParser::new().with_usages(true);
        let content = r#"
def main():
    x = 42
    y = 10
    z = x + y
"#;

        let result = parser.parse(content).await.unwrap();

        let assignments: Vec<_> = result
            .elements
            .iter()
            .filter(|e| e.element_type == ElementType::Assignment)
            .collect();
        // Should detect assignments: x, y, z
        assert_eq!(assignments.len(), 3);

        // Check assignment names
        let assignment_names: Vec<String> = assignments
            .iter()
            .filter_map(|e| e.name.clone())
            .collect();
        assert!(assignment_names.contains(&"x".to_string()));
        assert!(assignment_names.contains(&"y".to_string()));
        assert!(assignment_names.contains(&"z".to_string()));
    }

    #[tokio::test]
    async fn test_usage_detection_import_usage() {
        let parser = PythonParser::new().with_usages(true);
        let content = r#"
import os
from typing import List

def main():
    os.path.join("a", "b")
    items: List[str] = []
"#;

        let result = parser.parse(content).await.unwrap();

        let import_usages: Vec<_> = result
            .elements
            .iter()
            .filter(|e| e.element_type == ElementType::ImportUsage)
            .collect();
        // Should detect import usages: os, List
        assert!(import_usages.len() >= 2);

        // Check that we have import names
        let import_names: Vec<String> = import_usages
            .iter()
            .filter_map(|e| e.name.clone())
            .collect();
        assert!(import_names.contains(&"os".to_string()));
        assert!(import_names.contains(&"List".to_string()));
    }
}