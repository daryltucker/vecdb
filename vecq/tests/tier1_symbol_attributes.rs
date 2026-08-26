//! `signature` and visibility are reported for every language, not just Rust.
//!
//! # Why this exists
//!
//! `attributes.signature` and `attributes.visibility` were populated for Rust
//! and `null` for Python and Go:
//!
//! ```text
//! vecq t.rs -q '.functions[] | {sig: .attributes.signature}'
//! {"sig":"pub fn alpha(a: u32) -> u32"}
//! vecq t.py -q '.functions[] | {sig: .attributes.signature}'
//! {"sig":null}
//! ```
//!
//! Not a crash, and not wrong — just absent, which is worse in one specific
//! way: a query keyed on a symbol's shape returns a confident empty answer for
//! two of the three languages rather than an error. The data was already
//! there in both parsers (Python had parameter names, annotations and the
//! return annotation; Go's body is a named field, so the signature is
//! everything preceding it) and simply was not assembled.
//!
//! # Two fields, on purpose
//!
//! `visibility` is the **language's own word**. Rust reports `pub`,
//! `pub(crate)` or `private`, and `pub(crate)` is a genuine distinction that
//! collapsing to a boolean would destroy. Five documented examples query
//! `select(.visibility == "pub")`, and they keep working.
//!
//! `is_public` is the **portable** predicate, uniform across every language, so
//! a cross-language query does not have to know that Rust says `pub`, Go says
//! `public` and Python has no keyword at all. Adding it rather than unifying
//! `visibility` is what keeps this non-breaking.
//!
//! Go's rule is not a convention: an identifier beginning with an uppercase
//! letter is exported from its package and one that does not is unreachable
//! outside it. Python's leading underscore *is* a convention, but a universal
//! one — with the deliberate exception of `__dunder__`, which is public.

use vecq::parsers::create_parser;
use vecq::types::{DocumentElement, FileType};

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

async fn attribute(
    file_type: FileType,
    source: &str,
    symbol: &str,
    key: &str,
) -> serde_json::Value {
    let parser = create_parser(file_type).expect("parser");
    let doc = parser.parse(source).await.expect("parse");
    let element = find(&doc.elements, symbol)
        .unwrap_or_else(|| panic!("{file_type:?} reported no symbol named {symbol:?}"));
    serde_json::to_value(&element.attributes)
        .ok()
        .and_then(|v| v.get(key).cloned())
        .unwrap_or(serde_json::Value::Null)
}

const RUST_SRC: &str = "fn hidden() {}\npub fn shown() {}\npub(crate) fn scoped() {}\n";
const PY_SRC: &str =
    "def alpha(a: int, b) -> str:\n    return \"x\"\n\nasync def _fetch(u: str):\n    pass\n";
const GO_SRC: &str = "package main\n\nfunc Alpha(a int) int {\n\treturn a\n}\n\nfunc beta() {}\n";

#[tokio::test]
async fn python_reports_a_signature() {
    assert_eq!(
        attribute(FileType::Python, PY_SRC, "alpha", "signature").await,
        serde_json::json!("def alpha(a: int, b) -> str"),
    );
}

/// `async` is part of the shape, not a modifier to be dropped.
#[tokio::test]
async fn python_signatures_keep_async() {
    assert_eq!(
        attribute(FileType::Python, PY_SRC, "_fetch", "signature").await,
        serde_json::json!("async def _fetch(u: str)"),
    );
}

#[tokio::test]
async fn go_reports_a_signature() {
    assert_eq!(
        attribute(FileType::Go, GO_SRC, "Alpha", "signature").await,
        serde_json::json!("func Alpha(a int) int"),
    );
}

/// A signature stops at the body. A declaration that drags its whole body along
/// is not a signature — it is the element's `content` under another name.
#[tokio::test]
async fn a_go_signature_excludes_the_body() {
    let sig = attribute(FileType::Go, GO_SRC, "Alpha", "signature").await;
    let sig = sig.as_str().unwrap_or_default();
    assert!(
        !sig.contains('{') && !sig.contains("return"),
        "Go signature swallowed the body: {sig:?}"
    );
}

/// Go's exported/unexported rule, which is a language rule.
#[tokio::test]
async fn go_visibility_follows_the_export_rule() {
    assert_eq!(
        attribute(FileType::Go, GO_SRC, "Alpha", "visibility").await,
        serde_json::json!("public"),
    );
    assert_eq!(
        attribute(FileType::Go, GO_SRC, "beta", "visibility").await,
        serde_json::json!("private"),
    );
}

#[tokio::test]
async fn python_visibility_follows_the_underscore_convention() {
    assert_eq!(
        attribute(FileType::Python, PY_SRC, "alpha", "visibility").await,
        serde_json::json!("public"),
    );
    assert_eq!(
        attribute(FileType::Python, PY_SRC, "_fetch", "visibility").await,
        serde_json::json!("private"),
    );
}

/// `__init__` is part of a class's public surface. Treating a dunder as private
/// because it starts with an underscore would hide most of Python's API.
#[tokio::test]
async fn python_dunders_are_public() {
    let src = "class Thing:\n    def __init__(self):\n        pass\n";
    assert_eq!(
        attribute(FileType::Python, src, "__init__", "visibility").await,
        serde_json::json!("public"),
    );
}

/// `visibility` stays the language's own word — this is what the documented
/// `select(.visibility == "pub")` examples depend on.
#[tokio::test]
async fn rust_visibility_is_still_the_literal_modifier() {
    assert_eq!(
        attribute(FileType::Rust, RUST_SRC, "shown", "visibility").await,
        serde_json::json!("pub"),
    );
    assert_eq!(
        attribute(FileType::Rust, RUST_SRC, "scoped", "visibility").await,
        serde_json::json!("pub(crate)"),
        "pub(crate) must not be collapsed to pub — the distinction is real"
    );
}

/// The portable predicate: same type, same meaning, every language.
#[tokio::test]
async fn is_public_is_a_bool_and_agrees_across_languages() {
    let cases = [
        (FileType::Rust, RUST_SRC, "shown", true),
        (FileType::Rust, RUST_SRC, "hidden", false),
        (FileType::Rust, RUST_SRC, "scoped", true),
        (FileType::Python, PY_SRC, "alpha", true),
        (FileType::Python, PY_SRC, "_fetch", false),
        (FileType::Go, GO_SRC, "Alpha", true),
        (FileType::Go, GO_SRC, "beta", false),
    ];

    for (file_type, src, symbol, expected) in cases {
        let got = attribute(file_type, src, symbol, "is_public").await;
        assert_eq!(
            got,
            serde_json::Value::Bool(expected),
            "{file_type:?} {symbol}: is_public must be the bool {expected}, got {got:?}. \
             A portable query cannot special-case each language's vocabulary."
        );
    }
}
