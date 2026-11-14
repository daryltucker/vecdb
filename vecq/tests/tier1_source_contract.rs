//! The source-span contract vecq makes to its consumers.
//!
//! ivaldi edits files through vecq: it selects an element with a query and
//! rewrites exactly that element's bytes. That is only safe if, for EVERY
//! element of EVERY file type vecq detects, the position vecq hands over is the
//! element and nothing else. Before this gate, Markdown's `line_end` named the
//! line AFTER the element and its `content` carried the following blank lines —
//! one list-item edit deleted the paragraph after it — while
//! TOML tables reported line 1 and YAML mappings had no span at all.
//!
//! For every fixture of every `FileType`, through the public `vecq::parse_file`
//! path, every element at every depth must satisfy:
//!
//! 1. it has a source span;
//! 2. `content` is exactly the source bytes of that span — not rendered, not
//!    re-serialised, no trailing newline (line breaks are trivia);
//! 3. `line_start` is the 1-indexed line holding the span's first byte and
//!    `line_end` the line holding its last byte — inclusive, both ends, in
//!    every language; so the file's lines `line_start..=line_end`, cut at the
//!    span's first and last column, are `content`;
//! 4. concatenating `leading + text + trailing` of the top-level elements, in
//!    order, reproduces the file byte for byte (`ParsedDocument::reconstruct`);
//! 5. children own their trivia inside their parent, no byte twice;
//! 6. every call and string literal a scope carries lies inside
//!    that scope, in no deeper one, with the same span/line contract — and the
//!    module scope holds only what no scope element contains;
//! 7. no two top-level elements overlap (one statement shared by two imports
//!    excepted).
//!
//! No file type is exempt: ivaldi offers `edit_file --query` on every file type
//! vecq detects and cannot carry a per-language exception list. A file type that
//! cannot meet this is not supported.
//!
//! The second half runs the RFC's own validation (V1–V5) against the built
//! `vecq` binary, byte offsets and all, so the CLI flag and the library agree.

mod common;

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use vecdb_common::LineCounter;
use vecq::types::{DocumentElement, FileType};

fn fixtures_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures")
}

/// Every fixture file under `tests/fixtures/<dir>/`, one level deep, with the
/// file type vecq detects for it. `external/` is vendored corpus, not ours.
fn fixtures() -> Vec<(PathBuf, FileType)> {
    let mut out = Vec::new();
    for dir in fs::read_dir(fixtures_root()).unwrap().flatten() {
        if !dir.path().is_dir() || dir.file_name() == "external" {
            continue;
        }
        for f in fs::read_dir(dir.path()).unwrap().flatten() {
            let path = f.path();
            let ft = vecq::detect_file_type(path.to_str().unwrap());
            if path.is_file() && ft != FileType::Unknown {
                out.push((path, ft));
            }
        }
    }
    out.sort_by(|a, b| a.0.cmp(&b.0));
    out
}

fn check_element(
    e: &DocumentElement,
    source: &str,
    lines: &LineCounter,
    at: &str,
    failures: &mut Vec<String>,
) {
    let what = format!(
        "{at}: {} {:?} (lines {}–{})",
        e.element_type,
        e.name.as_deref().unwrap_or(""),
        e.line_start,
        e.line_end
    );
    // 1. a span
    let Some((start, end)) = e.source_span() else {
        failures.push(format!("{what}: no source span"));
        return;
    };
    // 2. content is the span's text
    match source.get(start..end) {
        None => failures.push(format!("{what}: span {start}..{end} is not in the file")),
        Some(text) if text != e.content => failures.push(format!(
            "{what}: content is not the source text of its span {start}..{end}\n      \
             content: {:?}\n      source:  {:?}",
            short(&e.content),
            short(text)
        )),
        Some(_) => {}
    }
    if e.content.ends_with('\n') || e.content.ends_with('\r') {
        failures.push(format!(
            "{what}: content ends in a line break — trivia, not content"
        ));
    }
    // 3. inclusive 1-indexed lines, derived from the span
    let first = lines.get_line_number(start);
    let last = lines.get_line_number(end.saturating_sub(1).max(start));
    if (e.line_start, e.line_end) != (first, last) {
        failures.push(format!(
            "{what}: lines should be {first}–{last} (the lines of its first and last byte)"
        ));
    } else {
        // The lines, cut at the span's columns, are the content.
        let from = lines.line_offset(e.line_start).unwrap();
        let to = lines.line_offset(e.line_end + 1).unwrap_or(source.len());
        let block = &source[from..to];
        let cut = &block[start - from..(end - from).min(block.len())];
        if cut != e.content {
            failures.push(format!(
                "{what}: lines {}–{} cut at the span are not the content",
                e.line_start, e.line_end
            ));
        }
    }
    if e.line_end > lines.count() {
        failures.push(format!(
            "{what}: line_end past the file's last line {}",
            lines.count()
        ));
    }
    // 5. children own their trivia INSIDE the parent, and no byte twice: each
    // child's `leading + text + trailing` is the source at that place, lies
    // within the parent's span, and does not overlap a sibling's (R5).
    let mut extents: Vec<(usize, usize)> = Vec::new();
    for c in &e.children {
        let Some((s, t)) = c.source_span() else {
            continue;
        };
        let (lead, trail) = (c.leading_trivia(), c.trailing_trivia());
        let (from, to) = (s.saturating_sub(lead.len()), t + trail.len());
        if source.get(from..s) != Some(lead) || source.get(t..to) != Some(trail) {
            failures.push(format!(
                "{what}: child {:?} trivia is not the source around its span",
                c.name.as_deref().unwrap_or("")
            ));
        }
        if from < start || to > end {
            failures.push(format!(
                "{what}: child {:?} owns bytes {from}..{to} outside its parent {start}..{end}",
                c.name.as_deref().unwrap_or("")
            ));
        }
        extents.push((from, to));
    }
    extents.sort_unstable();
    if extents.windows(2).any(|w| w[1].0 < w[0].1) {
        // Shared spans (Go's grouped imports) own nothing; only a real
        // overlap of owned bytes is a violation.
        let real = e
            .children
            .iter()
            .filter_map(|c| c.source_span())
            .collect::<Vec<_>>();
        let mut spans = real.clone();
        spans.sort_unstable();
        spans.dedup();
        if spans.windows(2).all(|w| w[1].0 >= w[0].1) {
            failures.push(format!("{what}: children own overlapping trivia"));
        }
    }
    // 6. a scope's calls and strings lie inside it, carry the
    // same span/line contract, and belong to no deeper scope.
    if let Some(scope) = &e.scope {
        let items = scope
            .calls
            .iter()
            .map(|c| {
                (
                    c.start,
                    c.end,
                    c.line,
                    c.line_end,
                    format!("call {}", c.callee),
                )
            })
            .chain(scope.strings.iter().map(|s| {
                (
                    s.start,
                    s.end,
                    s.line,
                    s.line_end,
                    format!("string {:?}", short(&s.value)),
                )
            }));
        for (s, t, line, line_end, label) in items {
            if s < start || t > end {
                failures.push(format!(
                    "{what}: {label} at {s}..{t} lies outside the scope"
                ));
            }
            if (line, line_end) != lines.line_range(s, t) {
                failures.push(format!(
                    "{what}: {label} lines {line}–{line_end} disagree with its span"
                ));
            }
            if in_scope(&e.children, s) {
                failures.push(format!("{what}: {label} belongs to a deeper scope"));
            }
        }
    }
    for child in &e.children {
        check_element(child, source, lines, at, failures);
    }
}

fn in_scope(elements: &[DocumentElement], at: usize) -> bool {
    elements.iter().any(|e| {
        e.source_span().is_some_and(|(s, t)| s <= at && at < t)
            && (e.scope.is_some() || in_scope(&e.children, at))
    })
}

/// Check 7: no two top-level elements claim overlapping bytes, unless they are the
/// same statement (`from a import b, c` is two imports of one span). A
/// synthetic element spanning other elements — Python's old `<module>`,
/// first statement to last, swallowing any `def` between — reconstructs
/// (overlaps are merged) but puts the same text in two elements.
fn check_top_level_disjoint(doc: &vecq::ParsedDocument, at: &str, failures: &mut Vec<String>) {
    let top: Vec<&DocumentElement> = if doc.file_type() == FileType::Markdown {
        Vec::new() // flat list, nested blocks are siblings; checked via reconstruct
    } else {
        doc.elements.iter().collect()
    };
    let mut spans: Vec<(usize, usize, &DocumentElement)> = top
        .into_iter()
        .filter_map(|e| e.source_span().map(|(s, t)| (s, t, e)))
        .collect();
    spans.sort_by_key(|(s, t, _)| (*s, *t));
    spans.dedup_by_key(|(s, t, _)| (*s, *t));
    for w in spans.windows(2) {
        let ((s0, t0, a), (s1, t1, b)) = (w[0], w[1]);
        if s1 < t0 {
            failures.push(format!(
                "{at}: top-level {} {:?} {s0}..{t0} and {} {:?} {s1}..{t1} overlap",
                a.element_type,
                a.name.as_deref().unwrap_or(""),
                b.element_type,
                b.name.as_deref().unwrap_or("")
            ));
        }
    }
}

/// The module scope holds only what no scope element contains.
fn check_module_scope(
    doc: &vecq::ParsedDocument,
    source: &str,
    at: &str,
    failures: &mut Vec<String>,
) {
    let Some(module) = &doc.module else { return };
    let lines = LineCounter::new(source);
    for (s, t, line, line_end, label) in module
        .calls
        .iter()
        .map(|c| {
            (
                c.start,
                c.end,
                c.line,
                c.line_end,
                format!("call {}", c.callee),
            )
        })
        .chain(
            module
                .strings
                .iter()
                .map(|x| (x.start, x.end, x.line, x.line_end, "string".to_string())),
        )
    {
        if t > source.len() || (line, line_end) != lines.line_range(s, t) {
            failures.push(format!(
                "{at}: module {label} at {s}..{t}: span/lines disagree"
            ));
        }
        if in_scope(&doc.elements, s) {
            failures.push(format!(
                "{at}: module {label} at {s}..{t} lies inside a scope element"
            ));
        }
    }
}

fn short(s: &str) -> String {
    if s.chars().count() <= 100 {
        s.to_string()
    } else {
        format!("{}…", s.chars().take(100).collect::<String>())
    }
}

async fn check_file(path: &Path, ft: FileType, failures: &mut Vec<String>) {
    let source = fs::read_to_string(path).unwrap();
    let at = path
        .strip_prefix(fixtures_root())
        .unwrap()
        .display()
        .to_string();
    let doc = match vecq::parse_file(&source, ft).await {
        Ok(doc) => doc,
        Err(e) => {
            failures.push(format!("{at}: does not parse: {e}"));
            return;
        }
    };
    let lines = LineCounter::new(&source);
    for e in &doc.elements {
        check_element(e, &source, &lines, &at, failures);
    }
    check_top_level_disjoint(&doc, &at, failures);
    check_module_scope(&doc, &source, &at, failures);
    // 4. lossless
    match doc.reconstruct(&source) {
        Ok(rebuilt) if rebuilt == source => {}
        Ok(rebuilt) => {
            let diff = rebuilt
                .bytes()
                .zip(source.bytes())
                .position(|(a, b)| a != b)
                .unwrap_or(rebuilt.len().min(source.len()));
            failures.push(format!(
                "{at}: top-level elements do not reconstruct the file (first difference at \
                 byte {diff}, line {}; rebuilt {} bytes, file {})",
                lines.get_line_number(diff),
                rebuilt.len(),
                source.len()
            ));
        }
        Err(e) => failures.push(format!("{at}: cannot reconstruct: {e}")),
    }
}

/// R3: the contract over every fixture of every file type.
#[tokio::test]
async fn every_element_of_every_file_type_meets_the_span_contract() {
    let fixtures = fixtures();
    // No file type may pass by having no fixture.
    for ft in vecq::parsers::available_parsers() {
        assert!(
            fixtures.iter().any(|(_, t)| *t == ft),
            "{ft:?} has no fixture under tests/fixtures — the contract is unchecked for it"
        );
    }
    let mut failures = Vec::new();
    for (path, ft) in &fixtures {
        check_file(path, *ft, &mut failures).await;
    }
    assert!(
        failures.is_empty(),
        "\n{} source-span contract violation(s):\n  {}\n",
        failures.len(),
        failures.join("\n  ")
    );
}

/// REGRESSION: the gate itself must see the Markdown defect —
/// an element whose `line_end` is the line after it, or whose content carries
/// the blank line after it, is a violation, not a pass.
#[test]
fn gate_rejects_an_exclusive_line_end_and_a_trailing_newline() {
    let src = "- [x] second item\n\nTrailing paragraph.\n";
    let lines = LineCounter::new(src);
    let mut bad = DocumentElement::new(
        vecq::types::ElementType::ListItem,
        None,
        "- [x] second item\n\n".to_string(),
        1,
        3,
    );
    bad.set_source_span(0, 19);
    let mut failures = Vec::new();
    check_element(&bad, src, &lines, "bad", &mut failures);
    assert!(
        failures.iter().any(|f| f.contains("line break")),
        "{failures:?}"
    );
    assert!(
        failures.iter().any(|f| f.contains("lines should be 1–2")),
        "{failures:?}"
    );
}

/// REGRESSION (2026-314): the old Python `<module>` element spanned the first
/// module statement to the last and so contained the `def` between them; and a
/// call must sit in its innermost scope. The gate must reject both.
#[test]
fn gate_rejects_overlapping_top_level_and_a_call_in_the_wrong_scope() {
    use vecq::types::{Call, ElementType, Scope};
    let src = "a()\n\ndef f():\n    g()\n\nb()\n";
    let mut module = DocumentElement::new(ElementType::Module, None, src[..27].to_string(), 1, 6);
    module.set_source_span(0, 27);
    let mut f = DocumentElement::new(ElementType::Function, None, src[5..22].to_string(), 3, 4);
    f.set_source_span(5, 22);
    let mut doc = vecq::ParsedDocument::new(vecq::DocumentMetadata::new(PathBuf::new(), 0));
    doc.metadata.file_type = FileType::Python;
    doc.elements = vec![module, f];
    let mut failures = Vec::new();
    check_top_level_disjoint(&doc, "old-module", &mut failures);
    assert!(
        failures.iter().any(|f| f.contains("overlap")),
        "{failures:?}"
    );

    // `g()` (bytes 18..21) reported on the class, not on the method inside it.
    let mut class = DocumentElement::new(ElementType::Class, None, src[5..22].to_string(), 3, 4);
    class.set_source_span(5, 22);
    let mut method =
        DocumentElement::new(ElementType::Function, None, src[18..21].to_string(), 4, 4);
    method.set_source_span(18, 21);
    method.scope = Some(Scope::default());
    class.children = vec![method];
    let call = Call {
        callee: "g".into(),
        line: 4,
        line_end: 4,
        start: 18,
        end: 21,
        args: vec![],
    };
    class.scope = Some(Scope {
        calls: vec![call],
        strings: vec![],
    });
    let mut failures = Vec::new();
    check_element(
        &class,
        src,
        &LineCounter::new(src),
        "wrong-scope",
        &mut failures,
    );
    assert!(
        failures.iter().any(|f| f.contains("deeper scope")),
        "{failures:?}"
    );
}

// ---- Validation, against the binary ---------------------------

fn vecq(args: &[&str]) -> serde_json::Value {
    let out = Command::new(common::vecq_bin())
        .args(args)
        .current_dir(fixtures_root().join("source_contract"))
        .output()
        .expect("vecq runs");
    assert!(
        out.status.success(),
        "vecq {args:?} failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    serde_json::from_slice(&out.stdout).unwrap_or_else(|e| {
        panic!(
            "vecq {args:?} printed non-JSON ({e}): {}",
            String::from_utf8_lossy(&out.stdout)
        )
    })
}

fn span(v: &serde_json::Value) -> (u64, u64) {
    (
        v["start"].as_u64().expect("source.start"),
        v["end"].as_u64().expect("source.end"),
    )
}

/// V1 (R1+R2, Markdown): exact content, inclusive lines, exact spans.
#[test]
fn v1_markdown_elements_are_exact() {
    let got = vecq(&[
        "probe.md",
        "--source-info",
        "-q",
        "[.headers[], .list_items[], .paragraphs[] | {type, content, line_start, line_end, source}] | sort_by(.source.start)",
        "--compact",
    ]);
    // (type, content, line_start, line_end, (start, end))
    type Row = (String, String, u64, u64, (u64, u64));
    let rows: Vec<Row> = got
        .as_array()
        .unwrap()
        .iter()
        .map(|r| {
            (
                r["type"].as_str().unwrap().to_string(),
                r["content"].as_str().unwrap().to_string(),
                r["line_start"].as_u64().unwrap(),
                r["line_end"].as_u64().unwrap(),
                span(&r["source"]),
            )
        })
        .collect();
    let want = [
        ("header", "# Title", 1, 1, (0, 7)),
        ("paragraph", "Intro paragraph.", 3, 3, (9, 25)),
        ("header", "## Tasks", 5, 5, (27, 35)),
        ("list_item", "- [ ] first item", 7, 7, (37, 53)),
        ("list_item", "- [x] second item", 8, 8, (54, 71)),
        ("paragraph", "Trailing paragraph.", 10, 10, (73, 92)),
    ];
    let want: Vec<_> = want
        .iter()
        .map(|(t, c, a, b, s)| (t.to_string(), c.to_string(), *a, *b, *s))
        .collect();
    assert_eq!(rows, want);
}

/// Without `--source-info`, no `source` reaches query output (default off).
#[test]
fn source_info_is_off_by_default() {
    let got = vecq(&[
        "probe.md",
        "-q",
        "[.. | objects | select(has(\"source\"))] | length",
    ]);
    assert_eq!(got, 0);
}

/// REGRESSION (2026-314): the CLI's path injection took any object with a
/// `kind` key for an element, so every coverage element's attributes got an
/// `attributes.attributes.file_path` nested inside them. Every element gets
/// `attributes.file_path` once; no attributes map gets attributes.
#[test]
fn file_path_is_injected_once_per_element() {
    let got = vecq(&[
        "broken.rs",
        "-q",
        "[.. | objects | select(.attributes?.attributes?)] | length",
    ]);
    assert_eq!(got, 0, "an attributes map was treated as an element");
    let got = vecq(&[
        "probe.rs",
        "-q",
        "[.. | objects | select(.type? and .line_start?) | .attributes.file_path] | unique",
    ]);
    assert_eq!(got, serde_json::json!(["probe.rs"]));
}

/// V2 (R1+R2+R5, Rust): a method's span starts after its indent, and it owns
/// its doc comment.
#[test]
fn v2_rust_spans_and_method_doc_comment() {
    let m = vecq(&[
        "impl.rs",
        "--source-info",
        "-q",
        ".functions[0] | {content, line_start, line_end, source}",
        "--compact",
    ]);
    assert_eq!(m["content"], "pub fn m(&self) -> u8 { 1 }");
    assert_eq!(
        (m["line_start"].as_u64(), m["line_end"].as_u64()),
        (Some(4), Some(4))
    );
    assert_eq!(span(&m["source"]), (37, 64));
    assert!(
        m["source"]["leading"]
            .as_str()
            .unwrap_or_default()
            .contains("/// doc m"),
        "the method owns its doc comment: {m}"
    );
    let fns = vecq(&[
        "probe.rs",
        "--source-info",
        "-q",
        "[.functions[] | {name, line_start, line_end, source: {start: .source.start, end: .source.end}}]",
        "--compact",
    ]);
    assert_eq!(
        fns,
        serde_json::json!([
            {"name": "alpha", "line_start": 1, "line_end": 3, "source": {"start": 0, "end": 20}},
            {"name": "beta", "line_start": 6, "line_end": 8, "source": {"start": 39, "end": 69}}
        ])
    );
}

/// V3 (R2, JSON as written): a pair's content is the pair's text.
#[test]
fn v3_json_pairs_are_as_written() {
    let got = vecq(&[
        "probe.json",
        "--source-info",
        "-q",
        // The RFC's query; an element appears in its group AND in its
        // parent's `children`, hence `unique_by`.
        "[.. | objects | select(.type?) | {name, content, source: {start: .source.start, end: .source.end}}] | unique_by(.source.start)",
        "--compact",
    ]);
    assert_eq!(
        got,
        serde_json::json!([
            {"name": "a", "content": "\"a\": 1", "source": {"start": 4, "end": 10}},
            {"name": "b", "content": "\"b\": {\"c\": 2}", "source": {"start": 14, "end": 27}},
            {"name": "c", "content": "\"c\": 2", "source": {"start": 20, "end": 26}}
        ])
    );
}

/// V4 (R3, TOML/YAML): tables and mappings carry spans and their real lines.
#[test]
fn v4_toml_and_yaml_spans_and_lines() {
    let toml = vecq(&[
        "probe.toml",
        "--source-info",
        "-q",
        "[.tables[] | {name, line_start, line_end, has: (.source != null)}]",
        "--compact",
    ]);
    let by_name = |v: &serde_json::Value, n: &str| -> serde_json::Value {
        v.as_array()
            .unwrap()
            .iter()
            .find(|r| r["name"] == n)
            .cloned()
            .unwrap_or_else(|| panic!("{n} missing from {v}"))
    };
    assert_eq!(by_name(&toml, "package")["line_start"], 1);
    assert_eq!(by_name(&toml, "deps")["line_start"], 4);
    let yaml = vecq(&[
        "probe.yaml",
        "--source-info",
        "-q",
        "[.. | objects | select(.type?) | {name, line_start, line_end, has: (.source != null)}] | unique",
        "--compact",
    ]);
    let services = by_name(&yaml, "services");
    assert_eq!(
        (
            services["line_start"].as_u64(),
            services["line_end"].as_u64()
        ),
        (Some(1), Some(3))
    );
    let qdrant = by_name(&yaml, "qdrant");
    assert_eq!(
        (qdrant["line_start"].as_u64(), qdrant["line_end"].as_u64()),
        (Some(2), Some(3))
    );
    for v in [&toml, &yaml] {
        assert!(
            v.as_array().unwrap().iter().all(|r| r["has"] == true),
            "an element without a span: {v}"
        );
    }
}

/// V5 (R4): syntax errors are counted in metadata and the ERROR element is
/// spanned.
#[test]
fn v5_syntax_errors_are_counted() {
    assert_eq!(vecq(&["broken.rs", "-q", ".metadata.syntax_errors"]), 1);
    assert_eq!(vecq(&["probe.rs", "-q", ".metadata.syntax_errors"]), 0);
    assert_eq!(
        vecq(&[
            "broken.rs",
            "--source-info",
            "-q",
            // Start/end only: the ERROR element is the file's only element, so
            // under R3-4 (lossless) it also owns the file's final newline as
            // `trailing` — the RFC's literal `{start, end}` cannot hold both.
            "[.. | objects | select(.attributes.kind? == \"ERROR\") | .source | {start, end}] | unique",
            "--compact",
        ]),
        serde_json::json!([{"start": 0, "end": 12}])
    );
}

// ---- errors and keys a consumer acts on -------------------------

/// The two fixtures (Python parse errors). Both fail on line 3:
/// `indent.py` where the indented block is missing (`return` at byte 30),
/// `late.py` at the `:` of `def broken(:` (byte 23).
const PY_ERRORS: [(&str, &str); 2] = [
    ("indent.py", "class C:\n    def f(self):\n    return 1\n"),
    ("late.py", "x = 1\ny = 2\ndef broken(:\n    pass\n"),
];

/// REGRESSION: every Python parse error said `line 0` in file
/// `<string>`, with the only real location a byte offset inside the message.
/// That message is the one signal an agent gets after writing a broken Python
/// file (ivaldi's SyntaxGuard), and it named no line it could act on. `line`
/// is the 1-indexed line of the error, like every other vecq line; the byte
/// offset is not part of the message; there is no made-up file name.
#[tokio::test]
async fn python_parse_error_names_the_line_not_a_byte_offset() {
    for (name, source) in PY_ERRORS {
        let err = vecq::parse_file(source, FileType::Python)
            .await
            .expect_err(name);
        match &err {
            vecq::VecqError::ParseError { line, .. } => {
                assert_eq!(*line, 3, "{name}: {err}")
            }
            other => panic!("{name}: not a ParseError: {other}"),
        }
        let shown = err.to_string();
        assert!(!shown.contains("byte offset"), "{name}: {shown}");
        assert!(!shown.contains("<string>"), "{name}: {shown}");
    }
}

/// The same through the CLI, which has the path: the error names the file the
/// caller gave, and line 3.
#[test]
fn python_parse_error_from_the_cli_names_the_file_and_line() {
    let dir = tempfile::tempdir().unwrap();
    for (name, source) in PY_ERRORS {
        fs::write(dir.path().join(name), source).unwrap();
        let out = Command::new(common::vecq_bin())
            .args([name, "-q", ".", "--compact"])
            .current_dir(dir.path())
            .output()
            .expect("vecq runs");
        assert!(!out.status.success(), "{name} parsed");
        let err = String::from_utf8_lossy(&out.stderr);
        assert!(err.contains(name), "{name}: no file in {err}");
        assert!(err.contains("line 3"), "{name}: no line 3 in {err}");
        assert!(!err.contains("line 0"), "{name}: {err}");
        assert!(!err.contains("byte offset"), "{name}: {err}");
        assert!(!err.contains("<string>"), "{name}: {err}");
    }
}

/// One snippet per language that emits variable elements:
/// Python, Bash, C, C++, JavaScript and Go.
const VARIABLE_SNIPPETS: [(&str, &str); 6] = [
    ("v.py", "X = 1\n"),
    ("v.sh", "X=1\n"),
    ("v.c", "int x = 1;\n"),
    ("v.cpp", "int x = 1;\n"),
    ("v.js", "var x = 1;\n"),
    ("v.go", "package main\n\nvar x = 1\n"),
];

/// REGRESSION: one element kind, two keys — Python, C, C++ and
/// Go listed variables under `variable`, Bash and JavaScript under
/// `variables`. A `-R` query over a mixed tree with `.variables[]` silently
/// missed Python. Every category key is plural; so is this one, everywhere.
#[tokio::test]
async fn variables_are_listed_under_one_plural_key_in_every_language() {
    for (name, source) in VARIABLE_SNIPPETS {
        let ft = vecq::detect_file_type(name);
        let doc = vecq::parse_file(source, ft)
            .await
            .unwrap_or_else(|e| panic!("{name}: {e}"));
        let json = vecq::convert_to_json(doc).unwrap();
        let keys: Vec<&String> = json.as_object().unwrap().keys().collect();
        assert!(
            json["variables"].as_array().is_some_and(|v| !v.is_empty()),
            "{name}: no `variables` — keys {keys:?}"
        );
        assert!(json.get("variable").is_none(), "{name}: keys {keys:?}");
    }
    // And over every fixture of every file type: no output carries the
    // singular key.
    for (path, ft) in fixtures() {
        let Ok(doc) = vecq::parse_file(&fs::read_to_string(&path).unwrap(), ft).await else {
            continue; // parse failures are the span contract's to report
        };
        let json = vecq::convert_to_json(doc).unwrap();
        assert!(
            json.get("variable").is_none(),
            "{}: emits `variable`",
            path.display()
        );
    }
}

/// The manual teaches the one key: no `.variable[]` or `.block[]` left in it
/// for an agent
/// to copy (`man --agent` and `docs/vecq/AGENT_MANUAL.md` are one text —
/// tier1_agent_manual).
#[test]
fn the_manual_does_not_teach_a_singular_key() {
    let manual = std::fs::read_to_string(
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("src/docs/man_agent.md"),
    )
    .unwrap();
    let bad: Vec<&str> = manual
        .lines()
        .filter(|l| l.contains(".variable[]") || l.contains(".block[]"))
        .collect();
    assert!(bad.is_empty(), "{bad:#?}");
}

/// REGRESSION:
/// the same singular/plural split as `variable`/`variables` — Python and
/// JavaScript listed grammar blocks under `block`, the text schema under
/// `blocks`.
#[tokio::test]
async fn blocks_are_listed_under_one_plural_key() {
    for (name, source) in [("b.py", "print(1)\n"), ("b.js", "foo();\n")] {
        let doc = vecq::parse_file(source, vecq::detect_file_type(name))
            .await
            .unwrap_or_else(|e| panic!("{name}: {e}"));
        let json = vecq::convert_to_json(doc).unwrap();
        let keys: Vec<&String> = json.as_object().unwrap().keys().collect();
        assert!(
            json["blocks"].as_array().is_some_and(|v| !v.is_empty()),
            "{name}: no `blocks` — keys {keys:?}"
        );
        assert!(json.get("block").is_none(), "{name}: keys {keys:?}");
    }
}

/// The rule behind both renames, so the next one cannot happen: no category
/// key exists in both a singular and a plural spelling — across every schema's
/// mappings AND every key any fixture actually emits (an unmapped element type
/// falls back to its singular type name, which is how Go got `variable` and
/// C++/CUDA `struct`/`macro` beside C's `structs`/`macros`).
#[tokio::test]
async fn no_category_key_has_a_singular_and_a_plural_spelling() {
    let mut keys = std::collections::BTreeSet::new();
    for schema in vecq::SchemaRegistry::new().list_schemas() {
        keys.extend(schema.element_mappings.values().cloned());
    }
    for (path, ft) in fixtures() {
        if let Ok(doc) = vecq::parse_file(&fs::read_to_string(&path).unwrap(), ft).await {
            let json = vecq::convert_to_json(doc).unwrap();
            // Lists only: `.module` is the file's one scope OBJECT, singular like `metadata` — not a category of elements.
            keys.extend(
                json.as_object()
                    .unwrap()
                    .iter()
                    .filter(|(_, v)| v.is_array())
                    .map(|(k, _)| k.clone()),
            );
        }
    }
    let both: Vec<_> = keys
        .iter()
        .filter(|k| keys.contains(&format!("{k}s")))
        .collect();
    assert!(both.is_empty(), "singular and plural both in use: {both:?}");
}

/// REGRESSION: Python's `value` printed
/// Rust's spelling of a bool — `d = True` reported `"true"`, which is not
/// Python and not what the file says. An agent copying it back wrote a
/// NameError.
#[tokio::test]
async fn python_bool_values_are_spelled_as_python() {
    let doc = vecq::parse_file("t = True\nf = False\n", FileType::Python)
        .await
        .unwrap();
    let json = vecq::convert_to_json(doc).unwrap();
    let vars = json
        .get("variables")
        .and_then(|v| v.as_array())
        .expect("variables");
    let values: Vec<&str> = vars
        .iter()
        .map(|v| v["attributes"]["value"].as_str().unwrap())
        .collect();
    assert_eq!(values, ["True", "False"]);
}
