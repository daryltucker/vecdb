//! JSON-with-comments and trailing commas.
//!
//! Ported from `vecdb-core/tests/tier1_json_comments.rs`, which tested
//! `vecdb_core::parsers::json::JsonParser` — a parser no binary ever
//! constructed. Every vecdb binary injects `VecqParserFactory`, so the capable
//! parser was unreachable and a `tsconfig.json` silently degraded to a single
//! unstructured text chunk while a green test said comments were handled.
//!
//! That is the "two implementations with different support levels" problem in
//! miniature: two JSON parsers, only one reachable, and the tests pointed at the
//! other one. The capability now lives in vecq, where the only JSON parser is,
//! and these assertions moved with it.
//!
//! JSONL and concatenated roots must keep working — the fallback is only allowed
//! to engage when strict parsing fails outright.

use vecq::parsers::create_parser;
use vecq::types::FileType;

async fn parse(content: &str) -> vecq::error::VecqResult<vecq::types::ParsedDocument> {
    create_parser(FileType::Json)
        .expect("JSON parser must exist")
        .parse(content)
        .await
}

#[tokio::test]
async fn handles_comments() {
    let doc = parse(
        r#"{
            "key": "value",
            // This is a comment
            "number": 123
        }"#,
    )
    .await
    .expect("JSONC must parse; tsconfig.json and .eslintrc.json are ordinary .json files");

    assert!(
        !doc.elements.is_empty(),
        "JSONC parsed but produced no elements — falling back to an empty document \
         is the same silent structure loss this test exists to prevent"
    );
}

#[tokio::test]
async fn handles_trailing_commas() {
    let doc = parse(
        r#"{
            "key": "value",
            "list": [1, 2, ],
        }"#,
    )
    .await
    .expect("trailing commas must parse via the JSON5 fallback");

    assert!(
        !doc.elements.is_empty(),
        "trailing-comma JSON produced no elements"
    );
}

#[tokio::test]
async fn handles_standard_json() {
    let doc = parse(r#"{"key": "value"}"#)
        .await
        .expect("standard JSON must parse");
    assert!(!doc.elements.is_empty());
}

/// The fallback must not swallow the streaming path.
///
/// JSON5 parses a single value. If it ran unconditionally, or on success of the
/// first item, a JSONL file would collapse from N records to one.
#[tokio::test]
async fn jsonl_still_yields_every_record() {
    let doc = parse("{\"a\": 1}\n{\"a\": 2}\n{\"a\": 3}\n")
        .await
        .expect("concatenated JSON roots must parse");

    let count = doc.elements.len();
    assert!(
        count >= 3,
        "expected at least one element per JSONL record, got {count}. The JSON5 \
         fallback parses a single value — if it engaged here, every record after \
         the first was dropped."
    );
}

/// Genuinely malformed input must still be an error, and must name the real
/// problem rather than a confusing JSON5 diagnostic.
#[tokio::test]
async fn malformed_json_is_still_an_error() {
    let err = parse(r#"{"key": "value" "#).await.unwrap_err();
    let msg = err.to_string();
    assert!(
        msg.contains("Failed to parse JSON"),
        "malformed JSON must report the strict parser's error; got: {msg}"
    );
}
