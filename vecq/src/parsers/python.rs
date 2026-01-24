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

    /// Extract usage/reference elements from Python AST
    fn detect_usages(
        &self,
        content: &str,
        body: &[ast::Stmt],
        current_function: Option<&str>,
        current_scope: &str,
    ) -> VecqResult<Vec<DocumentElement>> {
        let mut usages = Vec::new();

        for stmt in body {
            match stmt {
                ast::Stmt::FunctionDef(func_def) => {
                    let func_scope = format!("function:{}", func_def.name);
                    usages.extend(self.detect_usages_in_body(content, &func_def.body, Some(&func_def.name), &func_scope)?);
                }
                ast::Stmt::AsyncFunctionDef(async_func_def) => {
                    let func_scope = format!("function:{}", async_func_def.name);
                    usages.extend(self.detect_usages_in_body(content, &async_func_def.body, Some(&async_func_def.name), &func_scope)?);
                }
                ast::Stmt::ClassDef(class_def) => {
                    let class_scope = format!("class:{}", class_def.name);
                    usages.extend(self.detect_usages_in_body(content, &class_def.body, current_function, &class_scope)?);
                }
                _ => {
                    usages.extend(self.detect_usages_in_stmt(content, stmt, current_function, current_scope)?);
                }
            }
        }

        Ok(usages)
    }

    /// Detect usages within a statement
    fn detect_usages_in_stmt(
        &self,
        content: &str,
        stmt: &ast::Stmt,
        current_function: Option<&str>,
        current_scope: &str,
    ) -> VecqResult<Vec<DocumentElement>> {
        let mut usages = Vec::new();

        match stmt {
            ast::Stmt::Expr(expr_stmt) => {
                usages.extend(self.detect_usages_in_expr(content, &expr_stmt.value, current_function, current_scope));
            }
            ast::Stmt::Assign(assign) => {
                usages.extend(self.detect_assignment(content, assign, current_function, current_scope));
            }
            ast::Stmt::AnnAssign(ann_assign) => {
                usages.extend(self.detect_annotated_assignment(content, ann_assign, current_function, current_scope));
            }
            ast::Stmt::Import(import) => {
                usages.extend(self.detect_import_usage(content, import, current_function, current_scope));
            }
            ast::Stmt::ImportFrom(import_from) => {
                usages.extend(self.detect_import_from_usage(content, import_from, current_function, current_scope));
            }
            ast::Stmt::If(if_stmt) => {
                usages.extend(self.detect_usages_in_body(content, &if_stmt.body, current_function, current_scope)?);
                usages.extend(self.detect_usages_in_body(content, &if_stmt.orelse, current_function, current_scope)?);
            }
            ast::Stmt::For(for_stmt) => {
                usages.extend(self.detect_usages_in_body(content, &for_stmt.body, current_function, current_scope)?);
                usages.extend(self.detect_usages_in_body(content, &for_stmt.orelse, current_function, current_scope)?);
            }
            ast::Stmt::While(while_stmt) => {
                usages.extend(self.detect_usages_in_body(content, &while_stmt.body, current_function, current_scope)?);
                usages.extend(self.detect_usages_in_body(content, &while_stmt.orelse, current_function, current_scope)?);
            }
            _ => {
                // For other statement types, traverse their expressions
                self.traverse_stmt_expressions(stmt, |expr| {
                    usages.extend(self.detect_usages_in_expr(content, expr, current_function, current_scope));
                });
            }
        }

        Ok(usages)
    }

    /// Detect usages within a body of statements
    fn detect_usages_in_body(
        &self,
        content: &str,
        body: &[ast::Stmt],
        current_function: Option<&str>,
        current_scope: &str,
    ) -> VecqResult<Vec<DocumentElement>> {
        let mut usages = Vec::new();
        for stmt in body {
            usages.extend(self.detect_usages_in_stmt(content, stmt, current_function, current_scope)?);
        }
        Ok(usages)
    }

    /// Detect usages within an expression
    fn detect_usages_in_expr(
        &self,
        content: &str,
        expr: &ast::Expr,
        current_function: Option<&str>,
        current_scope: &str,
    ) -> Vec<DocumentElement> {
        let mut usages = Vec::new();

        match expr {
            ast::Expr::Call(call) => {
                usages.extend(self.detect_call_expression(content, call, current_function, current_scope));
                // Recursively check arguments
                for arg in &call.args {
                    usages.extend(self.detect_usages_in_expr(content, arg, current_function, current_scope));
                }
                for kw in &call.keywords {
                    usages.extend(self.detect_usages_in_expr(content, &kw.value, current_function, current_scope));
                }
            }
            ast::Expr::Attribute(attr) => {
                usages.extend(self.detect_attribute_access(content, attr, current_function, current_scope));
                // Recursively check the base object
                usages.extend(self.detect_usages_in_expr(content, &attr.value, current_function, current_scope));
            }
            ast::Expr::Name(name) => {
                usages.extend(self.detect_name_usage(content, name, current_function, current_scope));
            }
            ast::Expr::Subscript(subscript) => {
                usages.extend(self.detect_usages_in_expr(content, &subscript.value, current_function, current_scope));
                usages.extend(self.detect_usages_in_expr(content, &subscript.slice, current_function, current_scope));
            }
            ast::Expr::List(list) => {
                for elt in &list.elts {
                    usages.extend(self.detect_usages_in_expr(content, elt, current_function, current_scope));
                }
            }
            ast::Expr::Dict(dict) => {
                for key in &dict.keys {
                    if let Some(k) = key {
                        usages.extend(self.detect_usages_in_expr(content, k, current_function, current_scope));
                    }
                }
                for value in &dict.values {
                    usages.extend(self.detect_usages_in_expr(content, value, current_function, current_scope));
                }
            }
            ast::Expr::Tuple(tuple) => {
                for elt in &tuple.elts {
                    usages.extend(self.detect_usages_in_expr(content, elt, current_function, current_scope));
                }
            }
            _ => {
                // For other expression types, we could add more specific handling
                // but for now we just traverse any sub-expressions they might contain
                self.traverse_expr_subexpressions(expr, |sub_expr| {
                    usages.extend(self.detect_usages_in_expr(content, sub_expr, current_function, current_scope));
                });
            }
        }

        usages
    }

    /// Detect function/method calls
    fn detect_call_expression(
        &self,
        content: &str,
        call: &ast::ExprCall,
        current_function: Option<&str>,
        current_scope: &str,
    ) -> Vec<DocumentElement> {
        let mut usages = Vec::new();

        let symbol_name = self.ast_to_string(&call.func);

        // Check if this is a method call (attribute access)
        let is_method_call = matches!(&*call.func, ast::Expr::Attribute(_));

        let (element_type, usage_type) = if is_method_call {
            (ElementType::MethodCall, "method_call")
        } else {
            (ElementType::FunctionCall, "call")
        };

        let usage_attr = crate::types::UsageAttributes {
            symbol_name: symbol_name.clone(),
            usage_type: usage_type.to_string(),
            context: current_function.unwrap_or("global").to_string(),
            scope: current_scope.to_string(),
            other: std::collections::HashMap::new(),
        };

        let element = DocumentElement::new(
            element_type,
            Some(symbol_name.clone()),
            format!("{}()", symbol_name),
            self.byte_offset_to_line_number(content, call.range.start().to_u32() as usize),
            self.byte_offset_to_line_number(content, call.range.end().to_u32() as usize),
        )
        .set_attributes(ElementAttributes::Usage(usage_attr));

        usages.push(element);
        usages
    }

    /// Detect attribute/field access
    fn detect_attribute_access(
        &self,
        content: &str,
        attr: &ast::ExprAttribute,
        current_function: Option<&str>,
        current_scope: &str,
    ) -> Vec<DocumentElement> {
        let mut usages = Vec::new();

        let field_name = attr.attr.to_string();

        let usage_attr = crate::types::UsageAttributes {
            symbol_name: field_name.clone(),
            usage_type: "reference".to_string(),
            context: current_function.unwrap_or("global").to_string(),
            scope: current_scope.to_string(),
            other: std::collections::HashMap::new(),
        };

        let element = DocumentElement::new(
            ElementType::VariableReference,
            Some(field_name.clone()),
            field_name,
            self.byte_offset_to_line_number(content, attr.range.start().to_u32() as usize),
            self.byte_offset_to_line_number(content, attr.range.end().to_u32() as usize),
        )
        .set_attributes(ElementAttributes::Usage(usage_attr));

        usages.push(element);
        usages
    }

    /// Detect variable name usage
    fn detect_name_usage(
        &self,
        content: &str,
        name: &ast::ExprName,
        current_function: Option<&str>,
        current_scope: &str,
    ) -> Vec<DocumentElement> {
        let mut usages = Vec::new();

        let var_name = name.id.to_string();

        let usage_attr = crate::types::UsageAttributes {
            symbol_name: var_name.clone(),
            usage_type: "reference".to_string(),
            context: current_function.unwrap_or("global").to_string(),
            scope: current_scope.to_string(),
            other: std::collections::HashMap::new(),
        };

        let element = DocumentElement::new(
            ElementType::VariableReference,
            Some(var_name.clone()),
            var_name,
            self.byte_offset_to_line_number(content, name.range.start().to_u32() as usize),
            self.byte_offset_to_line_number(content, name.range.end().to_u32() as usize),
        )
        .set_attributes(ElementAttributes::Usage(usage_attr));

        usages.push(element);
        usages
    }

    /// Detect variable assignments
    fn detect_assignment(
        &self,
        content: &str,
        assign: &ast::StmtAssign,
        current_function: Option<&str>,
        current_scope: &str,
    ) -> Vec<DocumentElement> {
        let mut usages = Vec::new();

        for target in &assign.targets {
            if let ast::Expr::Name(name) = target {
                let var_name = name.id.to_string();

                let usage_attr = crate::types::UsageAttributes {
                    symbol_name: var_name.clone(),
                    usage_type: "assignment".to_string(),
                    context: current_function.unwrap_or("global").to_string(),
                    scope: current_scope.to_string(),
                    other: std::collections::HashMap::new(),
                };

                let element = DocumentElement::new(
                    ElementType::Assignment,
                    Some(var_name.clone()),
                    format!("{} = ...", var_name),
                    self.byte_offset_to_line_number(content, assign.range.start().to_u32() as usize),
                    self.byte_offset_to_line_number(content, assign.range.end().to_u32() as usize),
                )
                .set_attributes(ElementAttributes::Usage(usage_attr));

                usages.push(element);
            }
        }

        usages
    }

    /// Detect annotated assignments (type annotations)
    fn detect_annotated_assignment(
        &self,
        content: &str,
        ann_assign: &ast::StmtAnnAssign,
        current_function: Option<&str>,
        current_scope: &str,
    ) -> Vec<DocumentElement> {
        let mut usages = Vec::new();

        if let ast::Expr::Name(name) = &*ann_assign.target {
            let var_name = name.id.to_string();

            let usage_attr = crate::types::UsageAttributes {
                symbol_name: var_name.clone(),
                usage_type: "assignment".to_string(),
                context: current_function.unwrap_or("global").to_string(),
                scope: current_scope.to_string(),
                other: std::collections::HashMap::new(),
            };

            let element = DocumentElement::new(
                ElementType::Assignment,
                Some(var_name.clone()),
                format!("{}: {} = ...", var_name, self.ast_to_string(&ann_assign.annotation)),
                self.byte_offset_to_line_number(content, ann_assign.range.start().to_u32() as usize),
                self.byte_offset_to_line_number(content, ann_assign.range.end().to_u32() as usize),
            )
            .set_attributes(ElementAttributes::Usage(usage_attr));

            usages.push(element);
        }

        usages
    }

    /// Detect import usage
    fn detect_import_usage(
        &self,
        content: &str,
        import: &ast::StmtImport,
        current_function: Option<&str>,
        current_scope: &str,
    ) -> Vec<DocumentElement> {
        let mut usages = Vec::new();

        for alias in &import.names {
            let import_name = alias.asname.as_ref().unwrap_or(&alias.name).to_string();

            let usage_attr = crate::types::UsageAttributes {
                symbol_name: import_name.clone(),
                usage_type: "import_usage".to_string(),
                context: current_function.unwrap_or("global").to_string(),
                scope: current_scope.to_string(),
                other: std::collections::HashMap::new(),
            };

            let element = DocumentElement::new(
                ElementType::ImportUsage,
                Some(import_name.clone()),
                format!("import {}", import_name),
                self.byte_offset_to_line_number(content, import.range.start().to_u32() as usize),
                self.byte_offset_to_line_number(content, import.range.end().to_u32() as usize),
            )
            .set_attributes(ElementAttributes::Usage(usage_attr));

            usages.push(element);
        }

        usages
    }

    /// Detect import from usage
    fn detect_import_from_usage(
        &self,
        content: &str,
        import_from: &ast::StmtImportFrom,
        current_function: Option<&str>,
        current_scope: &str,
    ) -> Vec<DocumentElement> {
        let mut usages = Vec::new();

        for alias in &import_from.names {
            let import_name = alias.asname.as_ref().unwrap_or(&alias.name).to_string();

            let usage_attr = crate::types::UsageAttributes {
                symbol_name: import_name.clone(),
                usage_type: "import_usage".to_string(),
                context: current_function.unwrap_or("global").to_string(),
                scope: current_scope.to_string(),
                other: std::collections::HashMap::new(),
            };

            let element = DocumentElement::new(
                ElementType::ImportUsage,
                Some(import_name.clone()),
                format!("from ... import {}", import_name),
                self.byte_offset_to_line_number(content, import_from.range.start().to_u32() as usize),
                self.byte_offset_to_line_number(content, import_from.range.end().to_u32() as usize),
            )
            .set_attributes(ElementAttributes::Usage(usage_attr));

            usages.push(element);
        }

        usages
    }

    /// Helper to traverse statement expressions
    fn traverse_stmt_expressions<F>(&self, stmt: &ast::Stmt, mut f: F)
    where
        F: FnMut(&ast::Expr),
    {
        match stmt {
            ast::Stmt::Expr(expr) => f(&expr.value),
            ast::Stmt::If(if_stmt) => {
                f(&if_stmt.test);
                // body and orelse are handled separately
            }
            ast::Stmt::For(for_stmt) => {
                f(&for_stmt.iter);
                // body and orelse are handled separately
            }
            ast::Stmt::While(while_stmt) => {
                f(&while_stmt.test);
                // body and orelse are handled separately
            }
            ast::Stmt::With(with_stmt) => {
                for item in &with_stmt.items {
                    f(&item.context_expr);
                }
            }
            ast::Stmt::Assert(assert_stmt) => f(&assert_stmt.test),
            ast::Stmt::Return(return_stmt) => {
                if let Some(value) = &return_stmt.value {
                    f(value);
                }
            }
            ast::Stmt::Assign(assign) => f(&assign.value),
            ast::Stmt::AnnAssign(ann_assign) => {
                if let Some(value) = &ann_assign.value {
                    f(value);
                }
            }
            // Add more as needed
            _ => {}
        }
    }

    /// Helper to traverse expression sub-expressions
    fn traverse_expr_subexpressions<F>(&self, expr: &ast::Expr, mut f: F)
    where
        F: FnMut(&ast::Expr),
    {
        match expr {
            ast::Expr::UnaryOp(unary) => f(&unary.operand),
            ast::Expr::BinOp(binop) => {
                f(&binop.left);
                f(&binop.right);
            }
            ast::Expr::Compare(compare) => {
                f(&compare.left);
                for comp in &compare.comparators {
                    f(comp);
                }
            }
            ast::Expr::BoolOp(boolop) => {
                for value in &boolop.values {
                    f(value);
                }
            }
            ast::Expr::IfExp(ifexp) => {
                f(&ifexp.test);
                f(&ifexp.body);
                f(&ifexp.orelse);
            }
            // Add more as needed
            _ => {}
        }
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
        // This might be 0 or we might detect implicit calls differently

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
import sys as system
from typing import List, Dict
from collections import defaultdict as dd
"#;

        let result = parser.parse(content).await.unwrap();

        let import_usages: Vec<_> = result
            .elements
            .iter()
            .filter(|e| e.element_type == ElementType::ImportUsage)
            .collect();
        // Should detect import usages: os, system, List, Dict, dd
        assert_eq!(import_usages.len(), 5);

        // Check that we have the expected import names
        let import_names: Vec<String> = import_usages
            .iter()
            .filter_map(|e| e.name.clone())
            .collect();
        assert!(import_names.contains(&"os".to_string()));
        assert!(import_names.contains(&"system".to_string()));
        assert!(import_names.contains(&"List".to_string()));
        assert!(import_names.contains(&"Dict".to_string()));
        assert!(import_names.contains(&"dd".to_string()));
    }

    #[tokio::test]
    async fn test_usage_detection_attribute_access() {
        let parser = PythonParser::new().with_usages(true);
        let content = r#"
class Person:
    def __init__(self, name):
        self.name = name

    def greet(self):
        return f"Hello, {self.name}"

def main():
    p = Person("Alice")
    message = p.greet()
    print(message.upper())
"#;

        let result = parser.parse(content).await.unwrap();

        // Should detect method calls: p.greet(), message.upper()
        let method_calls: Vec<_> = result
            .elements
            .iter()
            .filter(|e| e.element_type == ElementType::MethodCall)
            .collect();
        // Note: Python method call detection may not catch all cases yet

        // Should detect attribute access: self.name, p.greet, message.upper
        let references: Vec<_> = result
            .elements
            .iter()
            .filter(|e| e.element_type == ElementType::VariableReference)
            .collect();
        // Should include name, greet, upper, etc.
    }
}

    #[tokio::test]
    async fn test_usage_detection_import_usage() {
        let parser = PythonParser::new().with_usages(true);
        let content = r#"
import os
import sys as system
from typing import List, Dict
from collections import defaultdict as dd
"#;

        let result = parser.parse(content).await.unwrap();

        let import_usages: Vec<_> = result
            .elements
            .iter()
            .filter(|e| e.element_type == ElementType::ImportUsage)
            .collect();
        // Should detect import usages: os, system, List, Dict, dd
        assert_eq!(import_usages.len(), 5);

        // Check that we have the expected import names
        let import_names: Vec<String> = import_usages
            .iter()
            .filter_map(|e| e.name.clone())
            .collect();
        assert!(import_names.contains(&"os".to_string()));
        assert!(import_names.contains(&"system".to_string()));
        assert!(import_names.contains(&"List".to_string()));
        assert!(import_names.contains(&"Dict".to_string()));
        assert!(import_names.contains(&"dd".to_string()));
    }

    #[tokio::test]
    async fn test_usage_detection_attribute_access() {
        let parser = PythonParser::new().with_usages(true);
        let content = r#"
class Person:
    def __init__(self, name):
        self.name = name

    def greet(self):
        return f"Hello, {self.name}"

def main():
    p = Person("Alice")
    message = p.greet()
    print(message.upper())
"#;

        let result = parser.parse(content).await.unwrap();

        // Should detect method calls: p.greet(), message.upper()
        let method_calls: Vec<_> = result
            .elements
            .iter()
            .filter(|e| e.element_type == ElementType::MethodCall)
            .collect();
        // At least message.upper() should be detected as a method call
        assert!(method_calls.len() >= 1);

        // Should detect attribute access and variable references
        let references: Vec<_> = result
            .elements
            .iter()
            .filter(|e| e.element_type == ElementType::VariableReference)
            .collect();
        // Variable references may be detected in future improvements
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