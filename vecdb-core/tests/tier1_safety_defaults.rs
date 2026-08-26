//! Regression tests for defaults whose wrong value costs something irreversible.
//!
//! Most configuration mistakes cost time. The ones pinned here cost content,
//! and the failure is silent — nothing in a search result says "the tail of this
//! chunk was dropped before it was embedded".

use vecdb_core::config::IngestionConfig;

/// Regression: embed-time truncation was unconditionally on.
#[test]
fn embed_truncation_is_off_by_default() {
    assert!(
        !IngestionConfig::default().allow_embed_truncation,
        "silently cutting oversized chunks at embed time must be opt-in"
    );
    let parsed: IngestionConfig =
        toml::from_str("target_chunk_size = 512").expect("minimal ingestion config should parse");
    assert!(
        !parsed.allow_embed_truncation,
        "omitting the key must not enable truncation"
    );
}

// ─────────────────────────────────────────────────────────────────────────────
// Embedding-space comparison: the Matryoshka narrowing contract
// ─────────────────────────────────────────────────────────────────────────────

use vecdb_core::types::{compare_spaces, Compatibility, ModelIdentity};

fn identity(name: &str, digest: &str) -> ModelIdentity {
    ModelIdentity {
        name: name.to_string(),
        digest: Some(digest.to_string()),
        architecture: Some("qwen3".to_string()),
        family: Some("qwen3".to_string()),
        parameter_size: Some("4B".to_string()),
        quantization_level: Some("Q8_0".to_string()),
        context_length: Some(8192),
        embedding_length: Some(2560),
    }
}

/// Same weights at two widths must stay `Identical`, and the ordering inside
/// `compare_spaces` is what makes that true.
///
/// This is load-bearing, not incidental: the plan for `docs` is one model
/// (`qwen3-embedding:4b-q8_0`) writing native 2560-dim in one place and
/// MRL-truncated 1024-dim in another. If the dimension check were moved ahead of
/// the digest check — which looks like a tightening — that configuration stops
/// being expressible and MRL truncation dies with it.
///
/// It is safe precisely because width is *resolved* rather than assumed:
/// `ensure_write_target` returns the collection's own dimension and the caller
/// embeds to it. The guard's job here is to permit the narrowing, not to police
/// a width the caller has not chosen yet.
#[test]
fn identical_weights_at_different_widths_stay_compatible() {
    let same = identity("qwen3-embedding:4b-q8_0", "sha256:abcdef0123456789");
    let report = compare_spaces(&same, Some(1024), &same.clone(), Some(2560));

    assert_eq!(
        report.tier,
        Compatibility::Identical,
        "MRL narrowing must remain permitted; got {:?} ({})",
        report.tier,
        report.reason
    );
    assert!(report.permits_write(false));
}

/// The other half of the contract: without digest equality there is no evidence
/// a width difference is a legitimate truncation, so it must be refused.
#[test]
fn different_weights_at_different_widths_are_incompatible() {
    let collection = identity("qwen3-embedding:4b-q8_0", "sha256:aaaa1111");
    let active = identity("qwen3-embedding:4b-q4_k_m", "sha256:bbbb2222");

    let report = compare_spaces(&collection, Some(1024), &active, Some(2560));

    assert_eq!(
        report.tier,
        Compatibility::Incompatible,
        "{}",
        report.reason
    );
    assert!(
        !report.permits_write(true),
        "no flag may override incompatible"
    );
    assert!(
        report.reason.contains("dimension mismatch"),
        "the diagnostic must name the real cause, got: {}",
        report.reason
    );
}

// ─────────────────────────────────────────────────────────────────────────────
// Chunking strategy: a name that resolves to nothing must be refused
// ─────────────────────────────────────────────────────────────────────────────

use vecdb_core::config::{validate_strategy, STRATEGIES};

fn strategy_error(strategy: &str) -> String {
    match validate_strategy(strategy) {
        Ok(()) => panic!("strategy {strategy:?} was accepted; it must be refused at load"),
        Err(e) => format!("{e:#}"),
    }
}

/// `Factory::get` fell through to `RecursiveChunker` for any unrecognised
/// strategy, so a typo silently changed how an entire corpus was chunked and
/// reported nothing.
#[test]
fn unknown_chunking_strategy_is_refused() {
    let err = strategy_error("recursiv");
    assert!(
        err.contains("not a known strategy") && err.contains("recursive"),
        "the error must name the bad value and list the valid ones; got: {err}"
    );
}

/// `code_aware` needs its own message rather than "unknown".
///
/// It was a documented option — `docs/CONFIG.md` carried a worked example — so
/// anyone hitting this had followed the docs. "Unknown strategy" would read as a
/// typo and send them hunting for the right spelling of something that no longer
/// exists; the real answer is that AST chunking is automatic.
#[test]
fn retired_code_aware_strategy_explains_itself() {
    let err = strategy_error("code_aware");
    assert!(
        err.contains("no longer a strategy"),
        "code_aware must be refused specifically, not as a generic typo; got: {err}"
    );
    assert!(
        err.contains("AST"),
        "the message must say AST chunking is automatic, or the reader will just \
         pick another strategy and still not get what they wanted; got: {err}"
    );
}

/// Every advertised strategy must actually be accepted — otherwise the error
/// message above sends people to a value that is itself rejected.
#[test]
fn every_listed_strategy_is_accepted() {
    for s in STRATEGIES {
        assert!(
            validate_strategy(s).is_ok(),
            "{s:?} is listed as valid in the error message but is rejected"
        );
    }
}
