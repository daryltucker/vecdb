//! A malformed query must not return vecq's own source code.
//!
//! # Why this exists
//!
//! `compile_jaq_filter` builds the program it hands to jaq by concatenating
//! `REGEX_PRELUDE`, every `stdlib` normalizer and renderer, `auto.jq`, and the
//! user's query. When jaq fails to parse, its error carries a
//! `File { code, .. }` describing the program it was parsing — which is that
//! whole concatenation. `query.rs` used to `{:?}`-format it straight into
//! `VecqError::QueryError.message`.
//!
//! The cost of one typo, measured on v1.1.0:
//!
//! | surface | bytes |
//! |---|---|
//! | `vecq` CLI | 42,144 |
//! | through MCP, once JSON-escaped | ~104,000 |
//! | actual diagnostic within that | ~40 |
//!
//! That is roughly 26,000 tokens returned for a single-line mistake, and it
//! affects every consumer of `query_json` — `vecdb-cli`, the `vecdb-server` MCP
//! tools, and Ivaldi, which had to add `ivaldi-core/src/util/vecq_error.rs` to
//! sanitise its own dependency's errors.
//!
//! A tool that answers a typo by consuming the caller's remaining context is
//! worse than one that just says "no".

use vecq::query::{JqQueryEngine, QueryEngine};

/// Something that parses as neither jq nor anything else.
const MALFORMED: &str = "## Decisions Made";

fn error_message(query: &str) -> String {
    let engine = JqQueryEngine::new_hermetic();
    let json = serde_json::json!({"headers": []});
    match engine.execute_query(&json, query) {
        Ok(_) => panic!("{query:?} was expected to fail to compile, but succeeded"),
        Err(e) => e.to_string(),
    }
}

/// The specific leak: none of the injected prelude may appear in the message.
///
/// Asserted on distinctive prelude tokens rather than on length alone, so this
/// still fails if someone reintroduces the program text but caps it short.
#[test]
fn a_query_error_never_contains_the_injected_prelude() {
    let msg = error_message(MALFORMED);

    for marker in [
        "_native_test",
        "_native_capture",
        "def test($r)",
        "def gsub(",
    ] {
        assert!(
            !msg.contains(marker),
            "query error leaked vecq's own prelude (found {marker:?}).\n\
             Do not Debug-format jaq's `File {{ code, .. }}` — `code` is the whole \
             program, prelude included.\n\nMessage was:\n{msg}"
        );
    }
}

/// Total size, as the blunt backstop.
///
/// 2 KB is far above the ~90 bytes a real diagnostic needs and far below the
/// 42 KB regression this guards. Anything in between is still a bug worth
/// looking at, but this is the line that must not be crossed.
#[test]
fn a_query_error_stays_small_enough_to_read() {
    let msg = error_message(MALFORMED);
    assert!(
        msg.len() < 2048,
        "query error is {} bytes; a one-line typo must not return a document.\n\
         Message began:\n{}",
        msg.len(),
        &msg[..msg.len().min(300)]
    );
}

/// Small is not enough — it must still say what went wrong, and echo what was
/// asked. Truncating to nothing would pass the size check and help no one.
#[test]
fn a_query_error_still_identifies_the_problem_and_the_query() {
    let msg = error_message(MALFORMED);

    assert!(
        msg.contains(MALFORMED),
        "the error must echo the offending query so the caller knows which one \
         failed.\nMessage was:\n{msg}"
    );
    assert!(
        msg.contains("Parse") || msg.contains("parse"),
        "the error must retain jaq's diagnostic, not just the fact of failure.\n\
         Message was:\n{msg}"
    );
}
