/*
 * PURPOSE:
 *   A collection's genesis must record how its text was CUT, not only which
 *   model embedded it.
 *
 * WHY THIS EXISTS
 *   Genesis described the model exhaustively — digest, architecture, parameter
 *   size, quantization, context length — and said nothing about chunking, which
 *   is baked into the vectors just as permanently and is recoverable from
 *   nowhere afterwards.
 *
 *   Observed 2026-238 on a live collection: `code` was first built at 512
 *   tokens with all-minilm-l6-v2, then rebuilt at 12000 with
 *   qwen3-embedding:0.6b. Nothing in the collection recorded which chunking
 *   produced which points, and the two are indistinguishable after the fact.
 *
 *   This is a prerequisite for RFC-2026-238 (corpus_format). Recording the fact
 *   is what makes a later gate possible; this file only asserts the fact is
 *   recorded, faithfully and completely.
 */

use vecdb_core::ingestion::options::{ChunkSpec, IngestionOptions};
use vecdb_core::types::ChunkingIdentity;

/// Built explicitly rather than via `Default`, deliberately: a default
/// `IngestionOptions` would have `target_chunk_size: 0`, which is not a
/// configuration anyone should be able to reach by accident.
fn options_for(collection: &str, target: usize, overlap: usize) -> IngestionOptions {
    IngestionOptions {
        path: ".".to_string(),
        file_allowlist: None,
        project_root: None,
        collection: collection.to_string(),
        vecdbrc_routes: None,
        vecdbrc_root: None,
        target_chunk_size: target,
        max_chunk_bytes: None,
        route_chunking: std::collections::HashMap::new(),
        on_oversize: Default::default(),
        chunk_overlap: overlap,
        respect_gitignore: false,
        ignore_vectorignore: false,
        strategy: "recursive".to_string(),
        tokenizer: "cl100k_base".to_string(),
        git_ref: None,
        extensions: None,
        excludes: None,
        dry_run: true,
        metadata: None,
        path_rules: Vec::new(),
        max_concurrent_requests: 1,
        gpu_batch_size: 1,
        quantization: None,
        allow_quantization_delta: false,
    }
}

/// The tokenizer must travel with the number it denominates.
///
/// `target_chunk_size` counts whatever `tokenizer` counts, so a bare 512 is
/// ambiguous — 512 cl100k_base tokens and 512 bytes are wildly different
/// corpora. Recording one without the other stores a figure nobody can act on.
#[test]
fn chunking_identity_carries_its_unit() {
    let id = ChunkingIdentity {
        target_chunk_size: 512,
        chunk_overlap: 50,
        max_chunk_bytes: 3072,
        tokenizer: "cl100k_base".to_string(),
    };
    assert_eq!(id.tokenizer, "cl100k_base");
    assert_eq!(id.target_chunk_size, 512);
}

/// It must round-trip through serde unchanged — this is what `vecdb list
/// --json` emits and what a future gate will compare.
#[test]
fn chunking_identity_round_trips() {
    let id = ChunkingIdentity {
        target_chunk_size: 12000,
        chunk_overlap: 50,
        max_chunk_bytes: 72000,
        tokenizer: "cl100k_base".to_string(),
    };
    let json = serde_json::to_string(&id).expect("serialize");
    let back: ChunkingIdentity = serde_json::from_str(&json).expect("deserialize");
    assert_eq!(id, back);
}

/// The resolved ceiling is recorded, never the `Option`.
///
/// `max_chunk_bytes` is derived from `target_chunk_size` when unset. Once a
/// chunk has been cut by that ceiling, a derived value and a written one are
/// the same fact, and storing `None` would lose it — the next reader cannot
/// re-derive it without also knowing which version's formula applied.
#[test]
fn a_derived_ceiling_is_recorded_as_the_number_that_applied() {
    // max_chunk_bytes left unset — must still be recorded resolved.
    let options = options_for("test_genesis_chunking", 512, 50);

    let id = options.chunking_identity("test_genesis_chunking");

    assert_eq!(id.target_chunk_size, 512);
    assert_eq!(id.chunk_overlap, 50);
    assert_eq!(id.tokenizer, "cl100k_base");
    assert_eq!(
        id.max_chunk_bytes,
        512 * vecdb_core::config::BYTES_PER_CHUNK_UNIT,
        "an unset ceiling must be recorded as the value that actually applied"
    );
    assert!(
        id.max_chunk_bytes >= id.target_chunk_size,
        "a ceiling below the target it protects is a second chunker, not a guard"
    );
}

/// A `.vecdbrc` fans one run across collections whose chunk parameters differ,
/// and each destination's genesis must record ITS OWN — not the run's primary.
///
/// This is the same defect class as the routed-dimension bug fixed on Day 236:
/// one value resolved once for every destination.
#[test]
fn each_destination_records_its_own_chunking() {
    let mut options = options_for("test_primary", 512, 50);
    options.route_chunking.insert(
        "test_routed".to_string(),
        ChunkSpec {
            target_chunk_size: 12000,
            chunk_overlap: 0,
            max_chunk_bytes: None,
        },
    );

    let primary = options.chunking_identity("test_primary");
    let routed = options.chunking_identity("test_routed");

    assert_eq!(primary.target_chunk_size, 512);
    assert_eq!(
        routed.target_chunk_size, 12000,
        "route must not inherit the run's"
    );
    assert_eq!(primary.chunk_overlap, 50);
    assert_eq!(routed.chunk_overlap, 0);
    assert_ne!(
        primary.max_chunk_bytes, routed.max_chunk_bytes,
        "derived ceilings must follow their own target, not the primary's"
    );
}

/// A version cannot identify a build, and semantics change between releases.
///
/// Measured 2026-238: the live `code` collection reports `vecdb:1.0.4` yet was
/// written by a development build that already contained the Python/Go
/// fidelity fix — the version had not been bumped yet. Anything reasoning from
/// the version alone would classify it stale and demand a needless re-ingest.
/// See RFC-2026-238.
#[test]
fn the_build_revision_is_recorded_separately_from_the_version() {
    let rev = vecdb_core::types::build_revision();
    assert!(
        !rev.is_empty(),
        "a build must always be able to name itself"
    );
    assert_ne!(
        rev, "unknown",
        "built inside a checkout, so build.rs must have resolved a revision"
    );

    // The marker stays the bare version. parse_marker returns everything after
    // the colon, so a suffix here would silently become part of "the version".
    let marker = vecdb_core::types::CollectionGenesis::marker_value();
    assert_eq!(marker, format!("vecdb:{}", env!("CARGO_PKG_VERSION")));
    assert!(
        !marker.contains(&rev),
        "the revision must not leak into the version marker: {marker}"
    );
}
