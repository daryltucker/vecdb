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
use crate::types::{DocumentElement, DocumentMetadata, ElementType, ParsedDocument, JavaScriptAttributes, ElementAttributes, UsageAttributes};
use async_trait::async_trait;
use std::collections::HashMap;
use std::path::PathBuf;

// Constants to avoid Rust 2021 raw string prefix conflicts
const JS_LANGUAGE_ERROR: &str = "Failed to set JavaScript language";
const JS_PARSE_ERROR: &str = "Failed to parse JavaScript content";
const STRING_PLACEHOLDER: &str = "<string>";
const JS_LANGUAGE_NAME: &str = "JavaScript";
const JS_FILE_EXTENSIONS: &[&str] = &["js", "mjs", "cjs"];

/// JavaScript parser using tree-sitter
#[derive(Debug, Clone)]
pub struct JavaScriptParser {
    enable_usages: bool,
    current_scope: String,
    _config: ParserConfig,
}

impl JavaScriptParser {
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

    pub fn with_config(config: ParserConfig) -> Self {
        Self {
            enable_usages: false,
            current_scope: "global".to_string(),
            _config: config,
        }
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
        JS_FILE_EXTENSIONS
    }

    fn language_name(&self) -> &str {
        JS_LANGUAGE_NAME
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
                file: PathBuf::from(STRING_PLACEHOLDER),
                line: 0,
                message: format!("{}: {}", JS_LANGUAGE_ERROR, e),
                source: None,
            })?;

        let tree = parser.parse(content, None)
            .ok_or_else(|| VecqError::ParseError {
                file: PathBuf::from(STRING_PLACEHOLDER),
                line: 0,
                message: JS_PARSE_ERROR.to_string(),
                source: None,
            })?;

        let functions = self.extract_functions(content, &tree)?;
        let classes = self.extract_classes(content, &tree)?;
        let imports = self.extract_imports(content, &tree)?;

        let mut elements = Vec::new();
        elements.extend(functions);
        elements.extend(classes);
        elements.extend(imports);

        // Extract usage/reference elements if enabled
        if self.enable_usages {
            elements.extend(self.detect_usages(content, &tree, None, &self.current_scope)?);
        }

        let mut doc = ParsedDocument::new(DocumentMetadata::new(PathBuf::new(), 0));
        doc.elements = elements;

        Ok(doc)
    }
}

impl JavaScriptParser {
    /// Extract usage/reference elements from JavaScript AST
    fn detect_usages(
        &self,
        content: &str,
        tree: &tree_sitter::Tree,
        current_function: Option<&str>,
        current_scope: &str,
    ) -> VecqResult<Vec<DocumentElement>> {
        let mut usages = Vec::new();
        let root = tree.root_node();
        let mut cursor = root.walk();

        for child in root.children(&mut cursor) {
            match child.kind() {
                "function_declaration" | "function" | "arrow_function" | "method_definition" => {
                    let func_name = self.extract_function_name(content, child).unwrap_or("anonymous");

                    let new_scope = format!("function:{}", func_name);
                    usages.extend(self.detect_usages_in_node(content, child, Some(func_name), &new_scope)?);
                }
                "class_declaration" => {
                    let class_name = child
                        .child_by_field_name("name")
                        .and_then(|n| n.utf8_text(content.as_bytes()).ok())
                        .unwrap_or("");

                    let new_scope = format!("class:{}", class_name);
                    usages.extend(self.detect_usages_in_node(content, child, current_function, &new_scope)?);
                }
                "call_expression" => {
                    usages.extend(self.detect_call_expression(content, child, current_function, current_scope));
                }
                "member_expression" => {
                    // Check if this is part of a method call
                    if let Some(parent) = child.parent() {
                        if parent.kind() == "call_expression" {
                            // This is handled in call_expression
                            continue;
                        }
                    }
                    usages.extend(self.detect_member_expression(content, child, current_function, current_scope));
                }
                "variable_declaration" | "lexical_declaration" => {
                    usages.extend(self.detect_variable_declaration(content, child, current_function, current_scope));
                }
                "assignment_expression" | "assignment_pattern" => {
                    usages.extend(self.detect_assignment(content, child, current_function, current_scope));
                }
                "import_statement" => {
                    usages.extend(self.detect_import_usage(content, child, current_function, current_scope));
                }
                _ => {
                    // Recursively check child nodes
                    usages.extend(self.detect_usages_in_node(content, child, current_function, current_scope)?);
                }
            }
        }

        Ok(usages)
    }

    /// Detect usages within a specific AST node
    fn detect_usages_in_node(
        &self,
        content: &str,
        node: tree_sitter::Node,
        current_function: Option<&str>,
        current_scope: &str,
    ) -> VecqResult<Vec<DocumentElement>> {
        let mut usages = Vec::new();
        let mut cursor = node.walk();

        for child in node.children(&mut cursor) {
            match child.kind() {
                "call_expression" => {
                    usages.extend(self.detect_call_expression(content, child, current_function, current_scope));
                }
                "member_expression" => {
                    if let Some(parent) = child.parent() {
                        if parent.kind() == "call_expression" {
                            continue;
                        }
                    }
                    usages.extend(self.detect_member_expression(content, child, current_function, current_scope));
                }
                "identifier" => {
                    usages.extend(self.detect_identifier_usage(content, child, current_function, current_scope));
                }
                _ => {
                    usages.extend(self.detect_usages_in_node(content, child, current_function, current_scope)?);
                }
            }
        }

        Ok(usages)
    }

    /// Extract function name from various function node types
    fn extract_function_name<'a>(&self, content: &'a str, node: tree_sitter::Node) -> Option<&'a str> {
        match node.kind() {
            "function_declaration" => node
                .child_by_field_name("name")
                .and_then(|n| n.utf8_text(content.as_bytes()).ok()),
            "method_definition" => node
                .child_by_field_name("name")
                .and_then(|n| n.utf8_text(content.as_bytes()).ok()),
            _ => None,
        }
    }

    /// Detect function/method calls
    fn detect_call_expression(
        &self,
        content: &str,
        node: tree_sitter::Node,
        current_function: Option<&str>,
        current_scope: &str,
    ) -> Vec<DocumentElement> {
        let mut usages = Vec::new();

        if let Some(function_node) = node.child_by_field_name("function") {
            let symbol_name = function_node
                .utf8_text(content.as_bytes())
                .unwrap_or("")
                .to_string();

            // Check if this is a method call (member_expression)
            let is_method_call = function_node.kind() == "member_expression";

            let (element_type, usage_type) = if is_method_call {
                (ElementType::MethodCall, "method_call")
            } else {
                (ElementType::FunctionCall, "call")
            };

            let usage_attr = UsageAttributes {
                symbol_name: symbol_name.clone(),
                usage_type: usage_type.to_string(),
                context: current_function.unwrap_or("global").to_string(),
                scope: current_scope.to_string(),
                other: HashMap::new(),
            };

            let element = DocumentElement::new(
                element_type,
                Some(symbol_name.clone()),
                format!("{}()", symbol_name),
                node.start_position().row,
                node.end_position().row,
            )
            .set_attributes(ElementAttributes::Usage(usage_attr));

            usages.push(element);
        }

        usages
    }

    /// Detect property/field references
    fn detect_member_expression(
        &self,
        content: &str,
        node: tree_sitter::Node,
        current_function: Option<&str>,
        current_scope: &str,
    ) -> Vec<DocumentElement> {
        let mut usages = Vec::new();

        if let Some(property_node) = node.child_by_field_name("property") {
            let property_name = property_node
                .utf8_text(content.as_bytes())
                .unwrap_or("")
                .to_string();

            let usage_attr = UsageAttributes {
                symbol_name: property_name.clone(),
                usage_type: "reference".to_string(),
                context: current_function.unwrap_or("global").to_string(),
                scope: current_scope.to_string(),
                other: HashMap::new(),
            };

            let element = DocumentElement::new(
                ElementType::VariableReference,
                Some(property_name.clone()),
                property_name,
                node.start_position().row,
                node.end_position().row,
            )
            .set_attributes(ElementAttributes::Usage(usage_attr));

            usages.push(element);
        }

        usages
    }

    /// Detect identifier usages
    fn detect_identifier_usage(
        &self,
        content: &str,
        node: tree_sitter::Node,
        current_function: Option<&str>,
        current_scope: &str,
    ) -> Vec<DocumentElement> {
        let mut usages = Vec::new();

        let identifier = node
            .utf8_text(content.as_bytes())
            .unwrap_or("")
            .to_string();

        let usage_attr = UsageAttributes {
            symbol_name: identifier.clone(),
            usage_type: "reference".to_string(),
            context: current_function.unwrap_or("global").to_string(),
            scope: current_scope.to_string(),
            other: HashMap::new(),
        };

        let element = DocumentElement::new(
            ElementType::VariableReference,
            Some(identifier.clone()),
            identifier,
            node.start_position().row,
            node.end_position().row,
        )
        .set_attributes(ElementAttributes::Usage(usage_attr));

        usages.push(element);

        usages
    }

    /// Detect variable declarations/assignments
    fn detect_variable_declaration(
        &self,
        content: &str,
        node: tree_sitter::Node,
        current_function: Option<&str>,
        current_scope: &str,
    ) -> Vec<DocumentElement> {
        let mut usages = Vec::new();

        // Handle variable declarators
        let mut cursor = node.walk();
        for child in node.children(&mut cursor) {
            if child.kind() == "variable_declarator" {
                if let Some(name_node) = child.child_by_field_name("name") {
                    let var_name = name_node
                        .utf8_text(content.as_bytes())
                        .unwrap_or("")
                        .to_string();

                    let usage_attr = UsageAttributes {
                        symbol_name: var_name.clone(),
                        usage_type: "assignment".to_string(),
                        context: current_function.unwrap_or("global").to_string(),
                        scope: current_scope.to_string(),
                        other: HashMap::new(),
                    };

                    let element = DocumentElement::new(
                        ElementType::Assignment,
                        Some(var_name.clone()),
                        format!("{} = ...", var_name),
                        node.start_position().row,
                        node.end_position().row,
                    )
                    .set_attributes(ElementAttributes::Usage(usage_attr));

                    usages.push(element);
                }
            }
        }

        usages
    }

    /// Detect assignments
    fn detect_assignment(
        &self,
        content: &str,
        node: tree_sitter::Node,
        current_function: Option<&str>,
        current_scope: &str,
    ) -> Vec<DocumentElement> {
        let mut usages = Vec::new();

        // Handle left side of assignments
        if let Some(left_node) = node.child_by_field_name("left") {
            if left_node.kind() == "identifier" {
                let var_name = left_node
                    .utf8_text(content.as_bytes())
                    .unwrap_or("")
                    .to_string();

                let usage_attr = UsageAttributes {
                    symbol_name: var_name.clone(),
                    usage_type: "assignment".to_string(),
                    context: current_function.unwrap_or("global").to_string(),
                    scope: current_scope.to_string(),
                    other: HashMap::new(),
                };

                let element = DocumentElement::new(
                    ElementType::Assignment,
                    Some(var_name.clone()),
                    format!("{} = ...", var_name),
                    node.start_position().row,
                    node.end_position().row,
                )
                .set_attributes(ElementAttributes::Usage(usage_attr));

                usages.push(element);
            }
        }

        usages
    }

    /// Detect import usages
    fn detect_import_usage(
        &self,
        content: &str,
        node: tree_sitter::Node,
        current_function: Option<&str>,
        current_scope: &str,
    ) -> Vec<DocumentElement> {
        let mut usages = Vec::new();

        let mut cursor = node.walk();
        for child in node.children(&mut cursor) {
            if child.kind() == "import_clause" {
                // Handle named imports
                let mut clause_cursor = child.walk();
                for clause_child in child.children(&mut clause_cursor) {
                    if clause_child.kind() == "named_imports" {
                        let mut named_cursor = clause_child.walk();
                        for named_child in clause_child.children(&mut named_cursor) {
                            if named_child.kind() == "import_specifier" {
                                if let Some(name_node) = named_child.child_by_field_name("name") {
                                    let import_name = name_node
                                        .utf8_text(content.as_bytes())
                                        .unwrap_or("")
                                        .to_string();

                                    let usage_attr = UsageAttributes {
                                        symbol_name: import_name.clone(),
                                        usage_type: "import_usage".to_string(),
                                        context: current_function.unwrap_or("global").to_string(),
                                        scope: current_scope.to_string(),
                                        other: HashMap::new(),
                                    };

                                    let element = DocumentElement::new(
                                        ElementType::ImportUsage,
                                        Some(import_name.clone()),
                                        format!("import {}", import_name),
                                        node.start_position().row,
                                        node.end_position().row,
                                    )
                                    .set_attributes(ElementAttributes::Usage(usage_attr));

                                    usages.push(element);
                                }
                            }
                        }
                    }
                }
            }
        }

        usages
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
        let content = "";

        let result = parser.parse(content).await.unwrap();
        assert_eq!(result.elements.len(), 0);
    }

    #[tokio::test]
    async fn test_usage_detection_function_calls() {
        let parser = JavaScriptParser::new().with_usages(true);
        let content = r#"
function greet(name) {
    const result = "Hello " + name;
    console.log(result);
    return result.length;
}

function main() {
    greet("world");
}
"#;

        let result = parser.parse(content).await.unwrap();

        // Should detect function calls: console.log(), greet()
        let function_calls: Vec<_> = result
            .elements
            .iter()
            .filter(|e| e.element_type == ElementType::FunctionCall)
            .collect();
        assert!(function_calls.len() >= 2); // At least log and greet

        // Check function call names
        let function_names: Vec<String> = function_calls
            .iter()
            .filter_map(|e| e.name.clone())
            .collect();
        assert!(function_names.contains(&"log".to_string()) || function_names.contains(&"console.log".to_string()));
        assert!(function_names.contains(&"greet".to_string()));
    }

    #[tokio::test]
    async fn test_usage_detection_assignments() {
        let parser = JavaScriptParser::new().with_usages(true);
        let content = r#"
function main() {
    let x = 42;
    const y = 10;
    var z = x + y;
}
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
        let parser = JavaScriptParser::new().with_usages(true);
        let content = r#"
import { useState } from 'react';
import fs from 'fs';
import * as utils from './utils';
"#;

        let result = parser.parse(content).await.unwrap();

        let import_usages: Vec<_> = result
            .elements
            .iter()
            .filter(|e| e.element_type == ElementType::ImportUsage)
            .collect();
        // Should detect import usages: useState, fs, utils
        assert!(import_usages.len() >= 2); // At least some imports

        // Check that we have import names
        let import_names: Vec<String> = import_usages
            .iter()
            .filter_map(|e| e.name.clone())
            .collect();
        assert!(import_names.len() > 0);
    }
}
