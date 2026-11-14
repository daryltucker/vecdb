//! Calls and string literals from a tree-sitter tree.
//!
//! One walk for every tree-sitter language; a language supplies only its
//! [`RefSpec`] — which node kinds are calls, strings and interpolation holes.
//! Ownership (which scope an item belongs to) is `scopes::assign`, shared with
//! Python.
//!
//! Strings are folded the way a reader reads them: `"UPDATE " + table + "
//! SET x"` is ONE string, `UPDATE {} SET x` with hole `table`; C's adjacent
//! literals `"a" "b"` are one; a shell string's `$expansions` and a template's
//! `${…}` are holes. The literal parts of a folded string are not listed again
//! on their own.

use crate::parsers::scopes;
use crate::types::{Arg, ArgKind, Call, DocumentElement, Scope, StringKind, StringLiteral};
use tree_sitter::Node;
use vecdb_common::LineCounter;

/// How a call node names its callee and holds its arguments.
#[derive(Debug, Clone, Copy)]
pub(crate) enum CallShape {
    /// `function(arguments)`: the callee and argument-list fields.
    Fields {
        callee: &'static str,
        args: &'static str,
    },
    /// A Rust macro: callee field `macro`, arguments a `token_tree` split on
    /// its top-level commas. The callee is reported with its `!`.
    Macro,
    /// A shell command: field `name`, each `argument` field an argument.
    Command,
}

/// A language's input to the walk.
pub(crate) struct RefSpec {
    pub calls: &'static [(&'static str, CallShape)],
    /// String literal kinds.
    pub strings: &'static [&'static str],
    /// Kinds that are a hole inside a string (`${x}`, `$x`, `$(cmd)`).
    pub holes: &'static [&'static str],
    /// Kinds that join adjacent parts into one string (C `concatenated_string`).
    pub concat: &'static [&'static str],
    /// Binary-expression kind whose `+` folds strings (`left`/`operator`/`right`).
    pub binary: Option<&'static str>,
    /// Argument kinds that are a name or a dotted path.
    pub names: &'static [&'static str],
    /// Rust only: a `token_tree` is tokens, not expressions — tree-sitter does
    /// not parse `assert_eq!(alpha(), 1)`'s `alpha()` as a call. With this on,
    /// a path immediately followed by a `( … )` token tree inside a token tree
    /// is a call, so calls in macro arguments are not lost.
    pub token_tree_calls: bool,
}

// The languages, in one table so they can be read side by side. Kinds are the
// grammars' own (`node-types.json`); `tests` below exercise each.

pub(crate) const RUST: RefSpec = RefSpec {
    calls: &[
        (
            "call_expression",
            CallShape::Fields {
                callee: "function",
                args: "arguments",
            },
        ),
        ("macro_invocation", CallShape::Macro),
    ],
    strings: &["string_literal", "raw_string_literal"],
    holes: &[],
    concat: &[],
    binary: Some("binary_expression"),
    names: &[
        "identifier",
        "field_expression",
        "scoped_identifier",
        "self",
        "generic_function",
    ],
    token_tree_calls: true,
};

pub(crate) const GO: RefSpec = RefSpec {
    calls: &[(
        "call_expression",
        CallShape::Fields {
            callee: "function",
            args: "arguments",
        },
    )],
    strings: &["interpreted_string_literal", "raw_string_literal"],
    holes: &[],
    concat: &[],
    binary: Some("binary_expression"),
    names: &["identifier", "selector_expression", "field_identifier"],
    token_tree_calls: false,
};

/// C, C++ and CUDA: one family of grammars, the same kinds.
pub(crate) const C_FAMILY: RefSpec = RefSpec {
    calls: &[(
        "call_expression",
        CallShape::Fields {
            callee: "function",
            args: "arguments",
        },
    )],
    strings: &["string_literal", "raw_string_literal"],
    holes: &[],
    concat: &["concatenated_string"],
    binary: Some("binary_expression"),
    names: &[
        "identifier",
        "field_expression",
        "qualified_identifier",
        "this",
    ],
    token_tree_calls: false,
};

pub(crate) const BASH: RefSpec = RefSpec {
    calls: &[("command", CallShape::Command)],
    strings: &["string", "raw_string", "ansi_c_string", "translated_string"],
    holes: &[
        "simple_expansion",
        "expansion",
        "command_substitution",
        "arithmetic_expansion",
    ],
    concat: &[],
    binary: None,
    names: &["simple_expansion", "expansion", "variable_name"],
    token_tree_calls: false,
};

// JavaScript has a parser but no `FileType` yet, so this
// is reached only through `JavaScriptParser` directly.
pub(crate) const JAVASCRIPT: RefSpec = RefSpec {
    calls: &[
        (
            "call_expression",
            CallShape::Fields {
                callee: "function",
                args: "arguments",
            },
        ),
        (
            "new_expression",
            CallShape::Fields {
                callee: "constructor",
                args: "arguments",
            },
        ),
    ],
    strings: &["string", "template_string"],
    holes: &["template_substitution"],
    concat: &[],
    binary: Some("binary_expression"),
    names: &["identifier", "member_expression", "this"],
    token_tree_calls: false,
};

/// Extract every call and string under `root`, give each scope element its
/// own, and return the module scope (`ParsedDocument::module`).
#[must_use = "store it as ParsedDocument::module"]
pub(crate) fn attach(
    elements: &mut [DocumentElement],
    root: Node,
    source: &str,
    spec: &RefSpec,
) -> Scope {
    let found = extract(root, source, spec);
    scopes::assign(elements, found)
}

pub(crate) fn extract(root: Node, source: &str, spec: &RefSpec) -> Scope {
    let mut w = Walk {
        source,
        spec,
        lines: LineCounter::new(source),
        out: Scope::default(),
    };
    w.visit(root);
    w.out.calls.sort_by_key(|c| c.start);
    w.out.strings.sort_by_key(|s| s.start);
    w.out
}

struct Walk<'a> {
    source: &'a str,
    spec: &'a RefSpec,
    lines: LineCounter,
    out: Scope,
}

impl<'a> Walk<'a> {
    fn text(&self, n: Node) -> &'a str {
        &self.source[n.byte_range()]
    }

    fn visit(&mut self, root: Node) {
        let mut stack = vec![root];
        while let Some(n) = stack.pop() {
            let kind = n.kind();
            if let Some(&(_, shape)) = self.spec.calls.iter().find(|(k, _)| *k == kind) {
                if let Some(call) = self.call(n, shape) {
                    self.out.calls.push(call);
                }
            } else if self.spec.token_tree_calls && kind == "token_tree" {
                self.token_tree_calls(n);
            }
            if let Some(s) = self.string_like(n) {
                // The parts of a folded string are not strings of their own;
                // its holes are still walked (a call inside `{…}` is a call).
                let holes = self.hole_nodes(n);
                self.out.strings.push(s);
                stack.extend(holes);
                continue;
            }
            let mut c = n.walk();
            let children: Vec<Node> = n.children(&mut c).collect();
            stack.extend(children.into_iter().rev());
        }
    }

    fn span(&self, n: Node) -> (usize, usize, usize, usize) {
        let (start, end) = (n.start_byte(), n.end_byte());
        let (line, line_end) = self.lines.line_range(start, end);
        (start, end, line, line_end)
    }

    fn call(&self, n: Node, shape: CallShape) -> Option<Call> {
        let (callee, args) = match shape {
            CallShape::Fields { callee, args } => {
                let f = n.child_by_field_name(callee)?;
                let args = n
                    .child_by_field_name(args)
                    .map(|a| self.named_args(a))
                    .unwrap_or_default();
                (collapse(self.text(f)), args)
            }
            CallShape::Macro => {
                let m = n.child_by_field_name("macro")?;
                let mut c = n.walk();
                let tt = n.children(&mut c).find(|k| k.kind() == "token_tree");
                let args = tt.map(|t| self.token_args(t)).unwrap_or_default();
                (format!("{}!", collapse(self.text(m))), args)
            }
            CallShape::Command => {
                let name = n.child_by_field_name("name")?;
                let mut c = n.walk();
                let args = n
                    .children_by_field_name("argument", &mut c)
                    .map(|a| self.arg(a))
                    .collect();
                (collapse(self.text(name)), args)
            }
        };
        let (start, end, line, line_end) = self.span(n);
        Some(Call {
            callee,
            line,
            line_end,
            start,
            end,
            args,
        })
    }

    fn named_args(&self, list: Node) -> Vec<Arg> {
        let mut c = list.walk();
        list.named_children(&mut c)
            .filter(|a| !a.kind().contains("comment"))
            .map(|a| self.arg(a))
            .collect()
    }

    fn arg(&self, n: Node) -> Arg {
        if let Some(s) = self.string_like(n) {
            return Arg {
                kind: if s.kind == StringKind::Bytes {
                    ArgKind::Bytes
                } else {
                    ArgKind::String
                },
                value: s.value,
                keyword: None,
                folded: s.folded,
                holes: s.holes,
            };
        }
        let kind = n.kind();
        let kind = if self.spec.calls.iter().any(|(k, _)| *k == kind) {
            ArgKind::Call
        } else if self.spec.names.contains(&kind) {
            ArgKind::Name
        } else if ["integer", "float", "number", "int_literal"]
            .iter()
            .any(|k| kind.contains(k))
        {
            ArgKind::Number
        } else if kind == "tuple_expression" {
            ArgKind::Tuple
        } else if kind.contains("array")
            || kind == "composite_literal"
            || kind == "initializer_list"
        {
            ArgKind::List
        } else {
            ArgKind::Other
        };
        other(kind, self.text(n))
    }

    /// A Rust macro's arguments: its token tree's children between the
    /// delimiters, split on top-level commas.
    fn token_args(&self, tt: Node) -> Vec<Arg> {
        let mut c = tt.walk();
        let kids: Vec<Node> = tt.children(&mut c).collect();
        let inner = kids.get(1..kids.len().saturating_sub(1)).unwrap_or(&[]);
        inner
            .split(|k| k.kind() == ",")
            .filter(|group| !group.is_empty())
            .map(|group| {
                if let [one] = group {
                    return self.arg(*one);
                }
                let text = &self.source[group[0].start_byte()..group[group.len() - 1].end_byte()];
                let path_token = |k: &Node| {
                    matches!(
                        k.kind(),
                        "identifier" | "." | "::" | "self" | "crate" | "super"
                    )
                };
                let (last, head) = group.split_last().expect("group is non-empty");
                let kind = if group.iter().all(path_token) {
                    ArgKind::Name
                } else if last.kind() == "token_tree"
                    && self.text(*last).starts_with('(')
                    && head.iter().all(|k| path_token(k) || k.kind() == "!")
                {
                    // `alpha()`, `a::b(x)`, `format!(..)` — listed as calls too.
                    ArgKind::Call
                } else {
                    ArgKind::Other
                };
                other(kind, text)
            })
            .collect()
    }

    /// Inside a token tree: `path ( … )` is a call, `path ! ( … )` a macro.
    fn token_tree_calls(&mut self, tt: Node) {
        let mut c = tt.walk();
        let kids: Vec<Node> = tt.children(&mut c).collect();
        for (i, k) in kids.iter().enumerate() {
            if k.kind() != "token_tree" || !self.text(*k).starts_with('(') || i == 0 {
                continue;
            }
            let mut j = i;
            let bang = kids[j - 1].kind() == "!";
            if bang {
                j -= 1;
            }
            if j == 0 || kids[j - 1].kind() != "identifier" {
                continue;
            }
            // Extend back over `a.b::c` path segments.
            let mut first = j - 1;
            while first >= 2
                && matches!(kids[first - 1].kind(), "." | "::")
                && matches!(
                    kids[first - 2].kind(),
                    "identifier" | "self" | "crate" | "super"
                )
            {
                first -= 2;
            }
            let callee_end = kids[j - 1].end_byte();
            let mut callee = collapse(&self.source[kids[first].start_byte()..callee_end]);
            if bang {
                callee.push('!');
            }
            let (start, end) = (kids[first].start_byte(), k.end_byte());
            let (line, line_end) = self.lines.line_range(start, end);
            self.out.calls.push(Call {
                callee,
                line,
                line_end,
                start,
                end,
                args: self.token_args(*k),
            });
        }
    }

    /// `n` as one string, when it is a literal, a concatenation, or a `+`
    /// chain with at least one literal in it.
    fn string_like(&self, n: Node) -> Option<StringLiteral> {
        let kind = n.kind();
        let (value, holes, folded) = if self.spec.strings.contains(&kind) {
            let (v, h) = self.literal(n);
            (v, h, false)
        } else if self.spec.concat.contains(&kind) {
            let mut c = n.walk();
            let parts: Vec<Node> = n.named_children(&mut c).collect();
            let (v, h) = self.fold(&parts)?;
            (v, h, true)
        } else if Some(kind) == self.spec.binary && self.is_plus(n) {
            let mut parts = Vec::new();
            self.plus_leaves(n, &mut parts);
            let (v, h) = self.fold(&parts)?;
            (v, h, true)
        } else {
            return None;
        };
        let text = self.text(n);
        let kind = if text.starts_with('b') && self.spec.strings.contains(&n.kind()) {
            StringKind::Bytes
        } else if holes.is_empty() {
            StringKind::Plain
        } else {
            StringKind::Fstring
        };
        let (start, end, line, line_end) = self.span(n);
        Some(StringLiteral {
            kind,
            value,
            line,
            line_end,
            start,
            end,
            folded,
            holes,
        })
    }

    fn is_plus(&self, n: Node) -> bool {
        n.child_by_field_name("operator")
            .is_some_and(|op| self.text(op) == "+")
    }

    fn plus_leaves<'t>(&self, n: Node<'t>, out: &mut Vec<Node<'t>>) {
        if Some(n.kind()) == self.spec.binary && self.is_plus(n) {
            for f in ["left", "right"] {
                if let Some(side) = n.child_by_field_name(f) {
                    self.plus_leaves(side, out);
                }
            }
        } else {
            out.push(n);
        }
    }

    /// Join parts into one value: literals contribute their text, anything
    /// else is a `{}` hole. `None` when no part is a literal.
    fn fold(&self, parts: &[Node]) -> Option<(String, Vec<String>)> {
        if !parts.iter().any(|p| self.spec.strings.contains(&p.kind())) {
            return None;
        }
        let mut value = String::new();
        let mut holes = Vec::new();
        for p in parts {
            if self.spec.strings.contains(&p.kind()) {
                let (v, h) = self.literal(*p);
                value.push_str(&v);
                holes.extend(h);
            } else {
                value.push_str("{}");
                holes.push(collapse(self.text(*p)));
            }
        }
        Some((value, holes))
    }

    /// A literal's text between its delimiters, each hole as `{}`.
    fn literal(&self, n: Node) -> (String, Vec<String>) {
        let mut c = n.walk();
        let kids: Vec<Node> = n.children(&mut c).collect();
        let is_delim = |k: &Node| !k.is_named() || k.kind().ends_with("delimiter");
        let inner: Vec<&Node> = {
            let mut lo = 0;
            let mut hi = kids.len();
            while lo < hi && is_delim(&kids[lo]) {
                lo += 1;
            }
            while hi > lo && is_delim(&kids[hi - 1]) {
                hi -= 1;
            }
            kids[lo..hi].iter().collect()
        };
        if kids.is_empty() {
            return (strip_quotes(self.text(n)).to_string(), Vec::new());
        }
        let (Some(first), Some(last)) = (inner.first(), inner.last()) else {
            return (String::new(), Vec::new());
        };
        let mut value = String::new();
        let mut holes = Vec::new();
        let mut at = first.start_byte();
        for k in &inner {
            if self.spec.holes.contains(&k.kind()) {
                value.push_str(&self.source[at..k.start_byte()]);
                value.push_str("{}");
                holes.push(collapse(self.text(**k)));
                at = k.end_byte();
            }
        }
        value.push_str(&self.source[at..last.end_byte()]);
        (value, holes)
    }

    /// The expressions behind a string's holes — walked on, since a hole can
    /// hold a call or a string of its own.
    fn hole_nodes<'t>(&self, n: Node<'t>) -> Vec<Node<'t>> {
        let mut c = n.walk();
        if self.spec.strings.contains(&n.kind()) {
            return n
                .children(&mut c)
                .filter(|k| self.spec.holes.contains(&k.kind()))
                .collect();
        }
        let parts: Vec<Node<'t>> = if self.spec.concat.contains(&n.kind()) {
            n.named_children(&mut c).collect()
        } else {
            let mut parts = Vec::new();
            self.plus_leaves(n, &mut parts);
            parts
        };
        parts
            .into_iter()
            .flat_map(|p| {
                if self.spec.strings.contains(&p.kind()) {
                    self.hole_nodes(p)
                } else {
                    vec![p]
                }
            })
            .collect()
    }
}

fn other(kind: ArgKind, text: &str) -> Arg {
    Arg {
        kind,
        value: collapse(text),
        keyword: None,
        folded: false,
        holes: Vec::new(),
    }
}

/// Whitespace runs to one space: a callee split over lines reads as written.
pub(crate) fn collapse(s: &str) -> String {
    s.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// A leaf literal with no child nodes (`'raw'` in Bash): the text between its
/// first quote and the last.
fn strip_quotes(s: &str) -> &str {
    let Some(open) = s.find(['"', '\'', '`']) else {
        return s;
    };
    let q = s.as_bytes()[open] as char;
    match s.rfind(q) {
        Some(close) if close > open => &s[open + 1..close],
        _ => s,
    }
}

#[cfg(test)]
mod tests {
    use crate::parser::Parser;
    use crate::types::{ArgKind, ParsedDocument, Scope, StringKind};

    async fn parse(parser: impl Parser, src: &str) -> ParsedDocument {
        parser.parse(src).await.unwrap()
    }

    fn scope_of<'d>(doc: &'d ParsedDocument, name: &str) -> &'d Scope {
        fn find<'e>(es: &'e [crate::types::DocumentElement], name: &str) -> Option<&'e Scope> {
            es.iter().find_map(|e| {
                if e.name.as_deref() == Some(name) {
                    e.scope.as_ref()
                } else {
                    find(&e.children, name)
                }
            })
        }
        find(&doc.elements, name).unwrap_or_else(|| panic!("no scope {name}"))
    }

    fn callees(s: &Scope) -> Vec<&str> {
        s.calls.iter().map(|c| c.callee.as_str()).collect()
    }

    /// Rust: plain, path and method calls; a macro is a call with its `!`; a
    /// call inside a macro's token tree is still found; `+` folds strings
    /// around a hole; `b".."` is bytes.
    #[tokio::test]
    async fn rust_calls_macros_folds_and_bytes() {
        let src =
            "fn f() {\n    assert_eq!(g(1), 2);\n    a::h(\"x\" + y, b\"raw\");\n    s.m();\n}\n";
        let doc = parse(crate::parsers::RustParser::new(), src).await;
        let f = scope_of(&doc, "f");
        assert_eq!(callees(f), ["assert_eq!", "g", "a::h", "s.m"]);
        assert_eq!(
            f.calls[0].args[0].kind,
            ArgKind::Call,
            "g(1) inside the macro"
        );
        let h = &f.calls[2];
        assert_eq!((h.args[0].value.as_str(), h.args[0].folded), ("x{}", true));
        assert_eq!(h.args[0].holes, ["y"]);
        assert_eq!(h.args[1].kind, ArgKind::Bytes);
        assert_eq!(&src[h.start..h.end], "a::h(\"x\" + y, b\"raw\")");
        let kinds: Vec<StringKind> = f.strings.iter().map(|s| s.kind).collect();
        assert_eq!(
            kinds,
            [StringKind::Fstring, StringKind::Bytes],
            "the folded string once, not its parts"
        );
    }

    #[tokio::test]
    async fn go_calls_and_raw_strings() {
        let src = "package main\n\nfunc run() {\n\tdb.Exec(`DELETE FROM t`, id)\n}\n";
        let doc = parse(crate::parsers::GoParser::new(), src).await;
        let run = scope_of(&doc, "run");
        assert_eq!(callees(run), ["db.Exec"]);
        assert_eq!(run.calls[0].args[0].value, "DELETE FROM t");
        assert_eq!(run.calls[0].args[1].kind, ArgKind::Name);
    }

    /// C: adjacent literals are one folded string; a macro between them is a
    /// hole; top-level initialisers are the module's.
    #[tokio::test]
    async fn c_adjacent_literals_fold() {
        let src = "const char *Q = \"SELECT \" \"x\";\nint main(void) {\n  printf(\"%\" PRIu64 \"\\n\", n);\n  return 0;\n}\n";
        let doc = parse(crate::parsers::CParser::new(), src).await;
        let module = doc.module.as_ref().unwrap();
        assert_eq!(module.strings.len(), 1);
        assert_eq!(
            (module.strings[0].value.as_str(), module.strings[0].folded),
            ("SELECT x", true)
        );
        let main = scope_of(&doc, "main");
        assert_eq!(callees(main), ["printf"]);
        let fmt = &main.calls[0].args[0];
        assert_eq!(
            (fmt.value.as_str(), fmt.holes.as_slice()),
            ("%{}\\n", ["PRIu64".to_string()].as_slice())
        );
    }

    /// Bash: a command is a call; a string's `$expansions` are holes.
    #[tokio::test]
    async fn bash_commands_and_expansions() {
        let src = "f() {\n  psql -c \"SELECT $col FROM t\" 'db'\n}\necho top\n";
        let doc = parse(crate::parsers::BashParser::new(), src).await;
        let f = scope_of(&doc, "f");
        assert_eq!(callees(f), ["psql"]);
        let sql = &f.calls[0].args[1];
        assert_eq!(
            (sql.value.as_str(), sql.holes.as_slice()),
            ("SELECT {} FROM t", ["$col".to_string()].as_slice())
        );
        assert_eq!(f.calls[0].args[2].value, "db");
        assert_eq!(callees(doc.module.as_ref().unwrap()), ["echo"]);
    }

    /// JavaScript (parser only; no `FileType` yet): template holes, `new`.
    #[tokio::test]
    async fn javascript_templates_and_new() {
        let src = "function f() {\n  db.query(`SELECT ${col} FROM t`);\n  new Pool(cfg);\n}\n";
        let doc = parse(crate::parsers::JavaScriptParser::new(), src).await;
        let module_and_scopes: Vec<&str> = {
            let mut all = callees(doc.module.as_ref().unwrap());
            for e in &doc.elements {
                if let Some(s) = &e.scope {
                    all.extend(callees(s));
                }
            }
            all
        };
        assert_eq!(module_and_scopes, ["db.query", "Pool"]);
    }
}
