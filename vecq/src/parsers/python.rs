// PURPOSE:
//   Python parser implementation for vecq using rustpython-parser.
//   Extracts Python AST elements (classes, functions, imports, variables) while
//   preserving line numbers and structural relationships. Essential for making
//   Python codebases queryable with queries like "Find all classes with async methods".
//
// REQUIREMENTS:
//   User-specified:
//   - Must extract all function definitions with parameters, return types, decorators
//   - Must parse class definitions with inheritance, methods, and attributes
//   - Must handle import statements (import, from...import, as aliases)
//   - Must extract global variables and constants with type annotations
//   - Must preserve docstrings and comments for documentation queries
//   
//   Implementation-discovered:
//   - Requires rustpython-parser for Python AST parsing
//   - Must handle Python-specific features like list comprehensions, decorators
//   - Needs to track indentation levels for proper scope analysis
//   - Must handle both Python 2 and 3 syntax variations gracefully
//
// IMPLEMENTATION RULES:
//   1. Use rustpython-parser for all Python AST parsing
//      Rationale: Provides accurate Python syntax parsing with proper error handling
//   
//   2. Extract all function metadata including decorators and type hints
//      Rationale: Modern Python uses extensive type annotations and decorators
//   
//   3. Preserve class inheritance relationships and method resolution order
//      Rationale: Essential for understanding Python object hierarchies
//   
//   4. Handle import statements with full module path resolution
//      Rationale: Python imports are complex with relative/absolute paths
//   
//   5. Extract docstrings as separate queryable elements
//      Rationale: Python docstrings are first-class documentation
//   
//   Critical:
//   - DO NOT lose type annotation information from function signatures
//   - DO NOT ignore decorator information as it affects behavior
//   - ALWAYS preserve line numbers for all AST elements
//
// USAGE:
//   use vecq::parsers::PythonParser;
//   use vecq::parser::Parser;
//   
//   let parser = PythonParser::new();
//   let content = "def hello(name: str) -> str:\n    return f'Hello {name}'";
//   let parsed = parser.parse(content).await?;
//
// SELF-HEALING INSTRUCTIONS:
//   When adding new Python language features:
//   1. Update the AST visitor to handle new node types
//   2. Add corresponding JSON schema elements
//   3. Update property tests to generate new syntax patterns
//   4. Test with real-world Python files using the new features
//   5. Update documentation with new queryable elements
//
// RELATED FILES:
//   - src/parsers/mod.rs - Parser registry that includes this parser
//   - src/types.rs - FileType enum that includes Python
//   - tests/property_python_parsing.rs - Property tests for this parser
//   - Cargo.toml - Dependencies including rustpython-parser
//
// MAINTENANCE:
//   Update when:
//   - New Python language features are released (match expressions, etc.)
//   - rustpython-parser API changes require adaptation
//   - JSON schema needs modification for new query patterns
//   - Performance issues are discovered with large Python files
//
// Last Verified: 2025-12-31

use crate::error::{VecqError, VecqResult};
use crate::parser::{Parser, ParserCapabilities, ParserConfig};
use crate::types::{DocumentElement, DocumentMetadata, ElementType, FileType, ParsedDocument, PythonAttributes, ElementAttributes};
use async_trait::async_trait;
use rustpython_parser::{ast, Parse};
use serde_json::json;
use std::collections::HashMap;
use std::path::PathBuf;

/// Python parser that extracts structural elements from Python source code
#[derive(Debug, Clone)]
pub struct PythonParser {
    _config: ParserConfig,
}

impl PythonParser {
    /// Create a new Python parser with default configuration
    pub fn new() -> Self {
        Self {
            _config: ParserConfig::default(),
        }
    }

    /// Create a new Python parser with custom configuration
    pub fn with_config(config: ParserConfig) -> Self {
        Self { _config: config }
    }

    /// Helper to convert byte offset to line number (0-indexed to match core)
    fn byte_offset_to_line_number(&self, content: &str, offset: usize) -> usize {
        content[..offset.min(content.len())]
            .chars()
            .filter(|&c| c == '\n')
            .count()
    }

    /// Extract function definitions from Python AST
    fn extract_functions(&self, content: &str, body: &[ast::Stmt]) -> VecqResult<Vec<DocumentElement>> {
        let mut functions = Vec::new();
        
        for stmt in body {
            if let ast::Stmt::FunctionDef(func_def) = stmt {
                let mut attributes = HashMap::new();
                
                // Extract function name
                attributes.insert("name".to_string(), json!(func_def.name.to_string()));
                
                // Extract parameters
                let mut params = Vec::new();
                for arg in &func_def.args.args {
                    let mut param = HashMap::new();
                    param.insert("name", json!(arg.def.arg.to_string()));
                    if let Some(annotation) = &arg.def.annotation {
                        param.insert("type", json!(self.ast_to_string(annotation)));
                    }
                    params.push(json!(param));
                }
                
                // Extract return type annotation
                let mut returns_str: Option<String> = None;
                if let Some(returns) = &func_def.returns {
                    returns_str = Some(self.ast_to_string(returns));
                }
                
                // Extract decorators
                let decorators: Vec<String> = func_def.decorator_list
                    .iter()
                    .map(|d| self.ast_to_string(d))
                    .collect();
                
                // Check for async
                attributes.insert("is_async".to_string(), json!(false));
                
                // Extract docstring if present
                if let Some(ast::Stmt::Expr(expr)) = func_def.body.first() {
                    if let ast::Expr::Constant(constant) = &*expr.value {
                        if let ast::Constant::Str(docstring) = &constant.value {
                            attributes.insert("docstring".to_string(), json!(docstring));
                        }
                    }
                }
                
                attributes.insert("parameters".to_string(), json!(params));
                if let Some(returns) = returns_str {
                    attributes.insert("return_type".to_string(), json!(returns));
                }
                attributes.insert("decorators".to_string(), json!(decorators));

                let element = DocumentElement::new(
                    ElementType::Function,
                    Some(func_def.name.to_string()),
                    format!("def {}(...)", func_def.name),
                    self.byte_offset_to_line_number(content, func_def.range.start().to_u32() as usize),
                    self.byte_offset_to_line_number(content, func_def.range.end().to_u32() as usize),
                ).set_attributes(ElementAttributes::Python(PythonAttributes {
                    is_async: false,
                    other: attributes,
                }));
                
                functions.push(element);
            } else if let ast::Stmt::AsyncFunctionDef(async_func_def) = stmt {
                let mut attributes = HashMap::new();
                
                // Extract function name
                attributes.insert("name".to_string(), json!(async_func_def.name.to_string()));
                
                // Extract parameters
                let mut params = Vec::new();
                for arg in &async_func_def.args.args {
                    let mut param = HashMap::new();
                    param.insert("name", json!(arg.def.arg.to_string()));
                    if let Some(annotation) = &arg.def.annotation {
                        param.insert("type", json!(self.ast_to_string(annotation)));
                    }
                    params.push(json!(param));
                }
                attributes.insert("parameters".to_string(), json!(params));
                
                // Extract return type annotation
                if let Some(returns) = &async_func_def.returns {
                    attributes.insert("return_type".to_string(), json!(self.ast_to_string(returns)));
                }
                
                // Extract decorators
                let decorators: Vec<String> = async_func_def.decorator_list
                    .iter()
                    .map(|d| self.ast_to_string(d))
                    .collect();
                attributes.insert("decorators".to_string(), json!(decorators));
                
                // Mark as async
                attributes.insert("is_async".to_string(), json!(true));
                
                // Extract docstring if present
                if let Some(ast::Stmt::Expr(expr)) = async_func_def.body.first() {
                    if let ast::Expr::Constant(constant) = &*expr.value {
                        if let ast::Constant::Str(docstring) = &constant.value {
                            attributes.insert("docstring".to_string(), json!(docstring));
                        }
                    }
                }
                
                let element = DocumentElement::new(
                    ElementType::Function,
                    Some(async_func_def.name.to_string()),
                    format!("async def {}(...)", async_func_def.name),
                    self.byte_offset_to_line_number(content, async_func_def.range.start().to_u32() as usize),
                    self.byte_offset_to_line_number(content, async_func_def.range.end().to_u32() as usize),
                ).set_attributes(ElementAttributes::Python(PythonAttributes {
                    is_async: true,
                    other: attributes,
                }));
                
                functions.push(element);
            }
        }
        
        Ok(functions)
    }

    /// Extract class definitions from Python AST
    fn extract_classes(&self, content: &str, body: &[ast::Stmt]) -> VecqResult<Vec<DocumentElement>> {
        let mut classes = Vec::new();
        
        for stmt in body {
            if let ast::Stmt::ClassDef(class_def) = stmt {
                let mut attributes = HashMap::new();
                
                // Extract class name
                attributes.insert("name".to_string(), json!(class_def.name.to_string()));
                
                // Extract base classes
                let bases: Vec<String> = class_def.bases
                    .iter()
                    .map(|b| self.ast_to_string(b))
                    .collect();
                attributes.insert("bases".to_string(), json!(bases));
                
                // Extract decorators
                let decorators: Vec<String> = class_def.decorator_list
                    .iter()
                    .map(|d| self.ast_to_string(d))
                    .collect();
                attributes.insert("decorators".to_string(), json!(decorators));
                
                // Extract docstring if present
                if let Some(ast::Stmt::Expr(expr)) = class_def.body.first() {
                    if let ast::Expr::Constant(constant) = &*expr.value {
                        if let ast::Constant::Str(docstring) = &constant.value {
                            attributes.insert("docstring".to_string(), json!(docstring));
                        }
                    }
                }
                
                // Extract methods
                let methods = self.extract_functions(content, &class_def.body)?;
                
                let element = DocumentElement::new(
                    ElementType::Class,
                    Some(class_def.name.to_string()),
                    format!("class {}(...)", class_def.name),
                    self.byte_offset_to_line_number(content, class_def.range.start().to_u32() as usize),
                    self.byte_offset_to_line_number(content, class_def.range.end().to_u32() as usize),
                ).set_attributes(ElementAttributes::Python(PythonAttributes {
                    is_async: false,
                    other: attributes,
                })).with_children(methods);
                
                classes.push(element);
            }
        }
        
        Ok(classes)
    }

    /// Extract import statements from Python AST
    fn extract_imports(&self, content: &str, body: &[ast::Stmt]) -> VecqResult<Vec<DocumentElement>> {
        let mut imports = Vec::new();
        
        for stmt in body {
            match stmt {
                ast::Stmt::Import(import_stmt) => {
                    for alias in &import_stmt.names {
                        let mut attributes = HashMap::new();
                        attributes.insert("module".to_string(), json!(alias.name.to_string()));
                        if let Some(asname) = &alias.asname {
                            attributes.insert("alias".to_string(), json!(asname.to_string()));
                        }
                        attributes.insert("import_type".to_string(), json!("import"));
                        
                        let element = DocumentElement::new(
                            ElementType::Import,
                            Some(alias.asname.as_ref().unwrap_or(&alias.name).to_string()),
                            format!("import {}", alias.name),
                            self.byte_offset_to_line_number(content, import_stmt.range.start().to_u32() as usize),
                            self.byte_offset_to_line_number(content, import_stmt.range.end().to_u32() as usize),
                        ).set_attributes(ElementAttributes::Python(PythonAttributes {
                            is_async: false,
                            other: attributes,
                        }));
                        
                        imports.push(element);
                    }
                }
                ast::Stmt::ImportFrom(import_from) => {
                    let module = import_from.module.as_ref().map(|m| m.as_str()).unwrap_or("");
                    
                    for alias in &import_from.names {
                        let mut attributes = HashMap::new();
                        attributes.insert("module".to_string(), json!(module));
                        attributes.insert("name".to_string(), json!(alias.name.to_string()));
                        if let Some(asname) = &alias.asname {
                            attributes.insert("alias".to_string(), json!(asname.to_string()));
                        }
                        attributes.insert("import_type".to_string(), json!("from_import"));
                        if let Some(level) = import_from.level {
                            attributes.insert("level".to_string(), json!(level.to_u32()));
                        }
                        
                        let element = DocumentElement::new(
                            ElementType::Import,
                            Some(alias.asname.as_ref().unwrap_or(&alias.name).to_string()),
                            format!("from {} import {}", module, alias.name),
                            self.byte_offset_to_line_number(content, import_from.range.start().to_u32() as usize),
                            self.byte_offset_to_line_number(content, import_from.range.end().to_u32() as usize),
                        ).set_attributes(ElementAttributes::Python(PythonAttributes {
                            is_async: false,
                            other: attributes,
                        }));
                        
                        imports.push(element);
                    }
                }
                _ => {}
            }
        }
        
        Ok(imports)
    }

    /// Extract global variable assignments
    fn extract_variables(&self, content: &str, body: &[ast::Stmt]) -> VecqResult<Vec<DocumentElement>> {
        let mut variables = Vec::new();
        
        for stmt in body {
            if let ast::Stmt::Assign(assign) = stmt {
                for target in &assign.targets {
                    if let ast::Expr::Name(name) = target {
                        let mut attributes = HashMap::new();
                        attributes.insert("name".to_string(), json!(name.id.to_string()));
                        attributes.insert("value".to_string(), json!(self.ast_to_string(&assign.value)));
                        
                        let element = DocumentElement::new(
                            ElementType::Variable,
                            Some(name.id.to_string()),
                            format!("{} = ...", name.id),
                            self.byte_offset_to_line_number(content, assign.range.start().to_u32() as usize),
                            self.byte_offset_to_line_number(content, assign.range.end().to_u32() as usize),
                        ).set_attributes(ElementAttributes::Python(PythonAttributes {
                            is_async: false,
                            other: attributes,
                        }));
                        
                        variables.push(element);
                    }
                }
            } else if let ast::Stmt::AnnAssign(ann_assign) = stmt {
                if let ast::Expr::Name(name) = &*ann_assign.target {
                    let mut attributes = HashMap::new();
                    attributes.insert("name".to_string(), json!(name.id.to_string()));
                    attributes.insert("type".to_string(), json!(self.ast_to_string(&ann_assign.annotation)));
                    if let Some(value) = &ann_assign.value {
                        attributes.insert("value".to_string(), json!(self.ast_to_string(value)));
                    }
                    
                    let element = DocumentElement::new(
                        ElementType::Variable,
                        Some(name.id.to_string()),
                        format!("{}: {} = ...", name.id, self.ast_to_string(&ann_assign.annotation)),
                        self.byte_offset_to_line_number(content, ann_assign.range.start().to_u32() as usize),
                        self.byte_offset_to_line_number(content, ann_assign.range.end().to_u32() as usize),
                    ).set_attributes(ElementAttributes::Python(PythonAttributes {
                        is_async: false,
                        other: attributes,
                    }));
                    
                    variables.push(element);
                }
            }
        }
        
        Ok(variables)
    }

    /// Convert AST node to string representation
    fn ast_to_string(&self, expr: &ast::Expr) -> String {
        match expr {
            ast::Expr::Name(name) => name.id.to_string(),
            ast::Expr::Constant(constant) => {
                match &constant.value {
                    ast::Constant::Str(s) => format!("\"{}\"", s),
                    ast::Constant::Int(i) => i.to_string(),
                    ast::Constant::Float(f) => f.to_string(),
                    ast::Constant::Bool(b) => b.to_string(),
                    ast::Constant::None => "None".to_string(),
                    _ => "...".to_string(),
                }
            }
            ast::Expr::Attribute(attr) => {
                format!("{}.{}", self.ast_to_string(&attr.value), attr.attr)
            }
            ast::Expr::Subscript(subscript) => {
                format!("{}[{}]", self.ast_to_string(&subscript.value), self.ast_to_string(&subscript.slice))
            }
            _ => "...".to_string(),
        }
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
def hello(name: str) -> str:
    """Say hello to someone."""
    return f"Hello {name}"
"#;

        let result = parser.parse(content).await.unwrap();
        assert_eq!(result.elements.len(), 1);
        
        let func = &result.elements[0];
        assert_eq!(func.element_type, ElementType::Function);
        assert_eq!(func.name, Some("hello".to_string()));
        assert!(func.attributes.contains_key("parameters"));
        assert!(func.attributes.contains_key("return_type"));
        assert!(func.attributes.contains_key("docstring"));
    }

    #[tokio::test]
    async fn test_parse_class() {
        let parser = PythonParser::new();
        let content = r#"
class Person:
    """A person class."""
    
    def __init__(self, name: str):
        self.name = name
    
    def greet(self) -> str:
        return f"Hello, I'm {self.name}"
"#;

        let result = parser.parse(content).await.unwrap();
        assert_eq!(result.elements.len(), 1);
        
        let class = &result.elements[0];
        assert_eq!(class.element_type, ElementType::Class);
        assert_eq!(class.name, Some("Person".to_string()));
        assert_eq!(class.children.len(), 2); // __init__ and greet methods
        assert!(class.attributes.contains_key("docstring"));
    }

    #[tokio::test]
    async fn test_parse_imports() {
        let parser = PythonParser::new();
        let content = r#"
import os
import sys as system
from typing import List, Dict
from .local import helper as h
"#;

        let result = parser.parse(content).await.unwrap();
        assert_eq!(result.elements.len(), 5); // os, system, List, Dict, h
        
        let imports: Vec<_> = result.elements.iter()
            .filter(|e| e.element_type == ElementType::Import)
            .collect();
        assert_eq!(imports.len(), 5);
    }

    #[tokio::test]
    async fn test_parse_variables() {
        let parser = PythonParser::new();
        let content = r#"
VERSION = "1.0.0"
count: int = 42
name: str = "test"
"#;

        let result = parser.parse(content).await.unwrap();
        let variables: Vec<_> = result.elements.iter()
            .filter(|e| e.element_type == ElementType::Variable)
            .collect();
        assert_eq!(variables.len(), 3);
    }

    #[tokio::test]
    async fn test_async_function() {
        let parser = PythonParser::new();
        let content = r#"
async def fetch_data(url: str) -> dict:
    """Fetch data from URL."""
    return {}
"#;

        let result = parser.parse(content).await.unwrap();
        assert_eq!(result.elements.len(), 1);
        
        let func = &result.elements[0];
        assert_eq!(func.element_type, ElementType::Function);
        assert_eq!(func.name, Some("fetch_data".to_string()));
        assert_eq!(func.attributes.get("is_async").unwrap(), &json!(true));
    }

    #[tokio::test]
    async fn test_invalid_syntax() {
        let parser = PythonParser::new();
        let content = "def invalid_function(\n    # missing closing parenthesis";

        let result = parser.parse(content).await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_empty_content() {
        let parser = PythonParser::new();
        let content = "";

        let result = parser.parse(content).await.unwrap();
        assert_eq!(result.elements.len(), 0);
        assert_eq!(result.metadata.file_type, FileType::Python);
    }

    #[tokio::test]
    async fn test_byte_offset_to_line_number() {
        let parser = PythonParser::new();
        let content = "line 1\nline 2\nline 3";

        // Offset 0 is start of line 0
        assert_eq!(parser.byte_offset_to_line_number(content, 0), 0);
        // Offset in line 0
        assert_eq!(parser.byte_offset_to_line_number(content, 3), 0);
        // Offset at newline character of line 0
        assert_eq!(parser.byte_offset_to_line_number(content, 6), 0);
        // Offset after newline, start of line 1
        assert_eq!(parser.byte_offset_to_line_number(content, 7), 1);
        // Offset in line 1
        assert_eq!(parser.byte_offset_to_line_number(content, 10), 1);
        // Offset at newline character of line 1
        assert_eq!(parser.byte_offset_to_line_number(content, 13), 1);
        // Offset after newline, start of line 2
        assert_eq!(parser.byte_offset_to_line_number(content, 14), 2);
        // Offset at end of content
        assert_eq!(parser.byte_offset_to_line_number(content, content.len()), 2);

        let content_with_crlf = "line 1\r\nline 2\r\nline 3";
        assert_eq!(parser.byte_offset_to_line_number(content_with_crlf, 0), 0);
        assert_eq!(parser.byte_offset_to_line_number(content_with_crlf, 8), 1); // After "line 1\r\n"
        assert_eq!(parser.byte_offset_to_line_number(content_with_crlf, 16), 2); // After "line 1\r\nline 2\r\n"
    }
}