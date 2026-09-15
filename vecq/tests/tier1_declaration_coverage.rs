//! Declaration-kind coverage: does a parser find what is actually there?
//!
//! `parser_compliance.rs` asserts a fixture *parses* — file type matches, no
//! `Err`, `line_end >= line_start`. A parser returning zero of a language's
//! declaration kinds passes it. That is how `vecq` shipped v1.1.0 extracting
//! two of Rust's nine kinds, with `enums` emitted always-empty and `traits`
//! never emitted at all. See `BUG_RUST_DECLARATION_COVERAGE-2026-241` in the
//! docs repo.
//!
//! Two gates here, answering two different questions:
//!
//! 1. [`rust_extracts_every_declaration_kind`] and its per-language siblings —
//!    *given a source file containing every kind, is each one extracted?*
//!    Catches a missing walker arm.
//! 2. [`every_element_mapping_is_reachable`] — *is every declared mapping
//!    actually producible?* Catches the subtler half: a mapping that exists in
//!    the converter and that nothing ever constructs is dead configuration
//!    which reads, from the outside, as a supported feature.
//!
//! The second is the one that closes the class rather than the instance.

use std::collections::HashSet;
use std::fs;
use std::path::PathBuf;

use tokio::runtime::Runtime;
use vecq::converter::SchemaRegistry;
use vecq::parsers::{available_parsers, create_parser};
use vecq::types::{DocumentElement, ElementType, FileType};

/// Collect every `ElementType` produced by a parse, including nested children
/// (Rust `mod` bodies and C++ namespaces nest their declarations).
fn collect_types(elements: &[DocumentElement], out: &mut HashSet<ElementType>) {
    for element in elements {
        out.insert(element.element_type);
        collect_types(&element.children, out);
    }
}

fn parse_types(file_type: FileType, source: &str) -> HashSet<ElementType> {
    let parser = create_parser(file_type)
        .unwrap_or_else(|e| panic!("failed to create parser for {:?}: {}", file_type, e));
    let rt = Runtime::new().expect("runtime");
    let doc = rt
        .block_on(parser.parse(source))
        .unwrap_or_else(|e| panic!("{:?} fixture failed to parse: {}", file_type, e));

    let mut found = HashSet::new();
    collect_types(&doc.elements, &mut found);
    found
}

/// Assert every expected kind was produced, reporting *all* misses at once —
/// a one-at-a-time failure turns a nine-kind gap into nine debug cycles.
fn assert_kinds(file_type: FileType, source: &str, expected: &[ElementType]) {
    let found = parse_types(file_type, source);
    let missing: Vec<_> = expected.iter().filter(|k| !found.contains(k)).collect();

    let mut found_sorted: Vec<String> = found.iter().map(|k| format!("{:?}", k)).collect();
    found_sorted.sort();

    assert!(
        missing.is_empty(),
        "{:?}: {} of {} declaration kinds not extracted.\n  missing: {:?}\n  found:   {}",
        file_type,
        missing.len(),
        expected.len(),
        missing,
        found_sorted.join(", "),
    );
}

// ---------------------------------------------------------------------------
// Gate 1 — per-language declaration kinds
// ---------------------------------------------------------------------------

/// Every declaration kind a Rust file can contain. This is the regression test
/// for the day-241 report: before the fix only `Function` and `Struct` passed.
#[test]
fn rust_extracts_every_declaration_kind() {
    const SOURCE: &str = r#"
use std::collections::HashMap;

/// A documented enum.
pub enum Kind { A, B }

/// A documented trait.
pub trait Doer { fn go(&self); }

pub type Alias = u32;
pub const C: u32 = 1;
pub static S: u32 = 2;
pub union U { a: u32 }

macro_rules! m { () => {} }

/// A documented struct.
pub struct Thing;

impl Thing { fn method(&self) {} }

pub fn free_function() {}

pub mod inner {}
"#;

    assert_kinds(
        FileType::Rust,
        SOURCE,
        &[
            ElementType::Function,
            ElementType::Struct,
            ElementType::Enum,
            ElementType::Trait,
            ElementType::TypeAlias,
            ElementType::Constant,
            ElementType::Variable, // `static`
            ElementType::Union,
            ElementType::Macro,
            ElementType::Implementation,
            ElementType::Module,
            ElementType::Import,
        ],
    );
}

/// Doc-comments must survive attribute construction. The `struct_item` arm used
/// to write `docstring` through `element.attributes` and then overwrite the
/// whole field, dropping it — invisible because nothing asserted on it.
#[test]
fn rust_declarations_retain_docstrings() {
    const SOURCE: &str = r#"
/// Struct doc.
pub struct Thing;

/// Enum doc.
pub enum Kind { A }

/// Trait doc.
pub trait Doer {}
"#;

    let parser = create_parser(FileType::Rust).expect("parser");
    let rt = Runtime::new().expect("runtime");
    let doc = rt.block_on(parser.parse(SOURCE)).expect("parse");

    for kind in [ElementType::Struct, ElementType::Enum, ElementType::Trait] {
        let element = doc
            .elements
            .iter()
            .find(|e| e.element_type == kind)
            .unwrap_or_else(|| panic!("{:?} not extracted at all", kind));

        let json = serde_json::to_value(&element.attributes).expect("serialize attributes");
        let docstring = json.get("docstring").and_then(|v| v.as_str());

        assert!(
            docstring.is_some_and(|d| d.contains("doc")),
            "{:?} lost its docstring; attributes were {}",
            kind,
            json,
        );
    }
}

#[test]
fn python_extracts_every_declaration_kind() {
    const SOURCE: &str = r#"
import os

@decorator
def free_function():
    pass

class Thing:
    def method(self):
        pass
"#;

    assert_kinds(
        FileType::Python,
        SOURCE,
        &[
            ElementType::Function,
            ElementType::Class,
            ElementType::Import,
        ],
    );

    // Decorators are deliberately *not* their own element — they are an
    // attribute of the thing they decorate. Assert that, so the modelling
    // decision is pinned rather than merely absent.
    let parser = create_parser(FileType::Python).expect("parser");
    let rt = Runtime::new().expect("runtime");
    let doc = rt.block_on(parser.parse(SOURCE)).expect("parse");
    let func = doc
        .elements
        .iter()
        .find(|e| e.element_type == ElementType::Function)
        .expect("function not extracted");
    let json = serde_json::to_value(&func.attributes).expect("serialize");

    assert_eq!(
        json.get("decorators")
            .and_then(|v| v.as_array())
            .map(|a| a.len()),
        Some(1),
        "decorator not recorded as a function attribute; attributes were {}",
        json,
    );
}

#[test]
fn go_extracts_every_declaration_kind() {
    const SOURCE: &str = r#"
package main

import "fmt"

type Thing struct { A int }

type Doer interface { Go() }

func FreeFunction() {}
"#;

    assert_kinds(
        FileType::Go,
        SOURCE,
        &[
            ElementType::Function,
            ElementType::Struct,
            ElementType::Interface,
            ElementType::Package,
            ElementType::Import,
        ],
    );
}

// ---------------------------------------------------------------------------
// Gate 2 — no dead mappings
// ---------------------------------------------------------------------------

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

/// Every `with_element_mapping(ElementType::X, "y")` must be reachable: some
/// fixture for that language must actually produce an `X`.
///
/// A mapping nothing constructs is indistinguishable, from the caller's side,
/// from a language that happens to contain none of that kind. `enums` was
/// emitted-and-always-empty and `traits` was mapped but never emitted, for the
/// entire life of the Rust parser, because no test asserted this.
#[test]
fn every_element_mapping_is_reachable() {
    let registry = SchemaRegistry::new();
    let fixtures_root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures");
    let rt = Runtime::new().expect("runtime");

    let mut failures: Vec<String> = Vec::new();

    for file_type in available_parsers() {
        let schema = match registry.get_schema(file_type) {
            Ok(s) => s,
            Err(_) => continue, // no schema registered; not this test's concern
        };
        let Some(dir_name) = fixture_dir(file_type) else {
            continue;
        };

        let dir = fixtures_root.join(dir_name);
        if !dir.is_dir() {
            continue; // parser_compliance.rs owns the missing-fixture failure
        }

        let parser = match create_parser(file_type) {
            Ok(p) => p,
            Err(_) => continue,
        };

        // Union of everything the fixtures for this language produce.
        let mut produced = HashSet::new();
        for entry in fs::read_dir(&dir).expect("read fixture dir").flatten() {
            let path = entry.path();
            if !path.is_file() {
                continue;
            }
            let Ok(content) = fs::read_to_string(&path) else {
                continue; // binary fixture
            };
            if let Ok(doc) = rt.block_on(parser.parse(&content)) {
                collect_types(&doc.elements, &mut produced);
            }
        }

        let mut unreachable: Vec<_> = schema
            .element_mappings
            .iter()
            .filter(|(element_type, _)| !produced.contains(element_type))
            .map(|(element_type, field)| format!("{:?} -> \"{}\"", element_type, field))
            .collect();
        unreachable.sort();

        if !unreachable.is_empty() {
            failures.push(format!(
                "  {:?} ({} fixture(s)): {}",
                file_type,
                dir_name,
                unreachable.join(", ")
            ));
        }
    }

    failures.sort();
    assert!(
        failures.is_empty(),
        "element mappings declared but never produced by any fixture.\n\
         Each is dead configuration that presents to callers as a supported \
         category which is simply always empty.\n\
         Either construct it in the parser, or drop the mapping, or add a \
         fixture that contains one:\n{}",
        failures.join("\n"),
    );
}
