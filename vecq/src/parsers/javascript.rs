// PURPOSE:
//   JavaScript parser implementation for vecq using tree-sitter-javascript.
//   Extracts JS AST elements (functions, classes, imports, arrow functions).
//
// RELATED FILES:
//   - src/parsers/mod.rs - Parser registry
//   - src/types.rs - DocumentElement, ElementType definitions
//   - docs/specs/PARSING_GUIDELINES.md - the structural contract a parser must meet

use crate::error::{VecqError, VecqResult};
use crate::parser::{Parser, ParserCapabilities, ParserConfig};
use crate::types::{
    DocumentElement, DocumentMetadata, ElementAttributes, ElementType, JavaScriptAttributes,
    ParsedDocument, UsageAttributes,
};
use async_trait::async_trait;
use std::collections::HashMap;
use std::path::PathBuf;

// Constants to avoid Rust 2021 raw string prefix conflicts
const JS_LANGUAGE_ERROR: &str = "Failed to set JavaScript language";
const JS_PARSE_ERROR: &str = "Failed to parse JavaScript content";
const STRING_PLACEHOLDER: &str = "<string>";
const JS_LANGUAGE_NAME: &str = "JavaScript";
const JS_FILE_EXTENSIONS: &[&str] = &["js", "mjs", "cjs", "jsx"];

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

    /// The walker: every top-level declaration that names something —
    /// functions (declared, generator, or a `const f = () => …` binding),
    /// classes with their methods as children, imports with their module — and
    /// the same declarations behind `export` / `export default`, which is most
    /// of modern JavaScript. An exported declaration's element is the whole
    /// `export …` statement (so editing it keeps the keyword) and carries
    /// `is_exported` / `is_public`. Everything else is completed by
    /// `ts_coverage` with its grammar kind.
    fn walk(&self, content: &str, tree: &tree_sitter::Tree) -> Vec<DocumentElement> {
        let root = tree.root_node();
        let mut out = Vec::new();
        let mut cursor = root.walk();
        for child in root.children(&mut cursor) {
            if child.kind() == "export_statement" {
                let inner = child
                    .child_by_field_name("declaration")
                    .or_else(|| child.child_by_field_name("value"));
                let is_default = {
                    let mut c = child.walk();
                    let found = child.children(&mut c).any(|k| k.kind() == "default");
                    found
                };
                if let Some(inner) = inner {
                    if let Some(mut e) = self.declaration(content, inner, child) {
                        e = e
                            .with_attribute("is_exported".to_string(), true)
                            .with_attribute("is_public".to_string(), true);
                        if is_default {
                            e = e.with_attribute("is_default_export".to_string(), true);
                        }
                        out.push(e);
                    }
                }
                continue;
            }
            if let Some(e) = self.declaration(content, child, child) {
                out.push(
                    e.with_attribute("is_exported".to_string(), false)
                        .with_attribute("is_public".to_string(), false),
                );
            }
        }
        out
    }

    /// `node` as a named element whose span is `outer` (the node itself, or
    /// the `export` statement around it). `None` for anything the walker does
    /// not name — coverage emits it.
    fn declaration(
        &self,
        content: &str,
        node: tree_sitter::Node,
        outer: tree_sitter::Node,
    ) -> Option<DocumentElement> {
        match node.kind() {
            "function_declaration"
            | "generator_function_declaration"
            | "function_expression"
            | "generator_function" => {
                let name = self.field_text(content, node, "name");
                // `export default function () {}` has no name; it is still the
                // module's default export, named as JavaScript names it.
                let name = name.or_else(|| {
                    (outer.kind() == "export_statement").then(|| "default".to_string())
                })?;
                Some(self.function(content, node, outer, name, false))
            }
            "class_declaration" | "class" => {
                let name = self.field_text(content, node, "name").or_else(|| {
                    (outer.kind() == "export_statement").then(|| "default".to_string())
                })?;
                Some(self.class(content, node, outer, name))
            }
            "lexical_declaration" | "variable_declaration" => {
                // Only a binding to a function is a function: `const f = () =>
                // …`. `const xs = a.map(y => y)` is a variable (coverage).
                let mut c = node.walk();
                let declarators: Vec<_> = node
                    .named_children(&mut c)
                    .filter(|k| k.kind() == "variable_declarator")
                    .collect();
                let [declarator] = declarators.as_slice() else {
                    return None;
                };
                let value = declarator.child_by_field_name("value")?;
                let is_arrow = value.kind() == "arrow_function";
                if !matches!(
                    value.kind(),
                    "arrow_function" | "function_expression" | "generator_function"
                ) {
                    return None;
                }
                let name = self.field_text(content, *declarator, "name")?;
                Some(self.function(content, value, outer, name, is_arrow))
            }
            "import_statement" => {
                let module = node
                    .child_by_field_name("source")
                    .map(|s| unquote(&content[s.byte_range()]).to_string());
                let mut e = DocumentElement::new(
                    ElementType::Import,
                    module.clone(),
                    content[outer.byte_range()].to_string(),
                    outer.start_position().row + 1,
                    outer.end_position().row + 1,
                )
                .with_attribute("import_type".to_string(), "import");
                if let Some(m) = module {
                    // `module` is the key `vecq refs` reads for every language.
                    e = e.with_attribute("module".to_string(), m);
                }
                Some(e)
            }
            _ => None,
        }
    }

    fn field_text(&self, content: &str, node: tree_sitter::Node, field: &str) -> Option<String> {
        node.child_by_field_name(field)
            .map(|n| content[n.byte_range()].to_string())
            .filter(|s| !s.is_empty())
    }

    /// The text from the element's start to its body: `async function f(a, b)`,
    /// `const add = (a, b) =>`, whitespace collapsed.
    fn signature(
        &self,
        content: &str,
        outer: tree_sitter::Node,
        func: tree_sitter::Node,
    ) -> String {
        let end = func
            .child_by_field_name("body")
            .map(|b| b.start_byte())
            .unwrap_or_else(|| func.end_byte());
        content[outer.start_byte()..end.max(outer.start_byte())]
            .split_whitespace()
            .collect::<Vec<_>>()
            .join(" ")
    }

    fn function(
        &self,
        content: &str,
        func: tree_sitter::Node,
        outer: tree_sitter::Node,
        name: String,
        is_arrow: bool,
    ) -> DocumentElement {
        let is_async = {
            let mut c = func.walk();
            let found = func.children(&mut c).any(|k| k.kind() == "async");
            found
        };
        let mut other = HashMap::new();
        other.insert(
            "signature".to_string(),
            serde_json::Value::String(self.signature(content, outer, func)),
        );
        DocumentElement::new(
            ElementType::Function,
            Some(name),
            content[outer.byte_range()].to_string(),
            outer.start_position().row + 1,
            outer.end_position().row + 1,
        )
        .set_attributes(ElementAttributes::JavaScript(JavaScriptAttributes {
            is_async,
            is_arrow,
            other,
        }))
    }

    /// A class, its methods as children (`constructor`, getters/setters,
    /// `static`, `#private`) — each addressable on its own, as a Rust impl's or
    /// a Python class's are.
    fn class(
        &self,
        content: &str,
        node: tree_sitter::Node,
        outer: tree_sitter::Node,
        name: String,
    ) -> DocumentElement {
        let mut methods = Vec::new();
        if let Some(body) = node.child_by_field_name("body") {
            let mut c = body.walk();
            for member in body.named_children(&mut c) {
                if member.kind() != "method_definition" {
                    continue;
                }
                let Some(method_name) = self.field_text(content, member, "name") else {
                    continue;
                };
                let mut m = self.function(content, member, member, method_name, false);
                let is_static = {
                    let mut mc = member.walk();
                    let found = member.children(&mut mc).any(|k| k.kind() == "static");
                    found
                };
                m = m.with_attribute("is_static".to_string(), is_static);
                methods.push(m);
            }
        }
        let mut other = HashMap::new();
        other.insert(
            "signature".to_string(),
            serde_json::Value::String(self.signature(content, outer, node)),
        );
        DocumentElement::new(
            ElementType::Class,
            Some(name),
            content[outer.byte_range()].to_string(),
            outer.start_position().row + 1,
            outer.end_position().row + 1,
        )
        .set_attributes(ElementAttributes::JavaScript(JavaScriptAttributes {
            is_async: false,
            is_arrow: false,
            other,
        }))
        .with_children(methods)
    }
}

/// A JS string literal's text without its quotes.
fn unquote(s: &str) -> &str {
    let s = s.trim();
    if s.len() >= 2 {
        let (first, last) = (s.as_bytes()[0], s.as_bytes()[s.len() - 1]);
        if first == last && matches!(first, b'"' | b'\'' | b'`') {
            return &s[1..s.len() - 1];
        }
    }
    s
}

impl Default for JavaScriptParser {
    fn default() -> Self {
        Self::new()
    }
}

/// Coverage spec: every kind tree-sitter-javascript can produce at top level or inside the
/// wrappers walked (`bodies`), classified. `ts_coverage::complete` emits whatever the
/// walker above did not; the completeness test fails on any grammar kind missing here.
pub(crate) const COVERAGE: crate::parsers::ts_coverage::Spec = {
    use crate::parsers::ts_coverage::Kind::{Comment, Unit};
    use crate::types::ElementType as E;
    crate::parsers::ts_coverage::Spec {
        bodies: &["program"],
        kinds: &[
            ("break_statement", Unit(E::Block)),
            ("class_declaration", Unit(E::Class)),
            ("comment", Comment),
            ("continue_statement", Unit(E::Block)),
            ("debugger_statement", Unit(E::Block)),
            ("do_statement", Unit(E::Block)),
            ("empty_statement", Unit(E::Block)),
            ("export_clause", Unit(E::Import)),
            ("export_statement", Unit(E::Block)),
            ("expression_statement", Unit(E::Block)),
            ("for_in_statement", Unit(E::Block)),
            ("for_statement", Unit(E::Block)),
            ("function_declaration", Unit(E::Function)),
            ("generator_function_declaration", Unit(E::Function)),
            ("hash_bang_line", Comment),
            ("html_comment", Comment),
            ("if_statement", Unit(E::Block)),
            ("import_statement", Unit(E::Import)),
            ("labeled_statement", Unit(E::Block)),
            ("lexical_declaration", Unit(E::Variable)),
            ("namespace_export", Unit(E::Import)),
            ("return_statement", Unit(E::Block)),
            ("statement_block", Unit(E::Block)),
            ("switch_statement", Unit(E::Block)),
            ("throw_statement", Unit(E::Block)),
            ("try_statement", Unit(E::Block)),
            ("using_declaration", Unit(E::Variable)),
            ("variable_declaration", Unit(E::Variable)),
            ("while_statement", Unit(E::Block)),
            ("with_statement", Unit(E::Block)),
        ],
    }
};

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
        parser
            .set_language(&tree_sitter_javascript::LANGUAGE.into())
            .map_err(|e| VecqError::ParseError {
                file: PathBuf::from(STRING_PLACEHOLDER),
                line: 0,
                message: format!("{}: {}", JS_LANGUAGE_ERROR, e),
                source: None,
            })?;

        let tree = parser
            .parse(content, None)
            .ok_or_else(|| VecqError::ParseError {
                file: PathBuf::from(STRING_PLACEHOLDER),
                line: 0,
                message: JS_PARSE_ERROR.to_string(),
                source: None,
            })?;

        let mut elements = self.walk(content, &tree);

        // Extract usage/reference elements if enabled
        if self.enable_usages {
            elements.extend(self.detect_usages(content, &tree, None, &self.current_scope)?);
        }

        let mut doc = ParsedDocument::new(
            DocumentMetadata::new(PathBuf::from(STRING_PLACEHOLDER), content.len() as u64)
                .with_line_count(content)
                .with_file_type(crate::types::FileType::JavaScript),
        );
        doc.metadata.syntax_errors = crate::parsers::ts_coverage::complete(
            &mut elements,
            tree.root_node(),
            content,
            &COVERAGE,
        );
        doc.module = Some(crate::parsers::ts_refs::attach(
            &mut elements,
            tree.root_node(),
            content,
            &crate::parsers::ts_refs::JAVASCRIPT,
        ));
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
                    let func_name = self
                        .extract_function_name(content, child)
                        .unwrap_or("anonymous");

                    let new_scope = format!("function:{}", func_name);
                    usages.extend(self.detect_usages_in_node(
                        content,
                        child,
                        Some(func_name),
                        &new_scope,
                    )?);
                }
                "class_declaration" => {
                    let class_name = child
                        .child_by_field_name("name")
                        .and_then(|n| n.utf8_text(content.as_bytes()).ok())
                        .unwrap_or("");

                    let new_scope = format!("class:{}", class_name);
                    usages.extend(self.detect_usages_in_node(
                        content,
                        child,
                        current_function,
                        &new_scope,
                    )?);
                }
                "call_expression" => {
                    usages.extend(self.detect_call_expression(
                        content,
                        child,
                        current_function,
                        current_scope,
                    ));
                }
                "member_expression" => {
                    // Check if this is part of a method call
                    if let Some(parent) = child.parent() {
                        if parent.kind() == "call_expression" {
                            // This is handled in call_expression
                            continue;
                        }
                    }
                    usages.extend(self.detect_member_expression(
                        content,
                        child,
                        current_function,
                        current_scope,
                    ));
                }
                "variable_declaration" | "lexical_declaration" => {
                    usages.extend(self.detect_variable_declaration(
                        content,
                        child,
                        current_function,
                        current_scope,
                    ));
                }
                "assignment_expression" | "assignment_pattern" => {
                    usages.extend(self.detect_assignment(
                        content,
                        child,
                        current_function,
                        current_scope,
                    ));
                }
                "import_statement" => {
                    usages.extend(self.detect_import_usage(
                        content,
                        child,
                        current_function,
                        current_scope,
                    ));
                }
                _ => {
                    // Recursively check child nodes
                    usages.extend(self.detect_usages_in_node(
                        content,
                        child,
                        current_function,
                        current_scope,
                    )?);
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
                    usages.extend(self.detect_call_expression(
                        content,
                        child,
                        current_function,
                        current_scope,
                    ));
                }
                "member_expression" => {
                    if let Some(parent) = child.parent() {
                        if parent.kind() == "call_expression" {
                            continue;
                        }
                    }
                    usages.extend(self.detect_member_expression(
                        content,
                        child,
                        current_function,
                        current_scope,
                    ));
                }
                "identifier" => {
                    usages.extend(self.detect_identifier_usage(
                        content,
                        child,
                        current_function,
                        current_scope,
                    ));
                }
                "variable_declaration" | "lexical_declaration" => {
                    usages.extend(self.detect_variable_declaration(
                        content,
                        child,
                        current_function,
                        current_scope,
                    ));
                }
                _ => {
                    usages.extend(self.detect_usages_in_node(
                        content,
                        child,
                        current_function,
                        current_scope,
                    )?);
                }
            }
        }

        Ok(usages)
    }

    /// Extract function name from various function node types
    fn extract_function_name<'a>(
        &self,
        content: &'a str,
        node: tree_sitter::Node,
    ) -> Option<&'a str> {
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
                node.start_position().row + 1,
                node.end_position().row + 1,
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
                node.start_position().row + 1,
                node.end_position().row + 1,
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

        let identifier = node.utf8_text(content.as_bytes()).unwrap_or("").to_string();

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
            node.start_position().row + 1,
            node.end_position().row + 1,
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
                        node.start_position().row + 1,
                        node.end_position().row + 1,
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
                    node.start_position().row + 1,
                    node.end_position().row + 1,
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
                                        node.start_position().row + 1,
                                        node.end_position().row + 1,
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

        let functions: Vec<_> = result
            .elements
            .iter()
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

        let functions: Vec<_> = result
            .elements
            .iter()
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

        let functions: Vec<_> = result
            .elements
            .iter()
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

        let classes: Vec<_> = result
            .elements
            .iter()
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

        let imports: Vec<_> = result
            .elements
            .iter()
            .filter(|e| e.element_type == ElementType::Import)
            .collect();

        assert_eq!(imports.len(), 2);
    }

    /// REGRESSION (2026-314, registering JavaScript): what modern JS is made
    /// of. Before, `export function` / `export default class` / `export const
    /// f = () => …` were anonymous blocks, a class's methods were not
    /// elements, and any declaration whose text contained `=>` was a
    /// "function" — `const xs = a.map(y => y)` included.
    #[tokio::test]
    async fn exports_methods_and_arrow_bindings() {
        let src = "import { a } from \"./a.js\";\n\
                   export function f() {}\n\
                   export const g = async () => 1;\n\
                   const xs = [1].map((y) => y);\n\
                   export default class C {\n  constructor() {}\n  static make() {}\n}\n\
                   export default function () {}\n";
        let doc = JavaScriptParser::new().parse(src).await.unwrap();
        let named = |t: ElementType| -> Vec<String> {
            doc.elements
                .iter()
                .filter(|e| e.element_type == t)
                .filter_map(|e| e.name.clone())
                .collect()
        };
        assert_eq!(named(ElementType::Function), ["f", "g", "default"]);
        assert_eq!(named(ElementType::Class), ["C"]);
        assert_eq!(
            named(ElementType::Variable),
            ["xs"],
            "a value, not a function"
        );

        let f = doc
            .elements
            .iter()
            .find(|e| e.name.as_deref() == Some("f"))
            .unwrap();
        assert_eq!(
            f.content, "export function f() {}",
            "the element is the export statement"
        );
        assert_eq!(
            f.attributes.get("is_exported"),
            Some(&serde_json::json!(true))
        );
        let g = doc
            .elements
            .iter()
            .find(|e| e.name.as_deref() == Some("g"))
            .unwrap();
        let ElementAttributes::JavaScript(ga) = &g.attributes else {
            panic!()
        };
        assert!(ga.is_arrow && ga.is_async);

        let class = doc
            .elements
            .iter()
            .find(|e| e.name.as_deref() == Some("C"))
            .unwrap();
        let methods: Vec<_> = class
            .children
            .iter()
            .filter_map(|m| m.name.as_deref())
            .collect();
        assert_eq!(methods, ["constructor", "make"]);
        assert_eq!(
            class.children[1].attributes.get("is_static"),
            Some(&serde_json::json!(true))
        );
        assert_eq!(
            class.attributes.get("is_default_export"),
            Some(&serde_json::json!(true))
        );

        let import = doc
            .elements
            .iter()
            .find(|e| e.element_type == ElementType::Import)
            .unwrap();
        assert_eq!(
            import.attributes.get("module"),
            Some(&serde_json::json!("./a.js"))
        );
        assert_eq!(doc.metadata.file_type, crate::types::FileType::JavaScript);
        crate::parsers::ownership::assert_reconstructs("js", &doc.elements, src, |_| true);
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

        // Should detect function/method calls: console.log(), greet()
        let calls: Vec<_> = result
            .elements
            .iter()
            .filter(|e| {
                e.element_type == ElementType::FunctionCall
                    || e.element_type == ElementType::MethodCall
            })
            .collect();
        assert!(calls.len() >= 2); // At least log and greet

        // Check call names
        let call_names: Vec<String> = calls.iter().filter_map(|e| e.name.clone()).collect();
        assert!(
            call_names.contains(&"log".to_string())
                || call_names.contains(&"console.log".to_string())
        );
        assert!(call_names.contains(&"greet".to_string()));
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
        let assignment_names: Vec<String> =
            assignments.iter().filter_map(|e| e.name.clone()).collect();
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
        assert!(!import_usages.is_empty()); // At least some imports

        // Check that we have import names
        let import_names: Vec<String> = import_usages
            .iter()
            .filter_map(|e| e.name.clone())
            .collect();
        assert!(!import_names.is_empty());
    }
}
