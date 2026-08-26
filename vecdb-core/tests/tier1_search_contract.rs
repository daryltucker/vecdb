//! Tier 1 — the search contract between a caller and the vector store.
//!
//! These are regression tests for four defects that all produced the same
//! user-visible symptom: a result list shorter than it should have been, with
//! nothing in the output explaining why. An agent reading a short list concludes
//! the corpus is thin and stops searching, so silent truncation is expensive out
//! of proportion to how small each bug looks.
//!
//!   1. `limit` was hardcoded to 10 and could not be raised.
//!   2. `min_score` was applied client-side AFTER the limit cut, so a threshold
//!      silently reduced the result count instead of filtering candidates.
//!   3. `min_score` was wired on the MCP path and ignored on the CLI path.
//!   4. Facet routing matched bare words, so ordinary prose scoped the search.
//!
//! The store-side half of (2) — that Qdrant applies `score_threshold` during
//! traversal — and the genesis over-fetch are exercised in `tier2_qdrant.rs`,
//! since they are properties of the real backend, not of this plumbing.

use anyhow::Result;
use async_trait::async_trait;
use std::sync::{Arc, Mutex};
use vecdb_common::FileTypeDetector;
use vecdb_core::backend::{Backend, SearchParams};
use vecdb_core::embedder::Embedder;
use vecdb_core::embedders::MockEmbedder;
use vecdb_core::parsers::ParserFactory;
use vecdb_core::types::{Chunk, CollectionInfo, SearchResult};
use vecdb_core::Core;

/// Records the `SearchParams` it was handed so the test can assert on what the
/// backend would actually have received.
struct RecordingBackend {
    seen: Arc<Mutex<Option<SearchParams>>>,
    facet_values: Vec<String>,
}

impl RecordingBackend {
    fn new(seen: Arc<Mutex<Option<SearchParams>>>) -> Self {
        Self {
            seen,
            facet_values: vec!["rust".to_string(), "python".to_string()],
        }
    }
}

#[async_trait]
impl Backend for RecordingBackend {
    async fn health_check(&self) -> Result<()> {
        Ok(())
    }
    async fn create_collection(
        &self,
        _n: &str,
        _s: u64,
        _q: Option<vecdb_core::config::QuantizationType>,
    ) -> Result<()> {
        Ok(())
    }
    async fn update_collection_quantization(
        &self,
        _n: &str,
        _q: vecdb_core::config::QuantizationType,
    ) -> Result<()> {
        Ok(())
    }
    async fn collection_exists(&self, _n: &str) -> Result<bool> {
        Ok(true)
    }
    async fn delete_collection(&self, _n: &str) -> Result<()> {
        Ok(())
    }
    async fn upsert(&self, _c: &str, _ch: Vec<Chunk>) -> Result<()> {
        Ok(())
    }
    async fn search(&self, _c: &str, _v: &[f32], p: SearchParams) -> Result<Vec<SearchResult>> {
        *self.seen.lock().unwrap() = Some(p);
        Ok(vec![])
    }
    async fn points_exists(&self, _c: &str, _i: Vec<String>) -> Result<Vec<String>> {
        Ok(vec![])
    }

    async fn delete_stale_points(
        &self,
        _c: &str,
        _d: &str,
        _k: &[String],
    ) -> anyhow::Result<usize> {
        Ok(0)
    }
    async fn list_collections(&self) -> Result<Vec<String>> {
        Ok(vec!["test".to_string()])
    }
    async fn get_collection_info(&self, name: &str) -> Result<CollectionInfo> {
        Ok(CollectionInfo {
            name: name.to_string(),
            vector_count: Some(100),
            vector_size: Some(768),
            quantization: None,
            vectors_on_disk: None,
            payload_on_disk: None,
        })
    }
    async fn list_metadata_values(&self, _c: &str, _k: &str) -> Result<Vec<String>> {
        Ok(self.facet_values.clone())
    }
    async fn get_collection_id(&self, _c: &str) -> Result<Option<String>> {
        Ok(None)
    }
    async fn set_collection_id(&self, _c: &str, _id: &str) -> Result<()> {
        Ok(())
    }
    async fn list_tasks(&self) -> Result<Vec<vecdb_core::types::TaskInfo>> {
        Ok(vec![])
    }

    async fn write_genesis(
        &self,
        _c: &str,
        _m: &vecdb_core::types::GenesisMetadata,
    ) -> anyhow::Result<()> {
        Ok(())
    }
    async fn read_genesis(&self, _c: &str) -> anyhow::Result<vecdb_core::types::CollectionGenesis> {
        // Marked as vecdb's, matching MockEmbedder's sentinel identity: these
        // tests are about the search contract, not about ownership. Ownership
        // has its own tests below.
        Ok(vecdb_core::types::CollectionGenesis {
            vecdb_version: Some("test".to_string()),
            vecdb_revision: None,
            chunking: None,
            collection_id: Some("mock-collection".to_string()),
            model: vecdb_core::types::ModelIdentity {
                name: "mock-embedder".to_string(),
                digest: Some("mock:test-double".to_string()),
                ..Default::default()
            },
            dimension: None,
            distance: Some("Cosine".to_string()),
            created_at: None,
        })
    }
}

struct DummyDetector;
impl FileTypeDetector for DummyDetector {
    fn detect(&self, _p: &std::path::Path, _c: &[u8]) -> vecdb_common::FileType {
        vecdb_common::FileType::Text
    }
}

struct DummyParserFactory;
impl ParserFactory for DummyParserFactory {
    fn get_parser(
        &self,
        _ft: vecdb_common::FileType,
    ) -> Option<Box<dyn vecdb_core::parsers::Parser>> {
        None
    }
}

fn core_with(seen: Arc<Mutex<Option<SearchParams>>>) -> Core {
    Core::with_backends(
        Arc::new(RecordingBackend::new(seen)),
        Arc::new(MockEmbedder::new(768)),
        Arc::new(DummyDetector),
        Arc::new(DummyParserFactory),
        vec!["source_type".to_string(), "language".to_string()], // smart_routing_keys
        vec![],                                                  // path_rules
        4,
        2,
    )
}

#[tokio::test]
async fn limit_and_threshold_reach_the_backend_unmodified() -> Result<()> {
    let seen = Arc::new(Mutex::new(None));
    core_with(seen.clone())
        .search(
            "test",
            "hello",
            SearchParams::new(42).with_score_threshold(Some(0.6)),
        )
        .await?;

    let got = seen.lock().unwrap().clone().expect("backend was called");
    assert_eq!(got.limit, 42, "limit must not be overridden downstream");
    assert_eq!(
        got.score_threshold,
        Some(0.6),
        "the threshold must be pushed to the store, not applied after truncation"
    );
    Ok(())
}

/// The core regression: a bare facet value in ordinary prose must not scope the
/// search. "how do I parse rust files" is a question about parsing, not a
/// request to restrict results to `language = rust`.
#[tokio::test]
async fn prose_containing_a_facet_value_does_not_filter() -> Result<()> {
    let seen = Arc::new(Mutex::new(None));
    let (_results, applied) = core_with(seen.clone())
        .search_smart("test", "how do I parse rust files", SearchParams::new(10))
        .await?;

    assert!(
        applied.is_empty(),
        "no qualifier was written, so no filter may be applied: {applied:?}"
    );
    let got = seen.lock().unwrap().clone().expect("backend was called");
    assert!(got.filter.is_none(), "filter reached the backend: {got:?}");
    Ok(())
}

#[tokio::test]
async fn explicit_qualifier_filters_and_is_reported_back() -> Result<()> {
    let seen = Arc::new(Mutex::new(None));
    let (_results, applied) = core_with(seen.clone())
        .search_smart("test", "parse files language:rust", SearchParams::new(10))
        .await?;

    assert_eq!(
        applied.get("language").and_then(|v| v.as_str()),
        Some("rust"),
        "the caller must be told what its search was narrowed by"
    );

    let got = seen.lock().unwrap().clone().expect("backend was called");
    assert_eq!(
        got.filter,
        Some(serde_json::json!({"language": "rust"})),
        "the qualifier must become a real backend filter"
    );
    Ok(())
}

/// An unknown facet value must fail loudly. Returning zero results is
/// indistinguishable from "this collection has no answer", which is the single
/// most misleading thing a search tool can tell an agent.
#[tokio::test]
async fn unknown_facet_value_is_an_error_naming_the_valid_ones() {
    let seen = Arc::new(Mutex::new(None));
    let err = core_with(seen.clone())
        .search_smart("test", "language:cobol parse", SearchParams::new(10))
        .await
        .expect_err("an unsatisfiable qualifier must not return an empty list")
        .to_string();

    assert!(err.contains("rust") && err.contains("python"), "{err}");
    assert!(
        seen.lock().unwrap().is_none(),
        "no search should be issued once the qualifier is known to be unsatisfiable"
    );
}

/// Smart mode must not silently rewrite an ordinary query. Only recognized
/// qualifiers are stripped; everything else is embedded exactly as written.
#[tokio::test]
async fn unrecognized_colon_token_is_left_in_the_query() -> Result<()> {
    let seen = Arc::new(Mutex::new(None));
    let (_r, applied) = core_with(seen.clone())
        .search_smart(
            "test",
            "see https://example.com/a for setup",
            SearchParams::new(10),
        )
        .await?;
    assert!(applied.is_empty(), "{applied:?}");
    Ok(())
}

// ── Embedding space guard ──────────────────────────────────────────────
//
// The defect: the guard compared dimension only, and dimension is the least
// discriminating field there is. 384 and 768 are the two most common embedding
// dimensions in existence, so two unrelated models routinely pass a dimension
// check and write into the same collection. Cosine across two embedding spaces
// is noise, so the failure is silent quality loss with no diagnostic.

use vecdb_core::types::{compare_spaces, Compatibility, ModelIdentity};

fn model(name: &str, digest: &str, arch: &str, params: &str, quant: &str) -> ModelIdentity {
    ModelIdentity {
        name: name.into(),
        digest: Some(digest.into()),
        architecture: Some(arch.into()),
        family: Some(arch.into()),
        parameter_size: Some(params.into()),
        quantization_level: Some(quant.into()),
        embedding_length: None,
        context_length: None,
    }
}

/// The exact fleet scenario from the bug report: sleipnir has
/// nomic-embed-text (768), another box resolves to bge-base-en-v1.5 (768).
/// The old guard compared 768 != 768, passed, and mixed two spaces forever.
#[test]
fn two_different_768_dim_models_cannot_share_a_collection() {
    let report = compare_spaces(
        &model(
            "nomic-embed-text",
            "sha256:aaa",
            "nomic-bert",
            "137M",
            "F16",
        ),
        Some(768),
        &model("bge-base-en-v1.5", "sha256:bbb", "bert", "109M", "F16"),
        Some(768),
    );

    assert_eq!(report.tier, Compatibility::Incompatible);
    assert!(!report.permits_write(false));
    assert!(
        !report.permits_write(true),
        "no flag may override an incompatible space — this is the corruption case"
    );
    assert!(!report.permits_read());
}

/// The same model at two quantizations is the case the spec deliberately
/// admits: tracked and known, free to read, gated on write.
#[test]
fn same_model_different_quant_is_read_free_write_gated() {
    let report = compare_spaces(
        &model(
            "qwen3-embedding:4b",
            "sha256:df5b",
            "qwen3",
            "4.02B",
            "Q4_K_M",
        ),
        Some(2560),
        &model(
            "qwen3-embedding:4b-q8_0",
            "sha256:357d",
            "qwen3",
            "4.02B",
            "Q8_0",
        ),
        Some(2560),
    );

    assert_eq!(report.tier, Compatibility::Compatible);
    assert!(
        report.permits_read(),
        "a quant delta must never block a read"
    );
    assert!(!report.permits_write(false));
    assert!(report.permits_write(true));
    assert!(
        report.warning().is_some(),
        "the delta is recorded, not hidden"
    );
}

/// Tags are not identity. On blade, `qwen3-embedding:4b` and
/// `qwen3-embedding:4b-q4_K_M` are the same blob under different names —
/// a name comparison would call these different models.
#[test]
fn digest_equality_beats_a_name_difference() {
    let report = compare_spaces(
        &model(
            "qwen3-embedding:4b",
            "sha256:df5b",
            "qwen3",
            "4.02B",
            "Q4_K_M",
        ),
        Some(2560),
        &model(
            "qwen3-embedding:4b-q4_K_M",
            "sha256:df5b",
            "qwen3",
            "4.02B",
            "Q4_K_M",
        ),
        Some(2560),
    );
    assert_eq!(report.tier, Compatibility::Identical);
    assert!(report.permits_write(false));
}

/// MERT audio (1024, Cosine) versus qwen3-embedding:0.6b text (1024, Cosine).
/// Nothing distinguishes them by dimension or distance. Refusing on
/// insufficient identity is the only safe answer.
#[test]
fn unknown_identity_refuses_rather_than_assuming_a_match() {
    let report = compare_spaces(
        &ModelIdentity::unknown("mert_v1_330m"),
        Some(1024),
        &model(
            "qwen3-embedding:0.6b",
            "sha256:ac6d",
            "qwen3",
            "596.05M",
            "Q8_0",
        ),
        Some(1024),
    );
    assert_eq!(report.tier, Compatibility::Incompatible);
    assert!(!report.permits_write(true));
}

// ── Collection ownership ───────────────────────────────────────────────
//
// "Not a vecdb collection" and "an incompatible vecdb collection" are different
// statements. The first is about ownership and is answered by a magic marker;
// the second is about embedding spaces and is only a meaningful question once
// the first is settled. Conflating them produces the nonsense claim that
// someone else's audio collection has "a model mismatch".

use vecdb_core::types::CollectionGenesis;

#[test]
fn a_collection_is_ours_only_if_it_says_so() {
    // Absence of the marker — the MERT case. Not inferred from missing fields.
    assert!(!CollectionGenesis::default().is_vecdb());

    let ours = CollectionGenesis {
        vecdb_version: CollectionGenesis::parse_marker(Some(CollectionGenesis::marker_value())),
        ..Default::default()
    };
    assert!(ours.is_vecdb());
    assert_eq!(
        ours.vecdb_version.as_deref(),
        Some(env!("CARGO_PKG_VERSION"))
    );
}

/// The marker is a magic number: it cannot be arrived at by accident, and a
/// point sitting at the nil UUID for unrelated reasons must not be mistaken
/// for ours.
#[test]
fn only_the_magic_prefix_counts_as_ownership() {
    assert_eq!(CollectionGenesis::parse_marker(None), None);
    assert_eq!(CollectionGenesis::parse_marker(Some(String::new())), None);
    assert_eq!(
        CollectionGenesis::parse_marker(Some("mert-provenance:2".into())),
        None
    );
    assert_eq!(
        CollectionGenesis::parse_marker(Some("notvecdb:1.0.0".into())),
        None
    );
    assert_eq!(
        CollectionGenesis::parse_marker(Some("vecdb:1.0.3".into())),
        Some("1.0.3".to_string())
    );
}

/// The marker is self-describing so it is unambiguous in a raw Qdrant payload.
#[test]
fn marker_is_readable_on_its_own() {
    let v = CollectionGenesis::marker_value();
    assert!(v.starts_with("vecdb:"), "{v}");
    assert_eq!(
        CollectionGenesis::parse_marker(Some(v.clone())).as_deref(),
        Some(env!("CARGO_PKG_VERSION")),
        "a marker vecdb writes must be one vecdb can read back: {v}"
    );
}

// ─────────────────────────────────────────────────────────────────────────────
// Decorator forwarding
//
// `ArbitratedEmbedder` wraps every embedder in the live wiring (`Core::new`).
// `Embedder::identity()` has a name-only default and the write guard refuses a
// name-only identity, so a decorator that forwards `embed` but not `identity`
// makes every model unidentifiable — including to itself — and blocks all
// ingestion with an error claiming the model does not match itself.
//
// That shipped once. These tests exist so it cannot ship twice.
// ─────────────────────────────────────────────────────────────────────────────

use vecdb_core::embedders::arbitrated::ArbitratedEmbedder;
use vecdb_core::resource::ResourceArbiter;

fn wrapped() -> ArbitratedEmbedder {
    ArbitratedEmbedder::new(
        Arc::new(MockEmbedder::new(384)) as Arc<dyn Embedder + Send + Sync>,
        Arc::new(ResourceArbiter::new()),
    )
}

#[tokio::test]
async fn arbitrated_embedder_forwards_identity() {
    let inner = MockEmbedder::new(384);
    let expected = inner.identity().await.expect("inner identity");
    let got = wrapped().identity().await.expect("wrapped identity");

    assert_eq!(
        got.digest, expected.digest,
        "the decorator dropped the digest and fell back to the name-only default"
    );
    assert_eq!(got.architecture, expected.architecture);
    assert_eq!(got.name, expected.name);
}

/// The specific end-state that broke ingestion: a wrapped embedder must be
/// judged compatible with a collection its own inner embedder created.
#[tokio::test]
async fn wrapped_embedder_is_compatible_with_its_own_collection() {
    let collection_identity = MockEmbedder::new(384).identity().await.unwrap();
    let active_identity = wrapped().identity().await.unwrap();

    let report = vecdb_core::types::compare_spaces(
        &collection_identity,
        Some(384),
        &active_identity,
        Some(384),
    );

    assert!(
        report.permits_write(false),
        "a model must be able to write to the collection it created; got {:?}: {}",
        report.tier,
        report.reason
    );
}
