//! `vecq refs <symbol>`: every call site and import of a name, across files,
//! with its exact span and enclosing scope.
//!
//! "Who calls `backends.connect`?" and "who imports `sqlite3`?" are the first
//! two questions of any refactor. Before this they were a grep, and a grep
//! matches comments, strings and lookalikes. This is a library API first —
//! ivaldi's symbol traversal ("look at a function and follow it across files without grepping") calls it directly; the CLI is a thin shell over it.
//!
//! Matching is by NAME, not resolution: the reference's dotted path (`a.b.c`,
//! `a::b::c`) matches when the symbol's segments are its tail (`refs connect`,
//! `refs sqlite3.connect`) or its head (`refs sqlite3` finds
//! `sqlite3.connect(..)`, a use of the module). `x.alpha()` matches `alpha`
//! whatever `x` is — vecq has no type information, and says so here rather
//! than pretending.

use crate::error::{VecqError, VecqResult};
use crate::types::{refs::MODULE_SCOPE, DocumentElement, ElementType, FileType, ParsedDocument};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ReferenceKind {
    Call,
    Import,
}

/// One reference to a symbol.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Reference {
    /// The file, when the reference came from [`find_references`].
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub file: Option<PathBuf>,
    pub kind: ReferenceKind,
    /// What matched, as written: a call's callee, or an import's path.
    pub name: String,
    pub line: usize,
    pub line_end: usize,
    /// Byte span: the whole call expression, or the import statement.
    pub start: usize,
    pub end: usize,
    /// Enclosing scopes, outermost first (`["Store", "save"]`);
    /// `["<module>"]` at module level.
    pub scope: Vec<String>,
}

/// The references to `symbol` in one parsed document, in source order.
pub fn references_in(doc: &ParsedDocument, symbol: &str) -> Vec<Reference> {
    let want = segments(symbol);
    if want.is_empty() {
        return Vec::new();
    }
    let mut out = Vec::new();
    if let Some(module) = &doc.module {
        for call in &module.calls {
            if path_matches(&call.callee, &want) {
                out.push(call_ref(call, vec![MODULE_SCOPE.to_string()]));
            }
        }
    }
    walk(&doc.elements, &mut Vec::new(), &want, &mut out);
    out.sort_by_key(|r| (r.start, r.end));
    out
}

fn call_ref(call: &crate::types::Call, scope: Vec<String>) -> Reference {
    Reference {
        file: None,
        kind: ReferenceKind::Call,
        name: call.callee.clone(),
        line: call.line,
        line_end: call.line_end,
        start: call.start,
        end: call.end,
        scope,
    }
}

fn walk(
    elements: &[DocumentElement],
    path: &mut Vec<String>,
    want: &[String],
    out: &mut Vec<Reference>,
) {
    for e in elements {
        if e.element_type == ElementType::Import {
            if let Some(hit) = import_paths(e).into_iter().find(|p| path_matches(p, want)) {
                let (start, end) = e.source_span().unwrap_or((0, 0));
                out.push(Reference {
                    file: None,
                    kind: ReferenceKind::Import,
                    name: hit,
                    line: e.line_start,
                    line_end: e.line_end,
                    start,
                    end,
                    scope: if path.is_empty() {
                        vec![MODULE_SCOPE.to_string()]
                    } else {
                        path.clone()
                    },
                });
            }
        }
        let named = e.scope.is_some();
        if named {
            path.push(e.name.clone().unwrap_or_else(|| e.element_type.to_string()));
        }
        if let Some(scope) = &e.scope {
            for call in &scope.calls {
                if path_matches(&call.callee, want) {
                    out.push(call_ref(call, path.clone()));
                }
            }
        }
        walk(&e.children, path, want, out);
        if named {
            path.pop();
        }
    }
}

/// The dotted paths an import brings in: `import os.path` → `os.path`;
/// `from a.b import c` → `a.b.c` (and `a.b`); `use a::{b, c::d}` → `a::b`,
/// `a::c::d`; Go `"os/exec"` → `os/exec`.
pub fn import_paths(e: &DocumentElement) -> Vec<String> {
    let text = |k: &str| {
        e.attributes
            .get(k)
            .and_then(|v| v.as_str())
            .map(str::to_string)
    };
    let mut out = Vec::new();
    if let Some(module) = text("module") {
        if let Some(name) =
            text("name").filter(|_| text("import_type").as_deref() == Some("from_import"))
        {
            out.push(format!("{module}.{name}"));
        }
        out.push(module);
        return out;
    }
    let content = e.content.trim();
    if let Some(at) = content
        .find("use ")
        .filter(|&i| i == 0 || content[..i].trim_start().starts_with("pub"))
    {
        let tree = content[at + 4..].trim().trim_end_matches(';').trim();
        expand_use(tree, "", &mut out);
        return out;
    }
    if let Some(name) = &e.name {
        out.push(name.clone());
    }
    out
}

/// Expand a Rust use tree into full paths.
fn expand_use(tree: &str, prefix: &str, out: &mut Vec<String>) {
    let tree = tree.trim();
    if let Some(open) = tree.find('{') {
        let head = tree[..open].trim().trim_end_matches("::");
        let close = tree.rfind('}').unwrap_or(tree.len());
        let inner = &tree[open + 1..close];
        let base = join(prefix, head);
        let mut depth = 0;
        let mut from = 0;
        for (i, ch) in inner.char_indices() {
            match ch {
                '{' => depth += 1,
                '}' => depth -= 1,
                ',' if depth == 0 => {
                    expand_use(&inner[from..i], &base, out);
                    from = i + 1;
                }
                _ => {}
            }
        }
        expand_use(&inner[from..], &base, out);
        return;
    }
    let leaf = tree.split(" as ").next().unwrap_or(tree).trim();
    match leaf {
        "" => {}
        "self" => out.push(prefix.to_string()),
        _ => out.push(join(prefix, leaf)),
    }
}

fn join(prefix: &str, rest: &str) -> String {
    match (prefix.is_empty(), rest.is_empty()) {
        (true, _) => rest.to_string(),
        (_, true) => prefix.to_string(),
        _ => format!("{prefix}::{rest}"),
    }
}

/// A path's segments, split on `.`, `::`, `->` and `/`, each stripped of call
/// parentheses, generic arguments and a macro's `!`: `x.iter().map` →
/// `x`, `iter`, `map`; `Vec::<u8>::new` → `Vec`, `new`.
pub fn segments(path: &str) -> Vec<String> {
    let mut clean = String::new();
    let mut depth = 0usize;
    for ch in path.chars() {
        match ch {
            '(' | '<' | '[' => depth += 1,
            ')' | '>' | ']' if depth > 0 => depth -= 1,
            _ if depth == 0 => clean.push(ch),
            _ => {}
        }
    }
    clean
        .replace("->", ".")
        .replace("::", ".")
        .replace('/', ".")
        .split('.')
        .map(|s| {
            s.trim()
                .trim_end_matches('!')
                .trim_end_matches('-')
                .to_string()
        })
        .filter(|s| !s.is_empty())
        .collect()
}

/// How many leading segments of `path` name a namespace or receiver — i.e.
/// come before the first called segment. `sqlite3.connect` → 2 (no call);
/// `alpha(1).map` → 0 (`alpha` is called, so what follows is its result);
/// `x.iter().map` → 1 (`x`).
fn uncalled_prefix_len(path: &str) -> usize {
    let mut depth = 0usize;
    for (i, ch) in path.char_indices() {
        match ch {
            '<' | '[' => depth += 1,
            '>' | ']' if depth > 0 => depth -= 1,
            '(' if depth == 0 => {
                // The segment just before the `(` is the call itself.
                return segments(&path[..i]).len().saturating_sub(1);
            }
            _ => {}
        }
    }
    segments(path).len()
}

/// A symbol matches a path by its tail (the thing named) or its head (a use
/// of the module/receiver it is reached through) — never its middle.
///
/// The head match only reaches as far as the first CALL.
/// `segments` drops call parentheses, so `alpha(1).map(..).unwrap_or` used to
/// read as the path `alpha.map.unwrap_or` and "start with" `alpha` — one call
/// of `alpha` written as a chain was reported once per link. `alpha(1).map`
/// calls `map` on alpha's result; `sqlite3.connect(..)` still uses `sqlite3`.
fn path_matches(path: &str, want: &[String]) -> bool {
    let have = segments(path);
    have.len() >= want.len()
        && (have.ends_with(want)
            || (want.len() <= uncalled_prefix_len(path) && have.starts_with(want)))
}

/// The result of a search over files.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct RefsReport {
    pub symbol: String,
    pub references: Vec<Reference>,
    pub files_searched: usize,
    /// Files of a supported type that failed to parse — reported, never
    /// silently skipped: a refs search that quietly missed a file is a wrong
    /// answer.
    pub failed: Vec<FailedFile>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FailedFile {
    pub file: PathBuf,
    pub error: String,
}

/// Search `paths` (files, or directories walked recursively) for references
/// to `symbol`. Directories honour `.vectorignore` and skip hidden entries;
/// `.gitignore` is NOT applied — it lists build artifacts, not what a search
/// should see (the project's standing rule). Files of a type vecq cannot
/// parse are not searched.
pub async fn find_references(paths: &[PathBuf], symbol: &str) -> VecqResult<RefsReport> {
    let mut report = RefsReport {
        symbol: symbol.to_string(),
        ..Default::default()
    };
    for root in paths {
        if !root.exists() {
            return Err(VecqError::IoError(std::io::Error::new(
                std::io::ErrorKind::NotFound,
                format!("{}: no such file or directory", root.display()),
            )));
        }
        for file in files_under(root) {
            let file_type = crate::detect_file_type(&file.to_string_lossy());
            if !matches!(
                file_type,
                FileType::Rust
                    | FileType::Python
                    | FileType::Go
                    | FileType::C
                    | FileType::Cpp
                    | FileType::Cuda
                    | FileType::Bash
                    | FileType::JavaScript
            ) {
                continue;
            }
            report.files_searched += 1;
            let parsed = match tokio::fs::read_to_string(&file).await {
                Ok(content) => crate::parse_file(&content, file_type).await,
                Err(e) => Err(VecqError::IoError(e)),
            };
            match parsed {
                Ok(doc) => {
                    report
                        .references
                        .extend(references_in(&doc, symbol).into_iter().map(|mut r| {
                            r.file = Some(file.clone());
                            r
                        }));
                }
                Err(e) => report.failed.push(FailedFile {
                    file: file.clone(),
                    error: e.to_string(),
                }),
            }
        }
    }
    Ok(report)
}

fn files_under(root: &Path) -> Vec<PathBuf> {
    if root.is_file() {
        return vec![root.to_path_buf()];
    }
    let mut builder = ignore::WalkBuilder::new(root);
    builder
        .hidden(true)
        .git_ignore(false)
        .git_global(false)
        .git_exclude(false)
        .add_custom_ignore_filename(".vectorignore");
    let mut files: Vec<PathBuf> = builder
        .build()
        .filter_map(Result::ok)
        .filter(|e| e.file_type().is_some_and(|t| t.is_file()))
        .map(|e| e.into_path())
        .collect();
    files.sort();
    files
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn segments_strip_calls_generics_and_macro_bangs() {
        assert_eq!(segments("x.iter().map"), ["x", "iter", "map"]);
        assert_eq!(segments("Vec::<u8>::new"), ["Vec", "new"]);
        assert_eq!(segments("println!"), ["println"]);
        assert_eq!(segments("os/exec"), ["os", "exec"]);
        assert_eq!(segments("p->next"), ["p", "next"]);
    }

    #[test]
    fn a_symbol_matches_a_path_by_its_head_or_tail_never_its_middle() {
        let m = |path: &str, sym: &str| path_matches(path, &segments(sym));
        assert!(m("sqlite3.connect", "sqlite3.connect"));
        assert!(m("sqlite3.connect", "connect"));
        assert!(m("sqlite3.connect", "sqlite3"), "a use of the module");
        assert!(m("crate::a::alpha", "alpha"));
        assert!(m("self.alpha", "alpha"));
        assert!(!m("alphabet", "alpha"), "segments, not substrings");
        assert!(
            !m("a.b.c", "b"),
            "the middle is neither the receiver nor the name"
        );
        assert!(!m("connect", "sqlite3.connect"));
    }

    /// A called segment ends the path: `alpha(1).map` calls `map` on alpha's
    /// result, not a use of `alpha` as a namespace; a call through a module
    /// (`sqlite3.connect(..)`) still uses the module.
    #[test]
    fn a_call_result_is_not_a_path_prefix() {
        let m = |path: &str, sym: &str| path_matches(path, &segments(sym));
        assert!(!m("alpha(1).map", "alpha"));
        assert!(!m("alpha(1).map(|v| v + 1).unwrap_or", "alpha"));
        assert!(m("alpha(1).map", "map"), "the tail is still the callee");
        assert!(m("sqlite3.connect", "sqlite3"));
        assert!(m("crate::a::alpha", "crate::a"));
        assert!(m("x.iter().map", "map"));
    }

    #[test]
    fn use_trees_expand_to_full_paths() {
        let mut out = Vec::new();
        expand_use("crate::a::{b, c::{d, e as f}, self}", "", &mut out);
        assert_eq!(
            out,
            [
                "crate::a::b",
                "crate::a::c::d",
                "crate::a::c::e",
                "crate::a"
            ]
        );
    }
}
