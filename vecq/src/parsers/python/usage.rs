// Usage detection and analysis for Python parser
// Contains methods for detecting function calls, variable references, assignments, and imports

use crate::error::VecqResult;
use crate::types::{DocumentElement, ElementAttributes, ElementType, UsageAttributes};
use rustpython_parser::ast;
use std::collections::HashMap;

impl super::PythonParser {
    /// Extract usage/reference elements from Python AST
    pub fn detect_usages(
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
                    usages.extend(self.detect_usages_in_body(
                        content,
                        &func_def.body,
                        Some(&func_def.name),
                        &func_scope,
                    )?);
                }
                ast::Stmt::AsyncFunctionDef(async_func_def) => {
                    let func_scope = format!("function:{}", async_func_def.name);
                    usages.extend(self.detect_usages_in_body(
                        content,
                        &async_func_def.body,
                        Some(&async_func_def.name),
                        &func_scope,
                    )?);
                }
                ast::Stmt::ClassDef(class_def) => {
                    let class_scope = format!("class:{}", class_def.name);
                    usages.extend(self.detect_usages_in_body(
                        content,
                        &class_def.body,
                        current_function,
                        &class_scope,
                    )?);
                }
                _ => {
                    usages.extend(self.detect_usages_in_stmt(
                        content,
                        stmt,
                        current_function,
                        current_scope,
                    )?);
                }
            }
        }

        Ok(usages)
    }

    /// Detect usages within a statement
    pub fn detect_usages_in_stmt(
        &self,
        content: &str,
        stmt: &ast::Stmt,
        current_function: Option<&str>,
        current_scope: &str,
    ) -> VecqResult<Vec<DocumentElement>> {
        let mut usages = Vec::new();

        match stmt {
            ast::Stmt::Expr(expr_stmt) => {
                usages.extend(self.detect_usages_in_expr(
                    content,
                    &expr_stmt.value,
                    current_function,
                    current_scope,
                ));
            }
            ast::Stmt::Assign(assign) => {
                usages.extend(self.detect_assignment(
                    content,
                    assign,
                    current_function,
                    current_scope,
                ));
            }
            ast::Stmt::AnnAssign(ann_assign) => {
                usages.extend(self.detect_annotated_assignment(
                    content,
                    ann_assign,
                    current_function,
                    current_scope,
                ));
            }
            ast::Stmt::Import(import) => {
                usages.extend(self.detect_import_usage(
                    content,
                    import,
                    current_function,
                    current_scope,
                ));
            }
            ast::Stmt::ImportFrom(import_from) => {
                usages.extend(self.detect_import_from_usage(
                    content,
                    import_from,
                    current_function,
                    current_scope,
                ));
            }
            ast::Stmt::If(if_stmt) => {
                usages.extend(self.detect_usages_in_body(
                    content,
                    &if_stmt.body,
                    current_function,
                    current_scope,
                )?);
                usages.extend(self.detect_usages_in_body(
                    content,
                    &if_stmt.orelse,
                    current_function,
                    current_scope,
                )?);
            }
            ast::Stmt::For(for_stmt) => {
                usages.extend(self.detect_usages_in_body(
                    content,
                    &for_stmt.body,
                    current_function,
                    current_scope,
                )?);
                usages.extend(self.detect_usages_in_body(
                    content,
                    &for_stmt.orelse,
                    current_function,
                    current_scope,
                )?);
            }
            ast::Stmt::While(while_stmt) => {
                usages.extend(self.detect_usages_in_body(
                    content,
                    &while_stmt.body,
                    current_function,
                    current_scope,
                )?);
                usages.extend(self.detect_usages_in_body(
                    content,
                    &while_stmt.orelse,
                    current_function,
                    current_scope,
                )?);
            }
            _ => {
                // For other statement types, traverse their expressions
                self.traverse_stmt_expressions(stmt, |expr| {
                    usages.extend(self.detect_usages_in_expr(
                        content,
                        expr,
                        current_function,
                        current_scope,
                    ));
                });
            }
        }

        Ok(usages)
    }

    /// Detect usages within a body of statements
    pub fn detect_usages_in_body(
        &self,
        content: &str,
        body: &[ast::Stmt],
        current_function: Option<&str>,
        current_scope: &str,
    ) -> VecqResult<Vec<DocumentElement>> {
        let mut usages = Vec::new();
        for stmt in body {
            usages.extend(self.detect_usages_in_stmt(
                content,
                stmt,
                current_function,
                current_scope,
            )?);
        }
        Ok(usages)
    }

    /// Detect usages within an expression
    pub fn detect_usages_in_expr(
        &self,
        content: &str,
        expr: &ast::Expr,
        current_function: Option<&str>,
        current_scope: &str,
    ) -> Vec<DocumentElement> {
        let mut usages = Vec::new();

        match expr {
            ast::Expr::Call(call) => {
                usages.extend(self.detect_call_expression(
                    content,
                    call,
                    current_function,
                    current_scope,
                ));
                // Recursively check arguments
                for arg in &call.args {
                    usages.extend(self.detect_usages_in_expr(
                        content,
                        arg,
                        current_function,
                        current_scope,
                    ));
                }
                for kw in &call.keywords {
                    usages.extend(self.detect_usages_in_expr(
                        content,
                        &kw.value,
                        current_function,
                        current_scope,
                    ));
                }
            }
            ast::Expr::Attribute(attr) => {
                usages.extend(self.detect_attribute_access(
                    content,
                    attr,
                    current_function,
                    current_scope,
                ));
                // Recursively check the base object
                usages.extend(self.detect_usages_in_expr(
                    content,
                    &attr.value,
                    current_function,
                    current_scope,
                ));
            }
            ast::Expr::Name(name) => {
                usages.extend(self.detect_name_usage(
                    content,
                    name,
                    current_function,
                    current_scope,
                ));
            }
            ast::Expr::Subscript(subscript) => {
                usages.extend(self.detect_usages_in_expr(
                    content,
                    &subscript.value,
                    current_function,
                    current_scope,
                ));
                usages.extend(self.detect_usages_in_expr(
                    content,
                    &subscript.slice,
                    current_function,
                    current_scope,
                ));
            }
            ast::Expr::List(list) => {
                for elt in &list.elts {
                    usages.extend(self.detect_usages_in_expr(
                        content,
                        elt,
                        current_function,
                        current_scope,
                    ));
                }
            }
            ast::Expr::Dict(dict) => {
                for k in dict.keys.iter().flatten() {
                    usages.extend(self.detect_usages_in_expr(
                        content,
                        k,
                        current_function,
                        current_scope,
                    ));
                }
                for value in &dict.values {
                    usages.extend(self.detect_usages_in_expr(
                        content,
                        value,
                        current_function,
                        current_scope,
                    ));
                }
            }
            ast::Expr::Tuple(tuple) => {
                for elt in &tuple.elts {
                    usages.extend(self.detect_usages_in_expr(
                        content,
                        elt,
                        current_function,
                        current_scope,
                    ));
                }
            }
            _ => {
                // For other expression types, we could add more specific handling
                // but for now we just traverse any sub-expressions they might contain
                self.traverse_expr_subexpressions(expr, |sub_expr| {
                    usages.extend(self.detect_usages_in_expr(
                        content,
                        sub_expr,
                        current_function,
                        current_scope,
                    ));
                });
            }
        }

        usages
    }

    /// Detect function/method calls
    pub fn detect_call_expression(
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
            self.byte_offset_to_line_number(content, call.range.start().to_u32() as usize),
            self.byte_offset_to_line_number(content, call.range.end().to_u32() as usize),
        )
        .set_attributes(ElementAttributes::Usage(usage_attr));

        usages.push(element);
        usages
    }

    /// Detect attribute/field access
    pub fn detect_attribute_access(
        &self,
        content: &str,
        attr: &ast::ExprAttribute,
        current_function: Option<&str>,
        current_scope: &str,
    ) -> Vec<DocumentElement> {
        let mut usages = Vec::new();

        let field_name = attr.attr.to_string();

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
            self.byte_offset_to_line_number(content, attr.range.start().to_u32() as usize),
            self.byte_offset_to_line_number(content, attr.range.end().to_u32() as usize),
        )
        .set_attributes(ElementAttributes::Usage(usage_attr));

        usages.push(element);
        usages
    }

    /// Detect variable name usage
    pub fn detect_name_usage(
        &self,
        content: &str,
        name: &ast::ExprName,
        current_function: Option<&str>,
        current_scope: &str,
    ) -> Vec<DocumentElement> {
        let mut usages = Vec::new();

        let var_name = name.id.to_string();

        let usage_attr = UsageAttributes {
            symbol_name: var_name.clone(),
            usage_type: "reference".to_string(),
            context: current_function.unwrap_or("global").to_string(),
            scope: current_scope.to_string(),
            other: HashMap::new(),
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
    pub fn detect_assignment(
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
                    self.byte_offset_to_line_number(
                        content,
                        assign.range.start().to_u32() as usize,
                    ),
                    self.byte_offset_to_line_number(content, assign.range.end().to_u32() as usize),
                )
                .set_attributes(ElementAttributes::Usage(usage_attr));

                usages.push(element);
            }
        }

        usages
    }

    /// Detect annotated assignments (type annotations)
    pub fn detect_annotated_assignment(
        &self,
        content: &str,
        ann_assign: &ast::StmtAnnAssign,
        current_function: Option<&str>,
        current_scope: &str,
    ) -> Vec<DocumentElement> {
        let mut usages = Vec::new();

        if let ast::Expr::Name(name) = &*ann_assign.target {
            let var_name = name.id.to_string();

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
                format!(
                    "{}: {} = ...",
                    var_name,
                    self.ast_to_string(&ann_assign.annotation)
                ),
                self.byte_offset_to_line_number(
                    content,
                    ann_assign.range.start().to_u32() as usize,
                ),
                self.byte_offset_to_line_number(content, ann_assign.range.end().to_u32() as usize),
            )
            .set_attributes(ElementAttributes::Usage(usage_attr));

            usages.push(element);
        }

        usages
    }

    /// Detect import usage
    pub fn detect_import_usage(
        &self,
        content: &str,
        import: &ast::StmtImport,
        current_function: Option<&str>,
        current_scope: &str,
    ) -> Vec<DocumentElement> {
        let mut usages = Vec::new();

        for alias in &import.names {
            let import_name = alias.asname.as_ref().unwrap_or(&alias.name).to_string();

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
                self.byte_offset_to_line_number(content, import.range.start().to_u32() as usize),
                self.byte_offset_to_line_number(content, import.range.end().to_u32() as usize),
            )
            .set_attributes(ElementAttributes::Usage(usage_attr));

            usages.push(element);
        }

        usages
    }

    /// Detect import from usage
    pub fn detect_import_from_usage(
        &self,
        content: &str,
        import_from: &ast::StmtImportFrom,
        current_function: Option<&str>,
        current_scope: &str,
    ) -> Vec<DocumentElement> {
        let mut usages = Vec::new();

        for alias in &import_from.names {
            let import_name = alias.asname.as_ref().unwrap_or(&alias.name).to_string();

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
                format!("from ... import {}", import_name),
                self.byte_offset_to_line_number(
                    content,
                    import_from.range.start().to_u32() as usize,
                ),
                self.byte_offset_to_line_number(content, import_from.range.end().to_u32() as usize),
            )
            .set_attributes(ElementAttributes::Usage(usage_attr));

            usages.push(element);
        }

        usages
    }

    /// Helper to traverse statement expressions
    pub fn traverse_stmt_expressions<F>(&self, stmt: &ast::Stmt, mut f: F)
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
    pub fn traverse_expr_subexpressions<F>(&self, expr: &ast::Expr, mut f: F)
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
}
