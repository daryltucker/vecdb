//! Python calls and string literals, from
//! rustpython's AST through its generated `Visitor`, so every expression kind
//! is walked — a call inside a lambda, a decorator, a default argument or an
//! f-string hole is still a call. Ownership is `scopes::assign`, the rule
//! every language shares.

use crate::parsers::ts_refs::collapse;
use crate::types::{Arg, ArgKind, Call, Scope, StringKind, StringLiteral};
use rustpython_ast::Visitor;
use rustpython_parser::ast::{self, Ranged};
use rustpython_parser::{lexer, Mode, Tok};
use std::collections::HashSet;
use vecdb_common::LineCounter;

/// Every call and string literal in `suite`, in source order.
pub(crate) fn extract(source: &str, suite: &[ast::Stmt]) -> Scope {
    let mut docstrings = HashSet::new();
    mark_docstring(suite, &mut docstrings);
    let mut w = Walk {
        source,
        lines: LineCounter::new(source),
        string_tokens: string_token_starts(source),
        docstrings,
        out: Scope::default(),
    };
    for stmt in suite.iter().cloned() {
        w.visit_stmt(stmt);
    }
    w.out.calls.sort_by_key(|c| c.start);
    w.out.strings.sort_by_key(|s| s.start);
    w.out
}

/// The first statement of a body, when it is a bare string, is its docstring.
fn mark_docstring(body: &[ast::Stmt], out: &mut HashSet<usize>) {
    if let Some(ast::Stmt::Expr(e)) = body.first() {
        if let ast::Expr::Constant(c) = e.value.as_ref() {
            if matches!(c.value, ast::Constant::Str(_)) {
                out.insert(c.range.start().to_usize());
            }
        }
    }
}

/// Start offset of every string token. rustpython joins implicitly
/// concatenated literals (`"a" "b"`) into one constant; two token starts
/// inside one constant's range is how that folding is seen.
fn string_token_starts(source: &str) -> Vec<usize> {
    lexer::lex(source, Mode::Module)
        .flatten()
        .filter(|(tok, _)| matches!(tok, Tok::String { .. }))
        .map(|(_, range)| range.start().to_usize())
        .collect()
}

struct Walk<'a> {
    source: &'a str,
    lines: LineCounter,
    string_tokens: Vec<usize>,
    docstrings: HashSet<usize>,
    out: Scope,
}

/// A string as one value: its kind, value, holes, and whether it was folded.
struct Folded {
    kind: StringKind,
    value: String,
    holes: Vec<String>,
    folded: bool,
}

impl Walk<'_> {
    fn text(&self, r: &impl Ranged) -> &str {
        let range = r.range();
        self.source
            .get(range.start().to_usize()..range.end().to_usize())
            .unwrap_or_default()
    }

    fn tokens_in(&self, start: usize, end: usize) -> usize {
        let lo = self.string_tokens.partition_point(|&s| s < start);
        let hi = self.string_tokens.partition_point(|&s| s < end);
        hi - lo
    }

    /// `e` as one string, when it is one: a literal, an f-string, or a `+`
    /// chain with a literal in it (`"UPDATE " + table + " SET x"`).
    fn as_string(&self, e: &ast::Expr) -> Option<Folded> {
        match e {
            ast::Expr::Constant(c) => {
                let (start, end) = (c.range.start().to_usize(), c.range.end().to_usize());
                let (kind, value) = match &c.value {
                    ast::Constant::Str(s) if self.docstrings.contains(&start) => {
                        (StringKind::Docstring, s.clone())
                    }
                    ast::Constant::Str(s) => (StringKind::Plain, s.clone()),
                    ast::Constant::Bytes(b) => {
                        (StringKind::Bytes, String::from_utf8_lossy(b).into_owned())
                    }
                    _ => return None,
                };
                Some(Folded {
                    kind,
                    value,
                    holes: Vec::new(),
                    folded: self.tokens_in(start, end) > 1,
                })
            }
            ast::Expr::JoinedStr(j) => {
                let mut value = String::new();
                let mut holes = Vec::new();
                for part in &j.values {
                    match part {
                        ast::Expr::Constant(c) => {
                            if let ast::Constant::Str(s) = &c.value {
                                value.push_str(s);
                            }
                        }
                        ast::Expr::FormattedValue(f) => {
                            value.push_str("{}");
                            holes.push(collapse(self.text(f.value.as_ref())));
                        }
                        other => {
                            value.push_str("{}");
                            holes.push(collapse(self.text(other)));
                        }
                    }
                }
                let (start, end) = (j.range.start().to_usize(), j.range.end().to_usize());
                Some(Folded {
                    kind: StringKind::Fstring,
                    value,
                    holes,
                    folded: self.tokens_in(start, end) > 1,
                })
            }
            ast::Expr::BinOp(b) if matches!(b.op, ast::Operator::Add) => {
                let mut leaves = Vec::new();
                plus_leaves(e, &mut leaves);
                let parts: Vec<Option<Folded>> = leaves.iter().map(|l| self.leaf(l)).collect();
                if !parts.iter().any(Option::is_some) {
                    return None;
                }
                let mut value = String::new();
                let mut holes = Vec::new();
                for (leaf, part) in leaves.iter().zip(parts) {
                    match part {
                        Some(p) => {
                            value.push_str(&p.value);
                            holes.extend(p.holes);
                        }
                        None => {
                            value.push_str("{}");
                            holes.push(collapse(self.text(*leaf)));
                        }
                    }
                }
                let kind = if holes.is_empty() {
                    StringKind::Plain
                } else {
                    StringKind::Fstring
                };
                Some(Folded {
                    kind,
                    value,
                    holes,
                    folded: true,
                })
            }
            _ => None,
        }
    }

    /// A `+` chain's leaf as a string — a literal or f-string, never a nested
    /// chain (those are flattened by `plus_leaves`).
    fn leaf(&self, e: &ast::Expr) -> Option<Folded> {
        match e {
            ast::Expr::Constant(_) | ast::Expr::JoinedStr(_) => self.as_string(e),
            _ => None,
        }
    }

    fn record_string(&mut self, e: &ast::Expr, s: Folded) {
        let (start, end) = (e.range().start().to_usize(), e.range().end().to_usize());
        let (line, line_end) = self.lines.line_range(start, end);
        self.out.strings.push(StringLiteral {
            kind: s.kind,
            value: s.value,
            line,
            line_end,
            start,
            end,
            folded: s.folded,
            holes: s.holes,
        });
    }

    fn arg(&self, e: &ast::Expr, keyword: Option<String>) -> Arg {
        if let Some(s) = self.as_string(e) {
            return Arg {
                kind: if s.kind == StringKind::Bytes {
                    ArgKind::Bytes
                } else {
                    ArgKind::String
                },
                value: s.value,
                keyword,
                folded: s.folded,
                holes: s.holes,
            };
        }
        let kind = match e {
            ast::Expr::Name(_) | ast::Expr::Attribute(_) => ArgKind::Name,
            ast::Expr::Constant(c) => match c.value {
                ast::Constant::Int(_) | ast::Constant::Float(_) | ast::Constant::Complex { .. } => {
                    ArgKind::Number
                }
                _ => ArgKind::Other,
            },
            ast::Expr::Tuple(_) => ArgKind::Tuple,
            ast::Expr::List(_) | ast::Expr::Set(_) => ArgKind::List,
            ast::Expr::Dict(_) => ArgKind::Dict,
            ast::Expr::Call(_) => ArgKind::Call,
            _ => ArgKind::Other,
        };
        Arg {
            kind,
            value: collapse(self.text(e)),
            keyword,
            folded: false,
            holes: Vec::new(),
        }
    }
}

fn plus_leaves<'e>(e: &'e ast::Expr, out: &mut Vec<&'e ast::Expr>) {
    match e {
        ast::Expr::BinOp(b) if matches!(b.op, ast::Operator::Add) => {
            plus_leaves(&b.left, out);
            plus_leaves(&b.right, out);
        }
        _ => out.push(e),
    }
}

impl Visitor for Walk<'_> {
    fn visit_stmt_function_def(&mut self, node: ast::StmtFunctionDef) {
        mark_docstring(&node.body, &mut self.docstrings);
        self.generic_visit_stmt_function_def(node)
    }

    fn visit_stmt_async_function_def(&mut self, node: ast::StmtAsyncFunctionDef) {
        mark_docstring(&node.body, &mut self.docstrings);
        self.generic_visit_stmt_async_function_def(node)
    }

    fn visit_stmt_class_def(&mut self, node: ast::StmtClassDef) {
        mark_docstring(&node.body, &mut self.docstrings);
        self.generic_visit_stmt_class_def(node)
    }

    fn visit_expr_call(&mut self, node: ast::ExprCall) {
        let mut args: Vec<Arg> = node.args.iter().map(|a| self.arg(a, None)).collect();
        args.extend(node.keywords.iter().map(|k| {
            // `**kwargs` has no name; it is reported as written.
            let mut a = self.arg(&k.value, k.arg.as_ref().map(|i| i.to_string()));
            if k.arg.is_none() {
                a.kind = ArgKind::Other;
                a.value = collapse(self.text(k));
            }
            a
        }));
        let (start, end) = (node.range.start().to_usize(), node.range.end().to_usize());
        let (line, line_end) = self.lines.line_range(start, end);
        self.out.calls.push(Call {
            callee: collapse(self.text(node.func.as_ref())),
            line,
            line_end,
            start,
            end,
            args,
        });
        self.generic_visit_expr_call(node)
    }

    fn visit_expr(&mut self, node: ast::Expr) {
        // A string (or a folded one) is recorded once, whole; only its holes
        // are walked on — its literal parts are not strings of their own.
        if let Some(s) = self.as_string(&node) {
            self.record_string(&node, s);
            let mut leaves = Vec::new();
            plus_leaves(&node, &mut leaves);
            let holes: Vec<ast::Expr> = leaves
                .into_iter()
                .flat_map(|leaf| match leaf {
                    ast::Expr::Constant(_) => vec![],
                    ast::Expr::JoinedStr(j) => j
                        .values
                        .iter()
                        .filter_map(|v| match v {
                            ast::Expr::FormattedValue(f) => Some((*f.value).clone()),
                            _ => None,
                        })
                        .collect(),
                    other => vec![other.clone()],
                })
                .collect();
            for h in holes {
                self.visit_expr(h);
            }
            return;
        }
        self.generic_visit_expr(node)
    }
}

#[cfg(test)]
mod tests {
    use crate::parser::Parser;
    use crate::parsers::PythonParser;
    use crate::types::{ArgKind, StringKind};

    /// Everything asked of a Python scope: docstrings,
    /// implicit concatenation folded, f-string holes, keyword and `**`
    /// arguments, and a call inside a lambda inside a call.
    #[tokio::test]
    async fn python_calls_and_strings_per_scope() {
        let src = "\"\"\"Mod.\"\"\"\n\ndef f():\n    \"\"\"Doc.\"\"\"\n    x = (\"a\" \"b\")\n    g(lambda: h(f\"v{y!r}\"), k=1, **kw)\n\nclass C:\n    def m(self):\n        return b\"raw\" + tail\n";
        let doc = PythonParser::new().parse(src).await.unwrap();
        let module = doc.module.as_ref().unwrap();
        assert_eq!(module.strings[0].kind, StringKind::Docstring);
        let f = doc
            .elements
            .iter()
            .find(|e| e.name.as_deref() == Some("f"))
            .unwrap();
        let s = f.scope.as_ref().unwrap();
        let callees: Vec<&str> = s.calls.iter().map(|c| c.callee.as_str()).collect();
        assert_eq!(callees, ["g", "h"], "h() inside the lambda is a call too");
        let g = &s.calls[0];
        assert_eq!(g.args[1].keyword.as_deref(), Some("k"));
        assert_eq!(g.args[1].kind, ArgKind::Number);
        assert_eq!(
            (g.args[2].kind, g.args[2].value.as_str()),
            (ArgKind::Other, "**kw")
        );
        assert_eq!(&src[g.start..g.end], "g(lambda: h(f\"v{y!r}\"), k=1, **kw)");
        let kinds: Vec<(StringKind, &str, bool)> = s
            .strings
            .iter()
            .map(|x| (x.kind, x.value.as_str(), x.folded))
            .collect();
        assert_eq!(
            kinds,
            [
                (StringKind::Docstring, "Doc.", false),
                (StringKind::Plain, "ab", true),
                (StringKind::Fstring, "v{}", false),
            ]
        );
        assert_eq!(s.strings[2].holes, ["y"]);
        // A method's strings are the method's, not the class's; `+` with a
        // non-literal is a hole.
        let class = doc
            .elements
            .iter()
            .find(|e| e.name.as_deref() == Some("C"))
            .unwrap();
        assert!(class.scope.as_ref().unwrap().strings.is_empty());
        let m = class.scope.as_ref().map(|_| &class.children[0]).unwrap();
        let ms = m.scope.as_ref().unwrap();
        assert_eq!(
            (ms.strings[0].value.as_str(), ms.strings[0].holes.as_slice()),
            ("raw{}", ["tail".to_string()].as_slice())
        );
    }
}
