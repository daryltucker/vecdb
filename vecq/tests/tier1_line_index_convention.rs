//! Every language reports line numbers in the SAME, 1-INDEXED convention.
//!
//! # Why this exists
//!
//! v1.1.0 shipped `line_start` 0-indexed for Python and Go, 1-indexed for Rust,
//! and **both at once** for JavaScript (elements 1-indexed, usage detection
//! 0-indexed). Same field name, same type, same query — different meaning per
//! language, with nothing in the output to distinguish them. A consumer that
//! edits `line_start..=line_end` therefore corrupts Python and Go files while
//! working correctly on Rust: the replacement eats the line *before* the
//! function and leaves its last line behind, which a user sees as duplicated
//! code.
//!
//! ```text
//! $ printf 'def first():\n    return 1\n' > edge.py
//! $ vecq edge.py -q '.functions[] | {name, line_start}'
//! {"line_start":0,"name":"first"}     # the def is on line 1
//! ```
//!
//! # Why nothing caught it
//!
//! This is the part worth remembering. A test asserting exactly this invariant
//! already existed and could never fail:
//!
//! ```text
//! vecq/tests/property_core_traits.rs:615
//!     prop_assert!(line_start > 0, "Line numbers must be 1-based");
//! ```
//!
//! Its input comes from `arbitrary_document_element()`, which *generates*
//! `line_start` in `1usize..100usize` and hands it straight to
//! `DocumentElement::new`. The fixture guarantees the property under test, and
//! no parser is ever invoked — it is a serde round-trip test wearing a
//! correctness test's error message, which is worse than no test at all
//! because grepping for "1-based" makes the invariant look covered.
//!
//! The other candidates were equally blind:
//!
//! - `parser_compliance.rs` asserts monotonicity and `line_end >= line_start`.
//!   **Both hold identically under 0- and 1-indexing**, so an off-by-one is
//!   invariant under every check it makes.
//! - `tier1_language_fidelity.rs` reads `line_start` *only to print it* in a
//!   failure message. It validates content, never spans.
//! - rust, go, python, javascript, c, cpp, cuda, bash and markdown have
//!   **zero** line assertions in their unit tests.
//!
//! So: not one test in this repo had ever compared a reported line number
//! against an actual file. That is the hole these tests close.
//!
//! # The contract
//!
//! > `line_start` is 1-indexed. The text at that line of the file is the first
//! > line of the element's content.
//!
//! 1-indexed because Rust and Markdown already were and are depended on, and
//! because it matches `sed -n`, `grep -n`, and every editor gutter. Line 0 does
//! not exist.
//!
//! `vecdb-common::lines::line_number_from_offset` is the canonical
//! implementation and has always been correct. Python broke by *forking* it
//! (`vecq/src/parsers/python/mod.rs`) and dropping the `+ 1`. Prefer the shared
//! helper over a local reimplementation.
//!
//! # Layout
//!
//! One `#[tokio::test]` per language, so a regression names the language in the
//! test name rather than burying it in a list, plus three cross-cutting sweeps
//! over every fixture in the repo.

use std::fs;
use std::path::PathBuf;
use vecq::parsers::{available_parsers, create_parser};
use vecq::types::{DocumentElement, FileType};

/// A language, a snippet, and the 1-indexed line its symbol is really on.
///
/// The snippets carry deliberate leading padding — imports, package
/// declarations, blank lines — because an off-by-one is invisible when the
/// symbol is on line 1 of a file that starts with the symbol.
struct Case {
    file_type: FileType,
    /// Name of the symbol the parser is expected to report.
    symbol: &'static str,
    /// Where that symbol actually is, counting from 1.
    expected_line: usize,
    source: &'static str,
}

const RUST: Case = Case {
    file_type: FileType::Rust,
    symbol: "alpha",
    expected_line: 3,
    source: "use std::fmt;\n\npub fn alpha(a: u32) -> u32 {\n    a\n}\n",
};

const PYTHON: Case = Case {
    file_type: FileType::Python,
    symbol: "alpha",
    expected_line: 4,
    source: "import os\n\n\ndef alpha(a, b):\n    return a + b\n",
};

const GO: Case = Case {
    file_type: FileType::Go,
    symbol: "Alpha",
    expected_line: 5,
    source: "package main\n\nimport \"fmt\"\n\nfunc Alpha(a int) int {\n\treturn a\n}\n",
};

const C: Case = Case {
    file_type: FileType::C,
    symbol: "alpha",
    expected_line: 3,
    source: "#include <stdio.h>\n\nint alpha(int a) {\n    return a;\n}\n",
};

const CPP: Case = Case {
    file_type: FileType::Cpp,
    symbol: "alpha",
    expected_line: 3,
    source: "#include <string>\n\nint alpha(int a) {\n    return a;\n}\n",
};

const CUDA: Case = Case {
    file_type: FileType::Cuda,
    symbol: "alpha",
    expected_line: 3,
    source: "#include <cuda.h>\n\n__global__ void alpha(int *a) {\n    *a = 1;\n}\n",
};

const BASH: Case = Case {
    file_type: FileType::Bash,
    symbol: "alpha",
    expected_line: 3,
    source: "#!/bin/bash\n\nalpha() {\n    echo hi\n}\n",
};

/// Recursively find the first element with this name.
fn find<'a>(elements: &'a [DocumentElement], name: &str) -> Option<&'a DocumentElement> {
    for e in elements {
        if e.name.as_deref() == Some(name) {
            return Some(e);
        }
        if let Some(found) = find(&e.children, name) {
            return Some(found);
        }
    }
    None
}

/// Assert the case's symbol is reported at its real, 1-indexed line.
async fn assert_line_convention(case: &Case) {
    let parser = create_parser(case.file_type)
        .unwrap_or_else(|e| panic!("no parser for {:?}: {e}", case.file_type));

    let doc = parser
        .parse(case.source)
        .await
        .unwrap_or_else(|e| panic!("{:?} failed to parse its snippet: {e}", case.file_type));

    let element = find(&doc.elements, case.symbol).unwrap_or_else(|| {
        panic!(
            "{:?} reported no element named {:?}. Elements found: {:?}",
            case.file_type,
            case.symbol,
            doc.elements
                .iter()
                .map(|e| e.name.clone())
                .collect::<Vec<_>>()
        )
    });

    // Line 0 cannot exist. Checked separately so a 0-indexed parser fails with
    // "this is 0-indexed" rather than a confusing off-by-one line comparison.
    assert!(
        element.line_start >= 1,
        "{:?}: {} reports line_start = {}. Line numbers are 1-INDEXED; line 0 \
         does not exist. This parser is emitting a raw tree-sitter row (or a \
         raw newline count) without the `+ 1`.",
        case.file_type,
        case.symbol,
        element.line_start,
    );

    assert_eq!(
        element.line_start,
        case.expected_line,
        "{:?}: {} is on line {} of the file (1-indexed), but the parser reports \
         line_start = {}. Off by {}.\n\nSource:\n{}",
        case.file_type,
        case.symbol,
        case.expected_line,
        element.line_start,
        element.line_start as i64 - case.expected_line as i64,
        numbered(case.source),
    );

    // The strongest form of the contract, and the one an editing consumer
    // actually depends on: the text AT that line is the element's first line.
    let reported = case
        .source
        .lines()
        .nth(element.line_start - 1)
        .unwrap_or_else(|| {
            panic!(
                "{:?}: {} reports line_start = {}, past the end of a {}-line file",
                case.file_type,
                case.symbol,
                element.line_start,
                case.source.lines().count()
            )
        });

    let first_content_line = element
        .content
        .lines()
        .find(|l| !l.trim().is_empty())
        .unwrap_or("")
        .trim();

    assert!(
        !first_content_line.is_empty() && reported.trim() == first_content_line,
        "{:?}: {} claims to start at line {}, but that line is:\n    {:?}\n\
         while the element's first content line is:\n    {:?}\n\
         A consumer replacing line_start..=line_end would edit the wrong lines.",
        case.file_type,
        case.symbol,
        element.line_start,
        reported,
        first_content_line,
    );
}

fn numbered(source: &str) -> String {
    source
        .lines()
        .enumerate()
        .map(|(i, l)| format!("  {:>3} | {l}", i + 1))
        .collect::<Vec<_>>()
        .join("\n")
}

// ─────────────────────────────────────────────────────────────────────────
// One test per language. A regression names the language, not a list index.
// ─────────────────────────────────────────────────────────────────────────

#[tokio::test]
async fn rust_line_numbers_are_1_indexed() {
    assert_line_convention(&RUST).await;
}

#[tokio::test]
async fn python_line_numbers_are_1_indexed() {
    assert_line_convention(&PYTHON).await;
}

#[tokio::test]
async fn go_line_numbers_are_1_indexed() {
    assert_line_convention(&GO).await;
}

#[tokio::test]
async fn c_line_numbers_are_1_indexed() {
    assert_line_convention(&C).await;
}

#[tokio::test]
async fn cpp_line_numbers_are_1_indexed() {
    assert_line_convention(&CPP).await;
}

#[tokio::test]
async fn cuda_line_numbers_are_1_indexed() {
    assert_line_convention(&CUDA).await;
}

#[tokio::test]
async fn bash_line_numbers_are_1_indexed() {
    assert_line_convention(&BASH).await;
}

// ─────────────────────────────────────────────────────────────────────────
// Cross-cutting sweeps. These catch a language nobody wrote a case for.
// ─────────────────────────────────────────────────────────────────────────

/// The edge case that rules out every benign explanation.
///
/// "The span absorbs a preceding decorator/comment" cannot explain a symbol on
/// the first line of a file reporting 0. Nothing legitimately reports line 0.
#[tokio::test]
async fn a_symbol_on_the_first_line_never_reports_line_zero() {
    let cases = [
        (FileType::Python, "def first():\n    return 1\n"),
        (FileType::Rust, "pub fn first() -> u32 { 1 }\n"),
        (FileType::Bash, "first() {\n    echo hi\n}\n"),
    ];

    let mut failures = Vec::new();
    for (file_type, source) in cases {
        let Ok(parser) = create_parser(file_type) else {
            continue;
        };
        let Ok(doc) = parser.parse(source).await else {
            continue;
        };
        if let Some(e) = doc.elements.first() {
            if e.line_start == 0 {
                failures.push(format!(
                    "  {file_type:?}: first element reports line_start = 0 for a \
                     symbol on line 1"
                ));
            }
        }
    }

    assert!(
        failures.is_empty(),
        "\nLine 0 does not exist in a 1-indexed scheme:\n{}\n",
        failures.join("\n")
    );
}

/// No parser, on any fixture in the repo, may report line 0.
///
/// Deliberately weaker than the per-language cases and deliberately broader:
/// it needs no knowledge of what a fixture contains, so it covers languages
/// added later that nobody wrote a `Case` for.
#[tokio::test]
async fn no_parser_reports_line_zero_on_any_fixture() {
    let fixtures_root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures");
    let mut failures = Vec::new();
    let mut checked = 0usize;

    for file_type in available_parsers() {
        let Some(dir_name) = fixture_dir(file_type) else {
            continue;
        };
        let dir = fixtures_root.join(dir_name);
        let Ok(entries) = fs::read_dir(&dir) else {
            continue;
        };
        let Ok(parser) = create_parser(file_type) else {
            continue;
        };

        for entry in entries
            .filter_map(|e| e.ok())
            .filter(|e| e.path().is_file())
        {
            let path = entry.path();
            let Ok(source) = fs::read_to_string(&path) else {
                continue;
            };
            let Ok(doc) = parser.parse(&source).await else {
                continue;
            };
            checked += 1;
            collect_zero_lines(&doc.elements, file_type, &path, &mut failures);
        }
    }

    assert!(checked > 0, "no fixtures were checked");
    assert!(
        failures.is_empty(),
        "\n{} element(s) report line_start = 0. Line numbers are 1-indexed.\n{}\n",
        failures.len(),
        failures.join("\n")
    );
}

fn collect_zero_lines(
    elements: &[DocumentElement],
    file_type: FileType,
    path: &std::path::Path,
    failures: &mut Vec<String>,
) {
    for e in elements {
        if e.line_start == 0 {
            failures.push(format!(
                "  {file_type:?} {:?}: {:?} reports line_start = 0",
                path.file_name().unwrap_or_default(),
                e.name.as_deref().unwrap_or("<anonymous>")
            ));
        }
        collect_zero_lines(&e.children, file_type, path, failures);
    }
}

/// JSON nodes report the line they are actually on.
///
/// Converters are exempt from the verbatim-content contract, but NOT from the
/// line contract: a consumer edits JSON by line range just as it edits Rust.
///
/// `serde_json::Value` discards positions, so `JsonParser` reported a literal
/// `1, 1` for every node in every file. That is not an off-by-one — it aimed
/// every edit at line 1, destroying the opening brace and leaving the intended
/// target in place, which is what "the edit duplicated my code" looked like
/// from the outside. The line-zero sweep cannot catch it, because 1 is a legal
/// line number.
#[tokio::test]
async fn json_nodes_report_their_real_lines() {
    let source = "{\n  \"alpha\": 1,\n  \"beta\": {\n    \"gamma\": 3\n  },\n  \"delta\": [\n    10,\n    20\n  ]\n}\n";
    let parser = create_parser(FileType::Json).expect("json parser");
    let doc = parser.parse(source).await.expect("parse");

    // (name, the 1-indexed line it is really on)
    for (name, expected) in [("alpha", 2), ("gamma", 4), ("[0]", 7), ("[1]", 8)] {
        let element = find(&doc.elements, name)
            .unwrap_or_else(|| panic!("JSON parser reported no node named {name:?}"));
        assert_eq!(
            element.line_start,
            expected,
            "JSON {name:?} is on line {expected}, reported {}.\n\n{}",
            element.line_start,
            numbered(source),
        );
    }
}

/// The nesting that made positional span-matching unsafe.
///
/// `serde_json::Map` is a `BTreeMap` without the `preserve_order` feature, so
/// its iteration order is alphabetical while the source is in document order.
/// Zipping the two trees positionally looks correct on any file whose keys
/// happen to be sorted and silently mis-assigns spans on every other one — so
/// this fixture is deliberately in reverse-alphabetical order.
#[tokio::test]
async fn json_spans_survive_keys_that_are_not_in_alphabetical_order() {
    let source = "{\n  \"zulu\": 1,\n  \"alpha\": 2\n}\n";
    let parser = create_parser(FileType::Json).expect("json parser");
    let doc = parser.parse(source).await.expect("parse");

    let zulu = find(&doc.elements, "zulu").expect("zulu");
    let alpha = find(&doc.elements, "alpha").expect("alpha");

    assert_eq!(
        zulu.line_start, 2,
        "zulu is on line 2, reported {}",
        zulu.line_start
    );
    assert_eq!(
        alpha.line_start, 3,
        "alpha is on line 3, reported {} — spans were matched positionally \
         against serde_json's alphabetical ordering",
        alpha.line_start
    );
}

/// YAML nodes report their own line, not the document's.
///
/// Two separate defects lived here. The convention one: spans were 0-indexed,
/// so every element in every YAML file named a line no editor has. And the
/// precision one: `serde_yaml_ng` discards positions, so every element got the
/// *whole document* as its range — true, and useless to anything resolving a
/// node to lines.
#[tokio::test]
async fn yaml_nodes_report_their_own_lines_not_the_documents() {
    let source = "version: \"3\"\nservices:\n  qdrant:\n    image: qdrant/qdrant\n    ports:\n      - 6333\n      - 6334\n";
    let parser = create_parser(FileType::Yaml).expect("yaml parser");
    let doc = parser.parse(source).await.expect("parse");

    // (name, first line, last line)
    for (name, start, end) in [
        ("version", 1, 1),
        ("services", 2, 7),
        ("qdrant", 3, 7),
        ("image", 4, 4),
        ("ports", 5, 7),
        ("ports[0]", 6, 6),
        ("ports[1]", 7, 7),
    ] {
        let element = find(&doc.elements, name)
            .unwrap_or_else(|| panic!("YAML parser reported no node named {name:?}"));
        assert_eq!(
            (element.line_start, element.line_end),
            (start, end),
            "YAML {name:?} covers lines {start}..{end}, reported {}..{}.\n\n{}",
            element.line_start,
            element.line_end,
            numbered(source),
        );
    }
}

/// The document-scoped fallback must not come back by accident.
///
/// If every node shares one range, the parser has regressed to reporting the
/// file instead of the node — which is what it did before day 238.
#[tokio::test]
async fn yaml_nodes_do_not_all_share_one_range() {
    let source = "a: 1\nb: 2\nc: 3\n";
    let parser = create_parser(FileType::Yaml).expect("yaml parser");
    let doc = parser.parse(source).await.expect("parse");

    let starts: Vec<usize> = doc.elements.iter().map(|e| e.line_start).collect();
    assert_eq!(
        starts,
        vec![1, 2, 3],
        "three keys on three lines must report three distinct lines"
    );
}

/// Every language must have a case above.
///
/// Without this, adding a parser silently opts it out of the convention — which
/// is exactly how JavaScript ended up 1-indexed in one half of its own file and
/// 0-indexed in the other.
#[test]
fn every_extractor_language_has_a_line_index_case() {
    let covered = [
        FileType::Rust,
        FileType::Python,
        FileType::Go,
        FileType::C,
        FileType::Cpp,
        FileType::Cuda,
        FileType::Bash,
    ];

    let missing: Vec<_> = available_parsers()
        .into_iter()
        .filter(|ft| is_code_language(*ft))
        .filter(|ft| !covered.contains(ft))
        .collect();

    assert!(
        missing.is_empty(),
        "These languages have a parser but no line-index case in this file: {missing:?}\n\
         Add a `Case` and a `#[tokio::test]` for each. Do not delete them from \
         `is_code_language` to silence this."
    );
}

/// Languages whose parsers report spans of real source, and therefore must obey
/// the line convention. Converters (Markdown, HTML, JSON, TOML, YAML, Text)
/// restructure their input and are covered only by the line-zero sweep.
fn is_code_language(file_type: FileType) -> bool {
    matches!(
        file_type,
        FileType::Rust
            | FileType::Python
            | FileType::Go
            | FileType::C
            | FileType::Cpp
            | FileType::Cuda
            | FileType::Bash
    )
}

fn fixture_dir(file_type: FileType) -> Option<&'static str> {
    Some(match file_type {
        FileType::Markdown => "markdown",
        FileType::Rust => "rust",
        FileType::Python => "python",
        FileType::C => "c",
        FileType::Cpp => "cpp",
        FileType::Cuda => "cuda",
        FileType::Go => "go",
        FileType::Bash => "bash",
        FileType::Html => "html",
        FileType::Text => "text",
        FileType::Toml => "toml",
        FileType::Yaml => "yaml",
        FileType::Json => "json",
        _ => return None,
    })
}
