// AST traversal and element extraction for Python parser
// Contains methods for extracting functions, classes, imports, and variables from Python AST

use crate::error::VecqResult;
use crate::types::{DocumentElement, ElementAttributes, ElementType, PythonAttributes};
use rustpython_parser::ast;
use serde_json::json;
use std::collections::HashMap;

impl super::PythonParser {
    /// Extract function definitions from Python AST
    pub fn extract_functions(
        &self,
        content: &str,
        body: &[ast::Stmt],
    ) -> VecqResult<Vec<DocumentElement>> {
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
                let decorators: Vec<String> = func_def
                    .decorator_list
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
                    self.byte_offset_to_line_number(
                        content,
                        func_def.range.start().to_u32() as usize,
                    ),
                    self.byte_offset_to_line_number(
                        content,
                        func_def.range.end().to_u32() as usize,
                    ),
                )
                .set_attributes(ElementAttributes::Python(PythonAttributes {
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
                    attributes.insert(
                        "return_type".to_string(),
                        json!(self.ast_to_string(returns)),
                    );
                }

                // Extract decorators
                let decorators: Vec<String> = async_func_def
                    .decorator_list
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
                    self.byte_offset_to_line_number(
                        content,
                        async_func_def.range.start().to_u32() as usize,
                    ),
                    self.byte_offset_to_line_number(
                        content,
                        async_func_def.range.end().to_u32() as usize,
                    ),
                )
                .set_attributes(ElementAttributes::Python(PythonAttributes {
                    is_async: true,
                    other: attributes,
                }));

                functions.push(element);
            }
        }

        Ok(functions)
    }

    /// Extract class definitions from Python AST
    pub fn extract_classes(
        &self,
        content: &str,
        body: &[ast::Stmt],
    ) -> VecqResult<Vec<DocumentElement>> {
        let mut classes = Vec::new();

        for stmt in body {
            if let ast::Stmt::ClassDef(class_def) = stmt {
                let mut attributes = HashMap::new();

                // Extract class name
                attributes.insert("name".to_string(), json!(class_def.name.to_string()));

                // Extract base classes
                let bases: Vec<String> = class_def
                    .bases
                    .iter()
                    .map(|b| self.ast_to_string(b))
                    .collect();
                attributes.insert("bases".to_string(), json!(bases));

                // Extract decorators
                let decorators: Vec<String> = class_def
                    .decorator_list
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
                    self.byte_offset_to_line_number(
                        content,
                        class_def.range.start().to_u32() as usize,
                    ),
                    self.byte_offset_to_line_number(
                        content,
                        class_def.range.end().to_u32() as usize,
                    ),
                )
                .set_attributes(ElementAttributes::Python(PythonAttributes {
                    is_async: false,
                    other: attributes,
                }))
                .with_children(methods);

                classes.push(element);
            }
        }

        Ok(classes)
    }

    /// Extract import statements from Python AST
    pub fn extract_imports(
        &self,
        content: &str,
        body: &[ast::Stmt],
    ) -> VecqResult<Vec<DocumentElement>> {
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
                            self.byte_offset_to_line_number(
                                content,
                                import_stmt.range.start().to_u32() as usize,
                            ),
                            self.byte_offset_to_line_number(
                                content,
                                import_stmt.range.end().to_u32() as usize,
                            ),
                        )
                        .set_attributes(ElementAttributes::Python(PythonAttributes {
                            is_async: false,
                            other: attributes,
                        }));

                        imports.push(element);
                    }
                }
                ast::Stmt::ImportFrom(import_from) => {
                    let module = import_from
                        .module
                        .as_ref()
                        .map(|m| m.as_str())
                        .unwrap_or("");

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
                            self.byte_offset_to_line_number(
                                content,
                                import_from.range.start().to_u32() as usize,
                            ),
                            self.byte_offset_to_line_number(
                                content,
                                import_from.range.end().to_u32() as usize,
                            ),
                        )
                        .set_attributes(ElementAttributes::Python(PythonAttributes {
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
    pub fn extract_variables(
        &self,
        content: &str,
        body: &[ast::Stmt],
    ) -> VecqResult<Vec<DocumentElement>> {
        let mut variables = Vec::new();

        for stmt in body {
            if let ast::Stmt::Assign(assign) = stmt {
                for target in &assign.targets {
                    if let ast::Expr::Name(name) = target {
                        let mut attributes = HashMap::new();
                        attributes.insert("name".to_string(), json!(name.id.to_string()));
                        attributes.insert(
                            "value".to_string(),
                            json!(self.ast_to_string(&assign.value)),
                        );

                        let element = DocumentElement::new(
                            ElementType::Variable,
                            Some(name.id.to_string()),
                            format!("{} = ...", name.id),
                            self.byte_offset_to_line_number(
                                content,
                                assign.range.start().to_u32() as usize,
                            ),
                            self.byte_offset_to_line_number(
                                content,
                                assign.range.end().to_u32() as usize,
                            ),
                        )
                        .set_attributes(ElementAttributes::Python(PythonAttributes {
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
                    attributes.insert(
                        "type".to_string(),
                        json!(self.ast_to_string(&ann_assign.annotation)),
                    );
                    if let Some(value) = &ann_assign.value {
                        attributes.insert("value".to_string(), json!(self.ast_to_string(value)));
                    }

                    let element = DocumentElement::new(
                        ElementType::Variable,
                        Some(name.id.to_string()),
                        format!(
                            "{}: {} = ...",
                            name.id,
                            self.ast_to_string(&ann_assign.annotation)
                        ),
                        self.byte_offset_to_line_number(
                            content,
                            ann_assign.range.start().to_u32() as usize,
                        ),
                        self.byte_offset_to_line_number(
                            content,
                            ann_assign.range.end().to_u32() as usize,
                        ),
                    )
                    .set_attributes(ElementAttributes::Python(
                        PythonAttributes {
                            is_async: false,
                            other: attributes,
                        },
                    ));

                    variables.push(element);
                }
            }
        }

        Ok(variables)
    }

    /// Convert AST expression to string representation
    pub fn ast_to_string(&self, expr: &ast::Expr) -> String {
        match expr {
            ast::Expr::Name(name) => name.id.to_string(),
            ast::Expr::Constant(constant) => match &constant.value {
                ast::Constant::Str(s) => format!("\"{}\"", s),
                ast::Constant::Int(i) => i.to_string(),
                ast::Constant::Float(f) => f.to_string(),
                ast::Constant::Bool(b) => b.to_string(),
                ast::Constant::None => "None".to_string(),
                _ => "...".to_string(),
            },
            ast::Expr::Attribute(attr) => {
                format!("{}.{}", self.ast_to_string(&attr.value), attr.attr)
            }
            ast::Expr::Subscript(subscript) => {
                format!(
                    "{}[{}]",
                    self.ast_to_string(&subscript.value),
                    self.ast_to_string(&subscript.slice)
                )
            }
            _ => "...".to_string(),
        }
    }
}
