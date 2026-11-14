// PURPOSE:
//   Make every tree-sitter parser COMPLETE: nothing the grammar parsed may be
//   dropped. One module, every tree-sitter language.
//
// WHY THIS EXISTS:
//   Each tree-sitter language has a hand-written walker that emits only the
//   node kinds someone wrote a `match` arm for; everything else fell to a
//   catch-all and vanished — C functions returning pointers, everything inside
//   `#ifdef` header guards, every Go `const`/`var`, every Bash command, Rust
//   doc comments. Measured loss: 24–100% per language. The tree-sitter tree
//   already covers every byte; the walkers threw that away.
//
//   "Parser completeness" had been defined as "every element has a name",
//   so no test ever asked whether anything was missing.
//   Here completeness means: every byte of the file the grammar parsed lands in
//   an element, and every node kind the grammar can produce is classified.
//
// HOW:
//   The language's own walker runs first and keeps doing what it does well —
//   names and attributes (visibility, signatures, docstrings, parameters).
//   Then `complete` walks the SAME tree and emits every node the walker did
//   not cover, with its real grammar kind. Nothing here is language-specific:
//   a language supplies only its `Spec` — a table classifying every kind.
//
//   This is NOT a text gap-filler. Every emitted element is a real tree-sitter
//   node with its real kind, and `Spec::kinds` must name every kind the grammar
//   can produce (enforced by this module's `*_spec_classifies_every_grammar_kind`
//   tests against the grammar's own NODE_TYPES; they were once a separate
//   `tier1_grammar_completeness.rs`) — nothing unclassified, nothing guessed.

use crate::types::{attr, DocumentElement, ElementType};
use tree_sitter::Node;
use vecdb_common::{has_meaning, LineCounter};

/// What a node kind is, for coverage.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Kind {
    /// Emitted as an element of this type when the walker did not cover it.
    Unit(ElementType),
    /// A wrapper whose children are the units (`#ifdef` blocks, `extern "C"`,
    /// declaration bodies). Its own text outside the children — `#ifndef X`,
    /// `#endif` — is emitted as a frame element so it is not lost either.
    Descend,
    /// A comment. Emitted as `ElementType::Comment`; consumers decide which
    /// neighbour owns it (vecq_adapter attaches a comment to what follows).
    Comment,
}

/// A language's coverage spec: the only per-language input.
pub(crate) struct Spec {
    /// Every node kind the grammar can produce at top level and in the
    /// declaration bodies listed in `bodies`, classified. The completeness test
    /// fails on any kind missing here.
    pub kinds: &'static [(&'static str, Kind)],
    /// Kinds of the containers whose children are walked: the root, and the
    /// declaration bodies (`declaration_list`, `field_declaration_list`, …).
    // Read by the completeness test, which must agree with the walk on what
    // "every kind" means; the walk itself starts from the root it is given.
    #[cfg_attr(not(test), allow(dead_code))]
    pub bodies: &'static [&'static str],
}

impl Spec {
    fn kind(&self, node_kind: &str) -> Option<Kind> {
        self.kinds
            .iter()
            .find(|(k, _)| *k == node_kind)
            .map(|(_, v)| *v)
    }
}

/// Complete `elements` (produced by the language's walker over `root`) so
/// that every node of the tree is covered, then put them in document order.
///
/// Every element leaves with its exact byte span (`DocumentElement::source_span`),
/// the walker's included — that span is the handoff to consumers, who must
/// never re-derive positions by searching text.
///
/// Returns the tree's syntax-error count ([`syntax_errors`]), which the caller
/// records as `DocumentMetadata::syntax_errors`. `must_use`,
/// so a parser cannot forget it.
#[must_use = "record it as DocumentMetadata::syntax_errors"]
pub(crate) fn complete(
    elements: &mut Vec<DocumentElement>,
    root: Node,
    source: &str,
    spec: &Spec,
) -> usize {
    complete_elements(elements, root, source, spec);
    syntax_errors(root)
}

/// The syntax errors tree-sitter recovered from: its `ERROR` nodes (text it
/// could not fit to the grammar — kept, spanned, as `kind: "ERROR"` elements)
/// and `MISSING` nodes (a token it had to assume, e.g. an unclosed `)`).
pub(crate) fn syntax_errors(root: Node) -> usize {
    if !root.has_error() {
        return 0;
    }
    let mut count = 0;
    let mut stack = vec![root];
    while let Some(n) = stack.pop() {
        if n.is_error() || n.is_missing() {
            count += 1;
        }
        if n.has_error() {
            let mut c = n.walk();
            stack.extend(n.children(&mut c));
        }
    }
    count
}

fn complete_elements(elements: &mut Vec<DocumentElement>, root: Node, source: &str, spec: &Spec) {
    let lines = LineCounter::new(source);
    let mut covered: Vec<(usize, usize)> = Vec::new();
    for element in elements.iter_mut() {
        element.stamp_source_span(source, &lines);
        collect_spans(element, &mut covered);
    }
    covered.sort_unstable();

    let mut added = Vec::new();
    walk(root, source, &lines, spec, &covered, &mut added);
    elements.extend(added);
    // A node's span can end in its newline (C's `preproc_include`, Bash's
    // heredocs): that newline is trivia, not the element's text.
    // Narrowed before trivia is assigned, so the byte is owned, not lost.
    for element in elements.iter_mut() {
        element.tighten_span(source);
    }
    // Document order. Stable, so a walker's own ordering of elements that start
    // on the same line is kept.
    elements.sort_by_key(|e| e.line_start);
    // Comments become trivia of the element they describe (see assign_trivia);
    // a file of nothing but comments keeps them as elements — no owner exists.
    let comments: Vec<(usize, usize)> = elements
        .iter()
        .filter(|e| e.element_type == ElementType::Comment)
        .filter_map(DocumentElement::source_span)
        .collect();
    if elements
        .iter()
        .all(|e| e.element_type == ElementType::Comment)
    {
        return;
    }
    elements.retain(|e| e.element_type != ElementType::Comment);
    super::ownership::assign_trivia(elements, source, &lines, &comments, |_| true);
    // Members own their trivia inside their container too:
    // a method's doc comment leads the method, not the `impl` frame. Every
    // comment of the tree counts — those inside a container are not elements.
    let all_comments = comment_spans(root);
    for element in elements.iter_mut() {
        super::ownership::assign_nested_trivia(element, source, &all_comments);
    }
}

/// Byte spans of every comment node in the tree, in order: the grammar's
/// extras whose kind names a comment (the same test `grammar_kinds` uses).
fn comment_spans(root: Node) -> Vec<(usize, usize)> {
    let mut out = Vec::new();
    let mut cursor = root.walk();
    let mut descend = true;
    loop {
        let node = cursor.node();
        if descend && node.is_extra() && node.kind().contains("comment") {
            out.push((node.start_byte(), node.end_byte()));
        }
        if descend && cursor.goto_first_child() {
            continue;
        }
        if cursor.goto_next_sibling() {
            descend = true;
            continue;
        }
        if !cursor.goto_parent() {
            break;
        }
        descend = false;
    }
    out.sort_unstable();
    out
}

/// Fields of a wrapper that hold its BODY, walked like unfielded children. Every
/// other field (`name`, `condition`, `value`, …) is the wrapper's own header —
/// `#ifndef GUARD_H`, `extern "C"` — and belongs to its frame, not to the units.
/// The same rule is applied by `grammar_kinds`, so the test and the walk agree.
const BODY_FIELDS: &[&str] = &["body", "alternative"];

/// The named children of `node` that are body items, in order.
fn body_children<'t>(node: Node<'t>) -> Vec<Node<'t>> {
    let mut cursor = node.walk();
    let mut out = Vec::new();
    if cursor.goto_first_child() {
        loop {
            let n = cursor.node();
            let field = cursor.field_name();
            if n.is_named() && field.is_none_or(|f| BODY_FIELDS.contains(&f)) {
                out.push(n);
            }
            if !cursor.goto_next_sibling() {
                break;
            }
        }
    }
    out
}

fn walk(
    node: Node,
    source: &str,
    lines: &LineCounter,
    spec: &Spec,
    covered: &[(usize, usize)],
    out: &mut Vec<DocumentElement>,
) {
    for child in body_children(node) {
        let range = (child.start_byte(), child.end_byte());
        if is_inside(range, covered) {
            continue;
        }
        if overlaps(range, covered) {
            // Part of it is already an element (a walker emitted a child but
            // not the wrapper): cover the rest at the next level down, then
            // whatever text of this node is STILL uncovered — Go's `type`
            // before the `type_spec` the walker emitted, CUDA's
            // `template <int N>` before the kernel.
            let before = out.len();
            walk(child, source, lines, spec, covered, out);
            let mut now: Vec<(usize, usize)> = covered.to_vec();
            now.extend(
                out[before..]
                    .iter()
                    .filter_map(DocumentElement::source_span),
            );
            let gaps = super::ownership::uncovered(range, &now);
            push_frames(child, source, lines, &gaps, out);
            continue;
        }
        match spec.kind(child.kind()) {
            Some(Kind::Descend) => {
                // Only the text before the first body item and after the last
                // is the wrapper's own (`#ifndef GUARD_H` … `#endif`); text
                // between body items is separators.
                let items: Vec<(usize, usize)> = body_children(child)
                    .iter()
                    .map(|c| (c.start_byte(), c.end_byte()))
                    .collect();
                let gaps = super::ownership::uncovered(range, &items);
                let ends = [
                    gaps.first().copied(),
                    gaps.last().copied().filter(|_| gaps.len() > 1),
                ];
                push_frames(
                    child,
                    source,
                    lines,
                    &ends.into_iter().flatten().collect::<Vec<_>>(),
                    out,
                );
                walk(child, source, lines, spec, covered, out);
            }
            // A comment only if the GRAMMAR says so (`is_extra`), never on the
            // table's word alone: comments become trivia, so a code kind
            // misclassified here would be hidden. Otherwise it is a unit.
            Some(Kind::Comment) if child.is_extra() => {
                out.push(element(child, source, ElementType::Comment))
            }
            Some(Kind::Comment) => out.push(element(child, source, ElementType::Block)),
            Some(Kind::Unit(t)) => out.push(element(child, source, t)),
            // Unclassified: the completeness test fails CI for this kind, but at
            // runtime the node is still emitted — never dropped — as a Block
            // carrying its real kind.
            None => out.push(element(child, source, ElementType::Block)),
        }
    }
}

/// Emit each gap of `node` that carries meaning as a frame element: the
/// node's own words that no other element holds. Whitespace and punctuation
/// alone (`};`, `}`) are not emitted — `vecdb_common::has_meaning`.
fn push_frames(
    node: Node,
    source: &str,
    lines: &LineCounter,
    gaps: &[(usize, usize)],
    out: &mut Vec<DocumentElement>,
) {
    for &(gs, ge) in gaps {
        let Some(text) = source.get(gs..ge) else {
            continue;
        };
        if !has_meaning(text) {
            continue;
        }
        let trimmed = text.trim();
        let start = gs + (text.len() - text.trim_start().len());
        let end = start + trimmed.len();
        let mut frame = DocumentElement::new(
            ElementType::Block,
            None,
            trimmed.to_string(),
            lines.get_line_number(start),
            lines.get_line_number(end.saturating_sub(1)),
        )
        .with_attribute(attr::KIND.to_string(), node.kind())
        .with_attribute(attr::FRAME.to_string(), true);
        frame.set_source_span(start, end);
        out.push(frame);
    }
}

fn element(node: Node, source: &str, t: ElementType) -> DocumentElement {
    let text = source
        .get(node.byte_range())
        .unwrap_or_default()
        .to_string();
    DocumentElement::new(
        t,
        name_of(node, source),
        text,
        node.start_position().row + 1,
        node.end_position().row + 1,
    )
    .with_attribute(attr::KIND.to_string(), node.kind())
    .with_source_span(node.start_byte(), node.end_byte())
}

/// A node's name by the rules grammars actually use — no per-language code:
/// the `name` field; else down the `declarator` chain (C, C++, CUDA name a
/// function inside `pointer_declarator` → `function_declarator` → …); else the
/// first named child that has a `name` field (Go's `type_declaration` names its
/// `type_spec`). A miss is an unnamed element, never a missing one.
fn name_of(node: Node, source: &str) -> Option<String> {
    let text = |n: Node| n.utf8_text(source.as_bytes()).ok().map(str::to_string);
    if let Some(n) = node.child_by_field_name("name") {
        return text(n);
    }
    let mut d = node.child_by_field_name("declarator");
    while let Some(n) = d {
        if let Some(inner) = n.child_by_field_name("declarator") {
            d = Some(inner);
            continue;
        }
        return matches!(
            n.kind(),
            "identifier"
                | "field_identifier"
                | "type_identifier"
                | "qualified_identifier"
                | "destructor_name"
                | "operator_name"
        )
        .then(|| text(n))
        .flatten();
    }
    let mut cursor = node.walk();
    let found = node
        .named_children(&mut cursor)
        .find_map(|c| c.child_by_field_name("name"))
        .and_then(text);
    found
}

// ---- spans of the walker's elements ------------------------------------------------

/// Record the spans of `element` and its children (stamped by
/// `DocumentElement::stamp_source_span`, the one implementation, shared with
/// `ParsedDocument::with_source`).
fn collect_spans(element: &DocumentElement, covered: &mut Vec<(usize, usize)>) {
    covered.extend(element.source_span());
    for child in &element.children {
        collect_spans(child, covered);
    }
}

fn is_inside((s, e): (usize, usize), covered: &[(usize, usize)]) -> bool {
    covered.iter().any(|&(cs, ce)| cs <= s && e <= ce)
}

fn overlaps((s, e): (usize, usize), covered: &[(usize, usize)]) -> bool {
    covered.iter().any(|&(cs, ce)| cs < e && s < ce)
}

/// Every kind a grammar can produce directly inside the given containers —
/// supertypes expanded — read from the grammar's own `node-types.json`. Used by
/// the completeness test; exposed so the test and the module agree on what
/// "every kind" means.
#[cfg_attr(not(test), allow(dead_code))]
pub fn grammar_kinds(node_types_json: &str, containers: &[&str]) -> Vec<String> {
    let nodes: Vec<serde_json::Value> = serde_json::from_str(node_types_json).unwrap_or_default();
    let by_name = |name: &str| {
        nodes
            .iter()
            .find(|n| n["type"] == name && n["named"] == true)
            .cloned()
    };
    fn expand(
        types: &[serde_json::Value],
        by_name: &dyn Fn(&str) -> Option<serde_json::Value>,
        seen: &mut std::collections::BTreeSet<String>,
        out: &mut std::collections::BTreeSet<String>,
    ) {
        for t in types {
            let Some(name) = t["type"].as_str() else {
                continue;
            };
            if t["named"] != true || !seen.insert(name.to_string()) {
                continue;
            }
            match by_name(name).and_then(|n| n["subtypes"].as_array().cloned()) {
                Some(subs) => expand(&subs, by_name, seen, out),
                None => {
                    out.insert(name.to_string());
                }
            }
        }
    }
    let mut out = std::collections::BTreeSet::new();
    for container in containers {
        let Some(node) = by_name(container) else {
            continue;
        };
        let mut types: Vec<serde_json::Value> = node["children"]["types"]
            .as_array()
            .cloned()
            .unwrap_or_default();
        if let Some(fields) = node["fields"].as_object() {
            for (name, f) in fields {
                // Header fields are frame text, never walked — see BODY_FIELDS.
                if BODY_FIELDS.contains(&name.as_str()) {
                    types.extend(f["types"].as_array().cloned().unwrap_or_default());
                }
            }
        }
        expand(
            &types,
            &by_name,
            &mut std::collections::BTreeSet::new(),
            &mut out,
        );
    }
    // Comments are extras: allowed anywhere, never listed as children.
    out.extend(
        nodes
            .iter()
            .filter(|n| n["named"] == true)
            .filter_map(|n| n["type"].as_str())
            .filter(|t| t.contains("comment"))
            .map(str::to_string),
    );
    out.into_iter().collect()
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::parser::Parser;

    /// Every kind the grammar can produce where the walk looks — from the
    /// grammar's own NODE_TYPES — must be classified in the language's Spec.
    /// A grammar bump that adds a kind fails here, naming it.
    pub(crate) fn assert_classifies_every_kind(lang: &str, node_types: &str, spec: &Spec) {
        let missing: Vec<String> = grammar_kinds(node_types, spec.bodies)
            .into_iter()
            .filter(|k| spec.kind(k).is_none())
            .collect();
        assert!(
            missing.is_empty(),
            "{lang}: grammar kinds not classified in COVERAGE: {missing:?}"
        );
    }

    /// LOSSLESS: rebuilding the file from its top-level elements — each one's
    /// owned leading trivia + its source text + its owned trailing trivia, in
    /// order — must reproduce `source` byte for byte.
    /// Proves nothing lost, nothing duplicated, order kept, comments kept. The
    /// parse itself panics (debug) if any non-comment text was left as trivia,
    /// so a coverage miss cannot hide inside the reconstruction.
    pub(crate) async fn assert_covers(lang: &str, parser: &dyn Parser, source: &str) {
        let doc = parser.parse(source).await.expect("fixture parses");
        super::super::ownership::assert_reconstructs(lang, &doc.elements, source, |_| true);
    }

    #[test]
    fn rust_spec_classifies_every_grammar_kind() {
        assert_classifies_every_kind(
            "rust",
            tree_sitter_rust::NODE_TYPES,
            &crate::parsers::rust::COVERAGE,
        );
    }

    #[tokio::test]
    async fn rust_fixtures_are_fully_covered() {
        let p = crate::parsers::rust::RustParser::new();
        for (name, src) in [
            (
                "declaration_kinds.rs",
                include_str!("../../tests/fixtures/rust/declaration_kinds.rs"),
            ),
            (
                "dummy.rs",
                include_str!("../../tests/fixtures/rust/dummy.rs"),
            ),
        ] {
            assert_covers(name, &p, src).await;
        }
    }

    macro_rules! lang_tests {
        ($kinds_test:ident, $cover_test:ident, $lang:literal, $node_types:expr,
         $spec:expr, $parser:expr, [$($fixture:literal),* $(,)?]) => {
            #[test]
            fn $kinds_test() {
                assert_classifies_every_kind($lang, $node_types, &$spec);
            }

            #[tokio::test]
            async fn $cover_test() {
                let p = $parser;
                $(
                    assert_covers(
                        $fixture,
                        &p,
                        include_str!(concat!("../../tests/fixtures/", $fixture)),
                    )
                    .await;
                )*
            }
        };
    }

    lang_tests!(
        c_spec_classifies_every_grammar_kind,
        c_fixtures_are_fully_covered,
        "c",
        tree_sitter_c::NODE_TYPES,
        crate::parsers::c::COVERAGE,
        crate::parsers::c::CParser::new(),
        ["c/declaration_kinds.c", "c/dummy.c"]
    );
    lang_tests!(
        cpp_spec_classifies_every_grammar_kind,
        cpp_fixtures_are_fully_covered,
        "cpp",
        tree_sitter_cpp::NODE_TYPES,
        crate::parsers::cpp::COVERAGE,
        crate::parsers::cpp::CppParser::new(),
        ["cpp/declaration_kinds.cpp", "cpp/complex.cpp"]
    );
    lang_tests!(
        cuda_spec_classifies_every_grammar_kind,
        cuda_fixtures_are_fully_covered,
        "cuda",
        tree_sitter_cuda::NODE_TYPES,
        crate::parsers::cuda::COVERAGE,
        crate::parsers::cuda::CudaParser::new(),
        [
            "cuda/declaration_kinds.cu",
            "cuda/kernel.cu",
            "cuda/ggml-cuda.cu"
        ]
    );
    lang_tests!(
        go_spec_classifies_every_grammar_kind,
        go_fixtures_are_fully_covered,
        "go",
        tree_sitter_go::NODE_TYPES,
        crate::parsers::go::COVERAGE,
        crate::parsers::go::GoParser::new(),
        ["go/declaration_kinds.go", "go/main.go"]
    );
    lang_tests!(
        bash_spec_classifies_every_grammar_kind,
        bash_fixtures_are_fully_covered,
        "bash",
        tree_sitter_bash::NODE_TYPES,
        crate::parsers::bash::COVERAGE,
        crate::parsers::bash::BashParser::new(),
        ["bash/declaration_kinds.sh", "bash/script.sh"]
    );

    /// HTML walks its own tree (elements nest), but the grammar's kinds are
    /// classified the same way, so a tree-sitter-html bump that adds a kind
    /// fails here rather than being dropped.
    #[test]
    fn html_spec_classifies_every_grammar_kind() {
        assert_classifies_every_kind(
            "html",
            tree_sitter_html::NODE_TYPES,
            &crate::parsers::html::COVERAGE,
        );
    }

    #[test]
    fn javascript_spec_classifies_every_grammar_kind() {
        assert_classifies_every_kind(
            "javascript",
            tree_sitter_javascript::NODE_TYPES,
            &crate::parsers::javascript::COVERAGE,
        );
    }

    #[tokio::test]
    async fn javascript_is_fully_covered() {
        // No vecq fixture exists for JavaScript; exports and top-level
        // statements are exactly what the walker drops.
        let src = "// config loader\nimport fs from 'fs';\nexport function load(p) { return fs.readFileSync(p); }\nexport const VERSION = '1.0';\nclass Cache { get(k) { return k; } }\nconsole.log(load('x'));\n";
        assert_covers(
            "inline.js",
            &crate::parsers::javascript::JavaScriptParser::new(),
            src,
        )
        .await;
    }

    /// The safeguard in `assign_trivia`: code left between elements must never
    /// be absorbed as trivia — that is exactly where a coverage miss would hide,
    /// with reconstruction still passing. Proven by mutation (2026-300).
    #[test]
    #[should_panic(expected = "uncovered text left for trivia")]
    fn trivia_never_absorbs_code() {
        let src = "a();\nb();\n";
        let lines = LineCounter::new(src);
        let mut els =
            vec![
                DocumentElement::new(ElementType::Block, None, "a();".to_string(), 1, 1)
                    .with_source_span(0, 4),
            ];
        // `b();` is in no element and is not a comment.
        super::super::ownership::assign_trivia(&mut els, src, &lines, &[], |_| true);
    }
}
