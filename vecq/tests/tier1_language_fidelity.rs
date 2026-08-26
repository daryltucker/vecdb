//! The gate that decides whether we may claim to support a language.
//!
//! Every other compliance test in this repo checks that a parser *exists* and
//! *does not crash*: `parser_compliance.rs` asserts the fixture parses, that
//! `metadata.file_type` round-trips (which the parser sets itself), and that
//! `line_end >= line_start`. `vecdb-core/tests/parser_compliance.rs` was weakened
//! further, to `assert!(!result.is_empty())` — with a comment saying the stronger
//! check had failed.
//!
//! None of that says the parser extracted the code. So for a long time it did
//! not, and nothing went red:
//!
//! ```text
//! vecq/src/parsers/python/visitor.rs:71   format!("def {}(...)", func_def.name)
//! vecq/src/parsers/go.rs:135              format!("func {}(...)", name)
//! ```
//!
//! Python and Go emitted a *synthesised label* as `element.content`. That label
//! is what `vecq_adapter` embeds, so every Python and Go function in every
//! collection was indexed as the fourteen bytes `def alpha(...)` — no body, no
//! docstring. Semantic search over those languages ranked noise. Worse, the
//! chunk ID is a UUIDv5 over the content hash, so the label being constant meant
//! editing a function could never change its ID: re-ingest was a permanent no-op
//! and the payload stayed frozen at whatever the first ingest saw.
//!
//! Both failures are downstream of one missing assertion, the one below:
//!
//! > **A code parser reports code that is actually in the file.**
//!
//! Content is compared verbatim against the source. That is deliberately the
//! strictest honest contract for an *extractor* — it cannot be satisfied by
//! summarising, labelling, or reformatting, which is exactly how this bug got
//! in. Parsers that legitimately transform their input (Markdown, HTML, JSON,
//! TOML, plain text) are converters rather than extractors and are listed as
//! such below; the match is exhaustive so a newly added file type cannot join
//! either group silently.

use std::fs;
use std::path::{Path, PathBuf};
use vecq::parsers::{available_parsers, create_parser};
use vecq::types::{DocumentElement, FileType};

/// How a file type's parser is allowed to relate to its input.
#[derive(Debug, Clone, Copy, PartialEq)]
enum Contract {
    /// Reports spans of the source verbatim. Every language we claim to support.
    Extractor,
    /// Restructures its input into something else; content need not be a span.
    Converter,
}

/// Exhaustive on purpose: a new `FileType` must state which contract it meets
/// before it can ship, rather than defaulting into the unchecked group.
fn contract_for(file_type: FileType) -> Contract {
    match file_type {
        FileType::Rust
        | FileType::Python
        | FileType::Go
        | FileType::C
        | FileType::Cpp
        | FileType::Cuda
        | FileType::Bash => Contract::Extractor,

        FileType::Markdown
        | FileType::Html
        | FileType::Json
        | FileType::Toml
        | FileType::Yaml
        | FileType::Text => Contract::Converter,

        other => panic!(
            "File type {other:?} has no fidelity contract. Decide whether its parser \
             extracts verbatim source spans (Extractor) or restructures its input \
             (Converter) and record it in contract_for(). Do not add it to Converter \
             to silence this — a language claimed as supported must be an Extractor."
        ),
    }
}

/// A parser is free to skip material, but whatever it does report must be real.
fn check_verbatim(
    element: &DocumentElement,
    source: &str,
    path: &Path,
    file_type: FileType,
    failures: &mut Vec<String>,
) {
    let content = element.content.trim();

    if !content.is_empty() && !source.contains(content) {
        let name = element.name.as_deref().unwrap_or("<anonymous>");
        failures.push(format!(
            "  {:?} {} {:?} (lines {}..{}) reported content that does not appear in \
             the file:\n      reported: {:?}\n    This is synthesised text, not an \
             extracted span. It is what gets embedded, so the real code never \
             reaches the index.",
            element.element_type,
            name,
            path.file_name().unwrap_or_default(),
            element.line_start,
            element.line_end,
            truncate(&element.content, 120),
        ));
        // Don't recurse: children of a synthesised element are not meaningful.
        return;
    }

    let _ = file_type;
    for child in &element.children {
        check_verbatim(child, source, path, file_type, failures);
    }
}

fn truncate(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        s.to_string()
    } else {
        let head: String = s.chars().take(max).collect();
        format!("{head}… ({} bytes total)", s.len())
    }
}

/// Every language we claim to support must report verbatim source spans.
#[tokio::test]
async fn extractors_report_verbatim_source_spans() {
    let fixtures_root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures");
    let mut failures: Vec<String> = Vec::new();
    let mut checked = 0usize;

    for file_type in available_parsers() {
        if contract_for(file_type) != Contract::Extractor {
            continue;
        }

        let dir = fixtures_root.join(fixture_dir(file_type));
        let entries = fs::read_dir(&dir).unwrap_or_else(|e| {
            panic!("failed to read fixtures for {file_type:?} at {dir:?}: {e}")
        });

        let parser = create_parser(file_type)
            .unwrap_or_else(|e| panic!("failed to create parser for {file_type:?}: {e}"));

        for entry in entries
            .filter_map(|e| e.ok())
            .filter(|e| e.path().is_file())
        {
            let path = entry.path();
            let Ok(source) = fs::read_to_string(&path) else {
                continue;
            };

            let doc = parser
                .parse(&source)
                .await
                .unwrap_or_else(|e| panic!("{file_type:?} failed to parse {path:?}: {e}"));

            let before = failures.len();
            for element in &doc.elements {
                check_verbatim(element, &source, &path, file_type, &mut failures);
            }
            if failures.len() > before {
                failures.insert(before, format!("{file_type:?}:"));
            }
            checked += 1;
        }
    }

    assert!(checked > 0, "no extractor fixtures were checked");

    assert!(
        failures.is_empty(),
        "\n{} language parser(s) reported content that is not in the source file.\n\
         Whatever a parser puts in `element.content` is what gets embedded — a \
         synthesised label means that language is not actually indexed.\n\n{}\n",
        failures.iter().filter(|f| f.ends_with(':')).count(),
        failures.join("\n")
    );
}

fn fixture_dir(file_type: FileType) -> &'static str {
    match file_type {
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
        other => panic!("no fixture directory mapped for {other:?}"),
    }
}
