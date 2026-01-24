// PURPOSE:
//   Go parser implementation for vecq using tree-sitter-go.
//   Extracts Go AST elements (functions, structs, interfaces, imports).
//
// RELATED FILES:
//   - src/parsers/c.rs - Reference implementation pattern
//   - src/types.rs - DocumentElement, ElementType definitions

use crate::error::{VecqError, VecqResult};
use crate::parser::{Parser, ParserCapabilities, ParserConfig};
use crate::types::{DocumentElement, DocumentMetadata, ElementType, FileType, ParsedDocument, GoAttributes, ElementAttributes, UsageAttributes};
use std::collections::HashMap;
use async_trait::async_trait;
use std::path::PathBuf;
use serde_json::json;

const PARSE_ERROR_MSG: &str = "Parse error";
const STRING_PLACEHOLDER: &str = "<string>";
const GO_LANGUAGE_ERROR: &str = "Failed to set Go language";
const GO_LANGUAGE_NAME: &str = "Go";
const GO_FILE_EXTENSION: &str = "go";
const SOURCE_FILE_NAME: &str = "source";

/// Go parser that extracts structural elements from Go source code
#[derive(Debug, Clone)]
pub struct GoParser {
    enable_usages: bool,
    current_scope: String,
    _config: ParserConfig,
}

impl GoParser {
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
        if let Some(ref rt) = receiver_type {
            attributes.insert("receiver".to_string(), json!(rt));
        }

        let element_type = if receiver_type.is_some() {
            ElementType::Function // Methods are still functions with receiver info
        } else {
            ElementType::Function
        };

        let element = DocumentElement::new(
            element_type,
            Some(name.clone()),
            format!("func {}(...)", name),
            node.start_position().row,
            node.end_position().row,
        ).set_attributes(ElementAttributes::Go(GoAttributes {
            other: attributes,
        }));

        Some(element)
    }

    fn parse_type_spec(&self, content: &str, node: tree_sitter::Node) -> Option<DocumentElement> {
        let mut name = String::new();
        let mut type_kind = "struct";

        // Extract name
        if let Some(name_node) = node.child_by_field_name("name") {
            name = name_node.utf8_text(content.as_bytes()).unwrap_or("").to_string();
        }

        // Determine type kind
        if let Some(type_node) = node.child_by_field_name("type") {
            type_kind = match type_node.kind() {
                "struct_type" => "struct",
                "interface_type" => "interface",
                _ => "type",
            };
        }

        if name.is_empty() { return None; }

        let mut attributes = HashMap::new();
        attributes.insert("type_kind".to_string(), json!(type_kind));

        let element_type = match type_kind {
            "interface" => ElementType::Interface,
            _ => ElementType::Struct,
        };

        let element = DocumentElement::new(
            element_type,
            Some(name.clone()),
            format!("type {} {} {{...}}", name, type_kind),
            node.start_position().row,
            node.end_position().row,
        ).set_attributes(ElementAttributes::Go(GoAttributes {
            other: attributes,
        }));

        Some(element)
    }

    fn parse_import_declaration(&self, content: &str, node: tree_sitter::Node) -> Vec<DocumentElement> {
        let mut imports = Vec::new();

        let mut cursor = node.walk();
        for child in node.children(&mut cursor) {
            match child.kind() {
                "import_spec" => {
                    let mut module = String::new();
                    let mut alias = None;

                    // Extract module path
                    if let Some(path_node) = child.child_by_field_name("path") {
                        module = path_node.utf8_text(content.as_bytes()).unwrap_or("").to_string();
                        // Remove quotes
                        module = module.trim_matches('"').to_string();
                    }

                    // Extract alias
                    if let Some(name_node) = child.child_by_field_name("name") {
                        alias = Some(name_node.utf8_text(content.as_bytes()).unwrap_or("").to_string());
                    }

                    if !module.is_empty() {
                        let mut attributes = HashMap::new();
                        attributes.insert("module".to_string(), json!(module));
                        if let Some(a) = &alias {
                            attributes.insert("alias".to_string(), json!(a));
                        }
                        attributes.insert("import_type".to_string(), json!("import"));

                        let import_name = alias.as_ref().unwrap_or(&module).to_string();

                        let element = DocumentElement::new(
                            ElementType::Import,
                            Some(import_name.clone()),
                            format!("import {}", module),
                            node.start_position().row,
                            node.end_position().row,
                        ).set_attributes(ElementAttributes::Go(GoAttributes {
                            other: attributes,
                        }));

                        imports.push(element);
                    }
                }
                "import_spec_list" => {
                    // Handle grouped imports
                    let mut list_cursor = child.walk();
                    for spec in child.children(&mut list_cursor) {
                        if spec.kind() == "import_spec" {
                            let mut module = String::new();
                            let mut alias = None;

                            // Extract module path
                            if let Some(path_node) = spec.child_by_field_name("path") {
                                module = path_node.utf8_text(content.as_bytes()).unwrap_or("").to_string();
                                // Remove quotes
                                module = module.trim_matches('"').to_string();
                            }

                            // Extract alias
                            if let Some(name_node) = spec.child_by_field_name("name") {
                                alias = Some(name_node.utf8_text(content.as_bytes()).unwrap_or("").to_string());
                            }

                            if !module.is_empty() {
                                let mut attributes = HashMap::new();
                                attributes.insert("module".to_string(), json!(module));
                                if let Some(a) = &alias {
                                    attributes.insert("alias".to_string(), json!(a));
                                }
                                attributes.insert("import_type".to_string(), json!("import"));

                                let import_name = alias.as_ref().unwrap_or(&module).to_string();

                                let element = DocumentElement::new(
                                    ElementType::Import,
                                    Some(import_name.clone()),
                                    format!("import {}", module),
                                    node.start_position().row,
                                    node.end_position().row,
                                ).set_attributes(ElementAttributes::Go(GoAttributes {
                                    other: attributes,
                                }));

                                imports.push(element);
                            }
                        }
                    }
                }
                _ => {}
            }
        }

        imports
    }

    fn link_methods(&self, elements: Vec<DocumentElement>) -> Vec<DocumentElement> {
        // For now, just return elements as-is
        // In a full implementation, this would link methods to their receiver types
        elements
    }

    /// Extract usage/reference elements from Go AST
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
                "function_declaration" | "method_declaration" => {
                    let func_name = child
                        .child_by_field_name("name")
                        .and_then(|n| n.utf8_text(content.as_bytes()).ok())
                        .unwrap_or("");

                    let new_scope = format!("function:{}", func_name);
                    usages.extend(self.detect_usages_in_node(content, child, Some(func_name), &new_scope)?);
                }
                "call_expression" => {
                    usages.extend(self.detect_call_expression(content, child, current_function, current_scope));
                }
                "selector_expression" => {
                    // Check if this is part of a method call
                    if let Some(parent) = child.parent() {
                        if parent.kind() == "call_expression" {
                            // This is handled in call_expression
                            continue;
                        }
                    }
                    usages.extend(self.detect_selector_expression(content, child, current_function, current_scope));
                }
                "short_var_declaration" | "var_declaration" => {
                    usages.extend(self.detect_variable_declaration(content, child, current_function, current_scope));
                }
                "assignment_statement" => {
                    usages.extend(self.detect_assignment(content, child, current_function, current_scope));
                }
                "import_declaration" => {
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
                "selector_expression" => {
                    if let Some(parent) = child.parent() {
                        if parent.kind() == "call_expression" {
                            continue;
                        }
                    }
                    usages.extend(self.detect_selector_expression(content, child, current_function, current_scope));
                }
                "short_var_declaration" | "var_declaration" => {
                    usages.extend(self.detect_variable_declaration(content, child, current_function, current_scope));
                }
                "assignment_statement" => {
                    usages.extend(self.detect_assignment(content, child, current_function, current_scope));
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

            // Check if this is a method call (selector_expression)
            let is_method_call = function_node.kind() == "selector_expression";

            // For method calls, extract just the method name (not the full qualified name)
            let element_name = if is_method_call {
                if let Some(field_node) = function_node.child_by_field_name("field") {
                    field_node.utf8_text(content.as_bytes()).unwrap_or("").to_string()
                } else {
                    symbol_name.clone()
                }
            } else {
                symbol_name.clone()
            };

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
                Some(element_name.clone()),
                format!("{}()", element_name),
                node.start_position().row,
                node.end_position().row,
            )
            .set_attributes(ElementAttributes::Usage(usage_attr));

            usages.push(element);
        }

        usages
    }

    /// Detect variable/field references
    fn detect_selector_expression(
        &self,
        content: &str,
        node: tree_sitter::Node,
        current_function: Option<&str>,
        current_scope: &str,
    ) -> Vec<DocumentElement> {
        let mut usages = Vec::new();

        if let Some(field_node) = node.child_by_field_name("field") {
            let field_name = field_node
                .utf8_text(content.as_bytes())
                .unwrap_or("")
                .to_string();

            let usage_attr = UsageAttributes {
                symbol_name: field_name.clone(),
                usage_type: "reference".to_string(),
                context: current_function.unwrap_or("global").to_string(),
                scope: current_scope.to_string(),
                other: HashMap::new(),
            };

            let element = DocumentElement::new(
                ElementType::VariableReference,
                Some(field_name.clone()),
                field_name,
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

        // For short_var_declaration (x := 42, z := x + y)
        if node.kind() == "short_var_declaration" {
            if let Some(left_node) = node.child_by_field_name("left") {
                let mut cursor = left_node.walk();
                for child in left_node.children(&mut cursor) {
                    if child.kind() == "identifier" {
                        let var_name = child
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
                            format!("{} := ...", var_name),
                            node.start_position().row,
                            node.end_position().row,
                        )
                        .set_attributes(ElementAttributes::Usage(usage_attr));

                        usages.push(element);
                    }
                }
            }
        }
        // For var_declaration (var y = 10)
        else if node.kind() == "var_declaration" {
            // Handle var_spec inside var_declaration
            let mut cursor = node.walk();
            for child in node.children(&mut cursor) {
                if child.kind() == "var_spec" {
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
                            format!("var {} = ...", var_name),
                            node.start_position().row,
                            node.end_position().row,
                        )
                        .set_attributes(ElementAttributes::Usage(usage_attr));

                        usages.push(element);
                    }
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
            let mut cursor = left_node.walk();
            for child in left_node.children(&mut cursor) {
                if child.kind() == "identifier" {
                    let var_name = child
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
            if child.kind() == "import_spec" {
                let import_name = self.extract_import_name(content, child);
                if !import_name.is_empty() {
                    usages.push(self.create_import_usage_element(import_name, node.start_position().row, node.end_position().row, current_function, current_scope));
                }
            }
            else if child.kind() == "import_spec_list" {
                let mut list_cursor = child.walk();
                for spec in child.children(&mut list_cursor) {
                    if spec.kind() == "import_spec" {
                        let import_name = self.extract_import_name(content, spec);
                        if !import_name.is_empty() {
                            usages.push(self.create_import_usage_element(import_name, node.start_position().row, node.end_position().row, current_function, current_scope));
                        }
                    }
                }
            }
        }

        usages
    }

    fn extract_import_name(&self, content: &str, spec_node: tree_sitter::Node) -> String {
        if let Some(name_node) = spec_node.child_by_field_name("name") {
            // Aliased import like: json "encoding/json"
            name_node.utf8_text(content.as_bytes()).unwrap_or("").to_string()
        } else if let Some(path_node) = spec_node.child_by_field_name("path") {
            // Unaliased import like: "fmt" - extract the package name from the path
            let path = path_node.utf8_text(content.as_bytes()).unwrap_or("");
            // Remove quotes and take the last component
            path.trim_matches('"').split('/').last().unwrap_or("").to_string()
        } else {
            String::new()
        }
    }

    fn create_import_usage_element(
        &self,
        import_name: String,
        start_row: usize,
        end_row: usize,
        current_function: Option<&str>,
        current_scope: &str,
    ) -> DocumentElement {
        let usage_attr = UsageAttributes {
            symbol_name: import_name.clone(),
            usage_type: "import_usage".to_string(),
            context: current_function.unwrap_or("global").to_string(),
            scope: current_scope.to_string(),
            other: HashMap::new(),
        };

        DocumentElement::new(
            ElementType::ImportUsage,
            Some(import_name.clone()),
            format!("import {}", import_name),
            start_row,
            end_row,
        )
        .set_attributes(ElementAttributes::Usage(usage_attr))
    }

    async fn parse(&self, content: &str) -> VecqResult<ParsedDocument> {
        let mut parser = tree_sitter::Parser::new();
        parser.set_language(&tree_sitter_go::LANGUAGE.into())
            .map_err(|e| VecqError::ParseError {
                file: PathBuf::from(STRING_PLACEHOLDER),
                line: 0,
                message: format!("{}: {}", GO_LANGUAGE_ERROR, e),
                source: None,
            })?;

        let tree = parser.parse(content, None)
            .ok_or_else(|| VecqError::ParseError {
                file: PathBuf::from(STRING_PLACEHOLDER),
                line: 0,
                message: PARSE_ERROR_MSG.to_string(),
                source: None,
            })?;

        let raw_elements = self.extract_raw_elements(content, &tree)?;
        let mut elements = self.link_methods(raw_elements);

        // Extract usage/reference elements if enabled
        if self.enable_usages {
            elements.extend(self.detect_usages(content, &tree, None, &self.current_scope)?);
        }

        let mut doc = ParsedDocument::new(
            DocumentMetadata::new(PathBuf::from(SOURCE_FILE_NAME), content.len() as u64)
                .with_line_count(content)
                .with_file_type(FileType::Go)
        );
        doc.elements = elements;

        Ok(doc)
    }
}

#[async_trait]
impl Parser for GoParser {
    fn file_extensions(&self) -> &[&str] {
        &[GO_FILE_EXTENSION]
    }

    fn language_name(&self) -> &str {
        GO_LANGUAGE_NAME
    }

    fn capabilities(&self) -> ParserCapabilities {
        ParserCapabilities {
            incremental: false,
            error_recovery: true,
            documentation: false,
            type_information: true,
            macros: false,
            max_file_size: None,
        }
    }

    async fn parse(&self, content: &str) -> VecqResult<ParsedDocument> {
        // Define error messages to avoid raw string prefix issues
        let go_lang_error = "Failed to set Go language";
        let parse_error = "Parse error";

        let mut parser = tree_sitter::Parser::new();
        parser.set_language(&tree_sitter_go::LANGUAGE.into())
            .map_err(|e| VecqError::ParseError {
                file: PathBuf::from(STRING_PLACEHOLDER),
                line: 0,
                message: format!("{}: {}", go_lang_error, e),
                source: None,
            })?;

        let tree = parser.parse(content, None)
            .ok_or_else(|| VecqError::ParseError {
                file: PathBuf::from(STRING_PLACEHOLDER),
                line: 0,
                message: parse_error.to_string(),
                source: None,
            })?;

        let raw_elements = self.extract_raw_elements(content, &tree)?;
        let mut elements = self.link_methods(raw_elements);

        // Extract usage/reference elements if enabled
        if self.enable_usages {
            elements.extend(self.detect_usages(content, &tree, None, &self.current_scope)?);
        }

        let mut doc = ParsedDocument::new(
            DocumentMetadata::new(PathBuf::from(SOURCE_FILE_NAME), content.len() as u64)
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

func greet(name string) string {
    return "Hello, " + name
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
    async fn test_parse_method() {
        let parser = GoParser::new();
        let content = r#"
package main

type User struct {
    name string
}

func (u User) GetName() string {
    return u.name
}
"#;
        let result = parser.parse(content).await.unwrap();

        let functions: Vec<_> = result.elements.iter()
            .filter(|e| e.element_type == ElementType::Function)
            .collect();

        assert_eq!(functions.len(), 1);
        assert_eq!(functions[0].name, Some("GetName".to_string()));
    }

    #[tokio::test]
    async fn test_parse_struct() {
        let parser = GoParser::new();
        let content = r#"
package main

type User struct {
    Name string
    Age  int
}
"#;
        let result = parser.parse(content).await.unwrap();

        let structs: Vec<_> = result.elements.iter()
            .filter(|e| e.element_type == ElementType::Struct)
            .collect();

        assert_eq!(structs.len(), 1);
        assert_eq!(structs[0].name, Some("User".to_string()));
    }

    #[tokio::test]
    async fn test_parse_imports() {
        let parser = GoParser::new();
        let content = r#"
package main

import (
    "fmt"
    "os"
)
"#;
        let result = parser.parse(content).await.unwrap();

        let imports: Vec<_> = result.elements.iter()
            .filter(|e| e.element_type == ElementType::Import)
            .collect();

        assert_eq!(imports.len(), 2);
    }

    #[tokio::test]
    async fn test_usage_detection_function_calls() {
        let parser = GoParser::new().with_usages(true);
        let content = r#"
package main

import "fmt"

func greet(name string) {
    fmt.Println("Hello", name)
}

func main() {
    greet("world")
}
"#;

        let result = parser.parse(content).await.unwrap();

        // Should detect function calls: greet()
        let function_calls: Vec<_> = result
            .elements
            .iter()
            .filter(|e| e.element_type == ElementType::FunctionCall)
            .collect();

        // Should detect method calls: fmt.Println()
        let method_calls: Vec<_> = result
            .elements
            .iter()
            .filter(|e| e.element_type == ElementType::MethodCall)
            .collect();

        assert!(function_calls.len() >= 1); // At least greet
        assert!(method_calls.len() >= 1); // At least Println

        // Check function call names
        let function_names: Vec<String> = function_calls
            .iter()
            .filter_map(|e| e.name.clone())
            .collect();
        assert!(function_names.contains(&"greet".to_string()));

        // Check method call names
        let method_names: Vec<String> = method_calls
            .iter()
            .filter_map(|e| e.name.clone())
            .collect();
        assert!(method_names.contains(&"Println".to_string()));
    }

    #[tokio::test]
    async fn test_usage_detection_assignments() {
        let parser = GoParser::new().with_usages(true);
        let content = r#"
package main

func main() {
    x := 42
    var y = 10
    z := x + y
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
        let parser = GoParser::new().with_usages(true);
        let content = r#"
package main

import (
    "fmt"
    json "encoding/json"
)

func main() {
    fmt.Println("Hello")
    json.Marshal(struct{}{})
}
"#;

        let result = parser.parse(content).await.unwrap();

        let import_usages: Vec<_> = result
            .elements
            .iter()
            .filter(|e| e.element_type == ElementType::ImportUsage)
            .collect();
        // Should detect import usages: fmt, json
        assert!(import_usages.len() >= 2);

        // Check that we have import names
        let import_names: Vec<String> = import_usages
            .iter()
            .filter_map(|e| e.name.clone())
            .collect();
        assert!(import_names.contains(&"fmt".to_string()));
        assert!(import_names.contains(&"json".to_string()));
    }

    #[tokio::test]
    async fn test_usage_detection_variable_references() {
        let parser = GoParser::new().with_usages(true);
        let content = r#"
package main

import "fmt"

func main() {
    x := 42
    y := x + 10
    fmt.Println(y)
}
"#;

        let result = parser.parse(content).await.unwrap();

        let references: Vec<_> = result
            .elements
            .iter()
            .filter(|e| e.element_type == ElementType::VariableReference)
            .collect();
        // Should detect variable references: x (in y := x + 10)
        assert!(references.len() >= 1);

        // Check reference names
        let reference_names: Vec<String> = references
            .iter()
            .filter_map(|e| e.name.clone())
            .collect();
        assert!(reference_names.contains(&"x".to_string()));
    }
}
