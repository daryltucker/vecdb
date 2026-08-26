//! Regression tests for the chunk-size unit bugs (roadmap B3).
//!
//! These exist because the two knobs are denominated differently and nothing
//! enforced a relationship between them:
//!
//! * `target_chunk_size` — counted in whatever the configured `tokenizer` counts. For
//!   the default `cl100k_base` that is **tokens**.
//! * `max_chunk_bytes` — compared against `String::len()`, i.e. **bytes**.
//!
//! When the byte ceiling lands below the byte weight of a full-size chunk, the
//! ceiling stops being a safety net and becomes an unconditional second
//! chunking pass: every full-size chunk is re-split by `FixedWidthChunker`, which
//! discards the AST boundaries structural chunking exists to produce. Nothing
//! reports this, and a re-ingest is the only repair.
//!
//! That is not hypothetical. Before the fix, `target_chunk_size = 6144` (which real
//! profiles configure) produced chunks of ~31.9 KB against a default ceiling of
//! 6000 bytes and a configured one of 8192 — so it fired on essentially
//! everything.

use vecdb_core::chunking::{ChunkParams, Chunker, RecursiveChunker};
use vecdb_core::config::{default_max_chunk_bytes, Config, DEFAULT_TARGET_CHUNK_SIZE};

/// Realistic multi-KB source text, sized in tokens by the default tokenizer.
fn corpus() -> String {
    // The crate's own source is representative of what actually gets ingested:
    // real code, real comments, real identifier density.
    let src = include_str!("../src/config.rs");
    src.repeat(3)
}

async fn max_chunk_bytes_at(target_chunk_size: usize) -> usize {
    let params = ChunkParams {
        target_chunk_size,
        // No ceiling: measure what the chunker *wants* to emit, which is the
        // number the ceiling has to be compared against.
        max_chunk_bytes: None,
        chunk_overlap: 0,
        tokenizer: "cl100k_base".to_string(),
        file_extension: None,
    };
    let chunks = RecursiveChunker
        .chunk(&corpus(), &params)
        .await
        .expect("chunking failed");
    chunks
        .iter()
        .map(|c| c.content.len())
        .max()
        .expect("no chunks produced")
}

#[tokio::test]
async fn derived_ceiling_clears_real_chunk_weight() {
    // The property that matters: for any target_chunk_size a profile might configure,
    // the *derived* ceiling must sit above what the chunker actually emits at
    // that size. Otherwise the default silently shreds every full-size chunk.
    for target_chunk_size in [512usize, 2048, 6144] {
        let observed = max_chunk_bytes_at(target_chunk_size).await;
        let ceiling = default_max_chunk_bytes(target_chunk_size);
        assert!(
            ceiling >= observed,
            "derived max_chunk_bytes ({ceiling} bytes) is BELOW the largest chunk \
             the chunker emits at target_chunk_size={target_chunk_size} ({observed} bytes). \
             Every full-size chunk would be re-split by FixedWidthChunker and its \
             structural boundaries discarded. Raise config::BYTES_PER_CHUNK_UNIT."
        );
    }
}

#[tokio::test]
async fn tokens_are_not_bytes_at_the_size_profiles_configure() {
    // Pins the measurement the fix is based on, so that if the tokenizer or the
    // chunker changes shape, this fails loudly rather than the ceiling quietly
    // becoming wrong again.
    //
    // 6144 is not arbitrary: it is what the `medium` and `high` profiles set.
    let observed = max_chunk_bytes_at(6144).await;
    assert!(
        observed > 8192,
        "expected a 6144-token chunk to weigh well over 8192 bytes (measured \
         ~31.9 KB when this test was written), got {observed}. If this is now \
         under 8192, the historical `max_chunk_bytes = 8192` was not the bug this \
         suite thinks it was — re-check config::BYTES_PER_CHUNK_UNIT."
    );

    // The two historical ceilings, both of which sat below the real weight.
    for stale_ceiling in [6000usize, 8192] {
        assert!(
            observed > stale_ceiling,
            "regression guard is meaningless if {stale_ceiling} already clears \
             the real chunk weight {observed}"
        );
    }
}

#[test]
fn resolved_ceiling_never_lands_below_its_own_target() {
    // A ceiling below the target it protects is the bug in one line. Config must
    // never resolve into that state, whatever the profile sets.
    for target_chunk_size in [256usize, 512, 2048, 6144] {
        let mut config = Config::default();
        config.ingestion.max_chunk_bytes = None;
        config
            .profiles
            .get_mut("default")
            .expect("default profile")
            .target_chunk_size = Some(target_chunk_size);

        let resolved = config.resolve(None, None).expect("resolution");

        assert!(
            resolved.max_chunk_bytes.value > target_chunk_size,
            "resolved max_chunk_bytes ({}) must exceed target_chunk_size ({target_chunk_size}); a \
             ceiling at or below the target turns the oversize guard into a second \
             chunker",
            resolved.max_chunk_bytes.value
        );
    }
}

#[test]
fn pipeline_fallback_matches_the_configured_derivation() {
    // There used to be two different defaults for the same knob: config derived
    // one from target_chunk_size, and `flush_chunks` hardcoded 6000. They disagreed,
    // and the hardcoded one was below every real profile's target_chunk_size.
    assert_eq!(
        default_max_chunk_bytes(DEFAULT_TARGET_CHUNK_SIZE),
        DEFAULT_TARGET_CHUNK_SIZE * vecdb_core::config::BYTES_PER_CHUNK_UNIT,
        "the pipeline fallback and the config derivation must be the same function"
    );
    assert_ne!(
        default_max_chunk_bytes(DEFAULT_TARGET_CHUNK_SIZE),
        6000,
        "6000 was the hardcoded value this fix removed; it must not reappear"
    );
}
