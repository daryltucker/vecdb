//! Per-route chunk parameters.
//!
//! A `.vecdbrc` fans one ingest across several collections. Chunking is a
//! property of the *destination*, not of the run: profiles in this fleet span
//! 16x in `target_chunk_size` (384 for a small-context model, 6144 for a large one).
//! Chunking every route identically means files headed to one collection are
//! cut to the size configured for another.
//!
//! That is not recoverable later. Chunk size is baked into the vectors at
//! ingest, so the only repair is a full re-ingest — which is why this is
//! asserted rather than left to the integration tier.

use std::collections::HashMap;
use vecdb_core::config::{default_max_chunk_bytes, OversizePolicy};
use vecdb_core::ingestion::options::ChunkSpec;
use vecdb_core::ingestion::IngestionOptions;

fn options(route_chunking: HashMap<String, ChunkSpec>) -> IngestionOptions {
    IngestionOptions {
        path: ".".to_string(),
        file_allowlist: None,
        project_root: None,
        collection: "test_fallback_collection".to_string(),
        vecdbrc_routes: None,
        vecdbrc_root: None,
        target_chunk_size: 512,
        max_chunk_bytes: None,
        on_oversize: OversizePolicy::default(),
        route_chunking,
        chunk_overlap: 50,
        respect_gitignore: false,
        ignore_vectorignore: false,
        strategy: "recursive".to_string(),
        tokenizer: "cl100k_base".to_string(),
        git_ref: None,
        extensions: None,
        excludes: None,
        dry_run: false,
        metadata: None,
        path_rules: vec![],
        max_concurrent_requests: 1,
        gpu_batch_size: 1,
        quantization: None,
        allow_quantization_delta: false,
    }
}

/// The whole point: two destinations, two chunk sizes, one run.
#[test]
fn each_destination_gets_its_own_chunk_size() {
    let mut routes = HashMap::new();
    routes.insert(
        "test_code".to_string(),
        ChunkSpec {
            target_chunk_size: 384,
            chunk_overlap: 32,
            max_chunk_bytes: None,
        },
    );
    routes.insert(
        "test_docs".to_string(),
        ChunkSpec {
            target_chunk_size: 6144,
            chunk_overlap: 256,
            max_chunk_bytes: None,
        },
    );

    let opts = options(routes);

    let code = opts.chunking_for("test_code");
    let docs = opts.chunking_for("test_docs");

    assert_eq!(code.target_chunk_size, 384);
    assert_eq!(docs.target_chunk_size, 6144);
    assert_eq!(code.chunk_overlap, 32);
    assert_eq!(docs.chunk_overlap, 256);
    assert_ne!(
        code.target_chunk_size, docs.target_chunk_size,
        "routed destinations must not collapse to a single chunk size"
    );
}

/// A destination with no entry falls through to the run's own parameters —
/// the same answer as before routing existed.
#[test]
fn unrouted_destination_falls_back_to_the_run() {
    let opts = options(HashMap::new());
    let spec = opts.chunking_for("anything_at_all");

    assert_eq!(spec.target_chunk_size, 512);
    assert_eq!(spec.chunk_overlap, 50);
    assert_eq!(spec.max_chunk_bytes, None);
}

/// The ceiling follows the route too. It used to be resolved once for the run,
/// so a destination configured for 6144-token chunks inherited a ceiling sized
/// for 384 and had every chunk re-split.
#[test]
fn the_ceiling_follows_the_route() {
    let mut routes = HashMap::new();
    routes.insert(
        "test_small".to_string(),
        ChunkSpec {
            target_chunk_size: 384,
            chunk_overlap: 0,
            max_chunk_bytes: None,
        },
    );
    routes.insert(
        "test_large".to_string(),
        ChunkSpec {
            target_chunk_size: 6144,
            chunk_overlap: 0,
            max_chunk_bytes: None,
        },
    );

    let opts = options(routes);
    let small = opts.chunking_for("test_small").ceiling();
    let large = opts.chunking_for("test_large").ceiling();

    assert_eq!(small, default_max_chunk_bytes(384));
    assert_eq!(large, default_max_chunk_bytes(6144));
    assert!(
        large > small,
        "a destination configured for larger chunks must get a larger ceiling; \
         one ceiling for the whole run is what shredded structural output"
    );
    assert!(
        large > 6144,
        "the ceiling must sit above the target it protects"
    );
}

/// An explicit ceiling on a route is honoured verbatim, not re-derived.
#[test]
fn explicit_ceiling_is_not_overridden() {
    let mut routes = HashMap::new();
    routes.insert(
        "test_pinned".to_string(),
        ChunkSpec {
            target_chunk_size: 2048,
            chunk_overlap: 0,
            max_chunk_bytes: Some(99_000),
        },
    );

    let opts = options(routes);
    assert_eq!(opts.chunking_for("test_pinned").ceiling(), 99_000);
}
