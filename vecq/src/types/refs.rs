//! Calls and string literals per scope — the expression layer.
//!
//! Why: an agent's first move on an unfamiliar repo is "where does X happen?"
//! — every SQL call, every `subprocess` command, every caller of
//! `backends.connect`. Function-level elements answer "what is here"; these
//! answer "what does it do, and with which literal", as a jq expression rather
//! than a regex over `content` (which matches comments, and which is what the
//! AST was supposed to save the agent from).
//!
//! Every function, method, class, impl, trait, module and namespace element
//! carries a [`Scope`]; whatever lies outside all of them belongs to the
//! document's module scope (`ParsedDocument::module`). A call or string is
//! listed once, in its innermost scope.
//!
//! Every position is the same contract as an element's: byte
//! span `start..end` into the source, 1-indexed inclusive lines.

use serde::{Deserialize, Serialize};

/// The module scope's name, as Python spells it in a traceback.
pub const MODULE_SCOPE: &str = "<module>";

/// What a scope does: the calls it makes and the string literals it holds.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Scope {
    pub calls: Vec<Call>,
    pub strings: Vec<StringLiteral>,
}

impl Scope {
    pub fn is_empty(&self) -> bool {
        self.calls.is_empty() && self.strings.is_empty()
    }
}

/// One call site.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Call {
    /// The called expression as written, whitespace collapsed: `conn.execute`,
    /// `sqlite3.connect`, `crate::a::alpha`, `println!` (a Rust macro is a
    /// call), `psql` (a shell command). As written so `endswith("execute")`,
    /// `== "sqlite3.connect"` and `test("^subprocess\\.")` all work. Not
    /// resolved: `x.alpha()` is a call to `x.alpha` whatever `x` is.
    pub callee: String,
    pub line: usize,
    pub line_end: usize,
    pub start: usize,
    pub end: usize,
    pub args: Vec<Arg>,
}

/// One argument of a [`Call`], in order.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Arg {
    pub kind: ArgKind,
    /// A string's value (see [`StringLiteral::value`]); anything else as
    /// written.
    pub value: String,
    /// Python keyword argument: `timeout` for `timeout=5`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub keyword: Option<String>,
    /// A string built from parts: `"a" + "b"`, `"a" "b"`, an f-string.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub folded: bool,
    /// The expressions behind each `{}` in a folded `value`, in order.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub holes: Vec<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ArgKind {
    /// A string literal, or a string folded from literals and holes.
    String,
    Bytes,
    /// A name or dotted path: `PAGE_SQL`, `self.conn`, `a::B`.
    Name,
    Number,
    Tuple,
    List,
    Dict,
    Call,
    Other,
}

/// One string literal, or one string folded from parts.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct StringLiteral {
    pub kind: StringKind,
    /// Python: the decoded value (the parser decodes escapes). Every other
    /// language: the text between the delimiters as written — escape
    /// sequences are not decoded. Each hole of a folded string is `{}`.
    pub value: String,
    pub line: usize,
    pub line_end: usize,
    pub start: usize,
    pub end: usize,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub folded: bool,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub holes: Vec<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum StringKind {
    Plain,
    /// A string with holes: a Python f-string, a JavaScript template, a shell
    /// string with `$expansions`, or `+` concatenation with a non-literal.
    Fstring,
    Bytes,
    /// Python: the first statement of a module, class or function.
    Docstring,
}
