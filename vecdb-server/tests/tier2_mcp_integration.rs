use async_trait::async_trait;
use serde_json::json;
use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use vecdb_core::config::Config;
use vecdb_core::parsers::ParserFactory;
use vecdb_core::Core;
use vecdb_server::core_registry::{CoreKey, CoreRegistry};
use vecdb_server::rpc::{handle_request, types::JsonRpcRequest};
use vecq::detection::HybridDetector;

mod common;
use common::{MockBackend, MockEmbedder};

/// A collection with no vecdb genesis point — i.e. one belonging to another
/// tool. `list_collections` must list it and label it, never hide it.
const FOREIGN_COLLECTION: &str = "test_mcp_foreign";

struct MockParserFactory;
impl ParserFactory for MockParserFactory {
    fn get_parser(
        &self,
        _file_type: vecdb_common::FileType,
    ) -> Option<Box<dyn vecdb_core::parsers::Parser>> {
        None
    }
}

// ──────────────────────────────────────────────────────────────────────────────
// Helpers
// ──────────────────────────────────────────────────────────────────────────────

/// Build a single-Core registry for tests that only need the default profile.
fn make_single_registry(core: Arc<Core>, config: &Config, profile_name: &str) -> Arc<CoreRegistry> {
    let resolution = config.resolve(Some(profile_name), None).unwrap();
    let key = CoreKey::from_resolution(&resolution);
    let mut cores = HashMap::new();
    cores.insert(key, core);
    Arc::new(CoreRegistry::from_map(cores, profile_name))
}

// ──────────────────────────────────────────────────────────────────────────────
// Test 1: Full lifecycle (search, embed, list) with a single mock Core.
// ──────────────────────────────────────────────────────────────────────────────
#[tokio::test]
async fn test_mcp_full_lifecycle() {
    let storage = Arc::new(Mutex::new(Vec::new()));

    // Pre-populate storage for search test
    {
        let mut store = storage.lock().unwrap();
        store.push(vecdb_core::types::Chunk {
            id: "test-id".to_string(),
            document_id: "doc-1".to_string(),
            content: "hello world".to_string(),
            metadata: std::collections::HashMap::new(),
            vector: None,
            page_num: None,
            start_line: None,
            end_line: None,
            byte_start: 0,
            byte_end: 5,
        });
    }

    let backend = Arc::new(MockBackend {
        storage: storage.clone(),
    });
    let embedder = Arc::new(MockEmbedder);
    let detector = Arc::new(HybridDetector::new());
    let parser_factory = Arc::new(MockParserFactory);

    let core = Arc::new(Core::with_backends(
        backend,
        embedder,
        detector,
        parser_factory,
        Vec::new(),
        Vec::new(),
        1,
        10,
    ));

    let mut config = Config::default();

    // `list_collections` does not use the injected MockBackend: it builds a real
    // QdrantBackend per configured endpoint so it can enumerate collections the
    // active profile knows nothing about. With `Config::default()` that endpoint
    // is the *production* URL (6334), so this test was silently asking whether
    // production Qdrant happened to be running on the machine — it passed when
    // it was, and enumerated nothing when it was not.
    let test_qdrant = std::env::var("VECDB_TEST_QDRANT_URL")
        .unwrap_or_else(|_| "http://localhost:6336".to_string());
    if let Some(profile) = config.profiles.get_mut("default") {
        // `test_`-prefixed: a default named after a real collection ("docs")
        // puts a production name in the test's assertions and in the instance.
        profile.default_collection_name = Some(FOREIGN_COLLECTION.to_string());
        profile.qdrant_url = test_qdrant.clone();
    }

    // Create the collection this test asserts on, rather than hoping one is
    // left over from an earlier run. It previously depended on whatever the
    // instance already contained, so it passed or failed based on test-ordering
    // and leftovers — and it broke the moment the suite started resetting the
    // instance (tests/tier0_reset_qdrant.py).
    //
    // Created bare, with no genesis point, which is precisely what "not a vecdb
    // collection" means: that is the state `is_compatible: false` describes.
    let raw = vecdb_core::backends::qdrant::QdrantBackend::new(&test_qdrant, None)
        .expect("test Qdrant must be reachable");
    {
        use vecdb_core::backend::Backend;
        let _ = raw.delete_collection(FOREIGN_COLLECTION).await;
        raw.create_collection(FOREIGN_COLLECTION, 384, None)
            .await
            .expect("failed to create the foreign fixture collection");
    }

    let registry = make_single_registry(core, &config, "default");
    let config = Arc::new(config);

    // 1. Initialize
    let req = JsonRpcRequest {
        jsonrpc: "2.0".to_string(),
        method: "initialize".to_string(),
        params: None,
        id: Some(json!(1)),
    };
    let res = handle_request(&registry, &config, &req, false, "default")
        .await
        .unwrap();
    assert_eq!(res["serverInfo"]["name"], "vecdb-mcp");

    // 2. List Collections
    let req = JsonRpcRequest {
        jsonrpc: "2.0".to_string(),
        method: "tools/call".to_string(),
        params: Some(json!({
            "name": "list_collections",
            "arguments": {}
        })),
        id: Some(json!(2)),
    };
    let res = handle_request(&registry, &config, &req, false, "default")
        .await
        .unwrap();
    let content = res["content"][0]["text"].as_str().unwrap();
    assert!(content.contains(r#""is_compatible": false"#));
    assert!(content.contains(r#""is_local": true"#));
    assert!(content.contains(FOREIGN_COLLECTION));

    // 3. Embed
    let req = JsonRpcRequest {
        jsonrpc: "2.0".to_string(),
        method: "tools/call".to_string(),
        params: Some(json!({
            "name": "embed",
            "arguments": {
                "texts": ["hello"]
            }
        })),
        id: Some(json!(3)),
    };
    let res = handle_request(&registry, &config, &req, false, "default")
        .await
        .unwrap();
    let content = res["content"][0]["text"].as_str().unwrap();
    assert!(content.contains("0.1"));

    // 4. Search
    let req = JsonRpcRequest {
        jsonrpc: "2.0".to_string(),
        method: "tools/call".to_string(),
        params: Some(json!({
            "name": "search_vectors",
            "arguments": {
                "query": "something",
                "collection": FOREIGN_COLLECTION,
                "profile": "default",
                "json": false,
                "smart": false
            }
        })),
        id: Some(json!(4)),
    };
    let res = handle_request(&registry, &config, &req, false, "default")
        .await
        .unwrap();
    let content = res["content"][0]["text"].as_str().unwrap();
    assert!(content.contains("0.99")); // Mock score

    // Leave the instance as we found it.
    {
        use vecdb_core::backend::Backend;
        let _ = raw.delete_collection(FOREIGN_COLLECTION).await;
    }
}

// ──────────────────────────────────────────────────────────────────────────────
// Test 2: Multi-profile dispatch — the core BUG 1 regression test.
//
// Server boots with "default" profile (dim=3 embedder, backend A).
// "test_alt_col" is configured with "alternate" profile (dim=7 embedder, backend B).
// Verifies that searching "test_alt_col" routes to backend B, not backend A.
// ──────────────────────────────────────────────────────────────────────────────

/// A parameterized MockEmbedder for multi-profile tests.
struct DimMockEmbedder {
    dim: usize,
}

#[async_trait]
impl vecdb_core::embedder::Embedder for DimMockEmbedder {
    async fn embed(&self, _text: &str, _target_dim: Option<usize>) -> anyhow::Result<Vec<f32>> {
        Ok(vec![0.1; self.dim])
    }
    async fn embed_batch(
        &self,
        texts: &[String],
        _target_dim: Option<usize>,
    ) -> anyhow::Result<Vec<Vec<f32>>> {
        Ok(vec![vec![0.1; self.dim]; texts.len()])
    }
    async fn dimension(&self) -> anyhow::Result<usize> {
        Ok(self.dim)
    }
    fn model_name(&self) -> String {
        format!("dim-mock-{}", self.dim)
    }

    async fn identity(&self) -> anyhow::Result<vecdb_core::types::ModelIdentity> {
        // Shared sentinel: every test double is one embedding space, so the
        // compatibility guard passes and these tests exercise what they are
        // actually about. Guard behaviour has its own dedicated tests.
        Ok(vecdb_core::types::ModelIdentity {
            name: "mock-embedder".to_string(),
            digest: Some("mock:test-double".to_string()),
            architecture: Some("mock".to_string()),
            family: Some("mock".to_string()),
            parameter_size: Some("0".to_string()),
            quantization_level: Some("none".to_string()),
            embedding_length: None,
            context_length: Some(8192),
        })
    }
}

#[tokio::test]
async fn test_mcp_multiprofile_dispatch() {
    // ── Build two backends with distinct marker content ──────────────────────
    let storage_default = Arc::new(Mutex::new(vec![vecdb_core::types::Chunk {
        id: "id-default".to_string(),
        document_id: "doc-default".to_string(),
        content: "MARKER_DEFAULT".to_string(),
        metadata: std::collections::HashMap::new(),
        vector: None,
        page_num: None,
        start_line: None,
        end_line: None,
        byte_start: 0,
        byte_end: 14,
    }]));
    let storage_alternate = Arc::new(Mutex::new(vec![vecdb_core::types::Chunk {
        id: "id-alternate".to_string(),
        document_id: "doc-alternate".to_string(),
        content: "MARKER_ALTERNATE".to_string(),
        metadata: std::collections::HashMap::new(),
        vector: None,
        page_num: None,
        start_line: None,
        end_line: None,
        byte_start: 0,
        byte_end: 16,
    }]));

    let backend_default = Arc::new(MockBackend {
        storage: storage_default,
    });
    let backend_alternate = Arc::new(MockBackend {
        storage: storage_alternate,
    });

    let detector = Arc::new(HybridDetector::new());
    let parser = Arc::new(MockParserFactory);

    // ── Build two Cores with different embedders and backends ────────────────
    let core_default = Arc::new(Core::with_backends(
        backend_default,
        Arc::new(DimMockEmbedder { dim: 3 }),
        detector.clone(),
        Arc::new(MockParserFactory),
        Vec::new(),
        Vec::new(),
        1,
        10,
    ));

    let core_alternate = Arc::new(Core::with_backends(
        backend_alternate,
        Arc::new(DimMockEmbedder { dim: 7 }),
        detector.clone(),
        parser,
        Vec::new(),
        Vec::new(),
        1,
        10,
    ));

    // ── Build a config with two profiles and one collection ──────────────────
    let mut config = Config::default();

    // A second embedder — a different model on the same backend, which is the
    // case that must produce a distinct CoreKey.
    config.embedder.insert(
        "alternate".to_string(),
        vecdb_core::config::EmbedderSpec {
            backend: "local".to_string(),
            model: "alternate-model".to_string(),
            num_ctx: None,
            batch_inputs: None,
            batch_rows: None,
            use_gpu: None,
            dimension: None,
        },
    );
    config.profiles.insert(
        "alternate".to_string(),
        vecdb_core::config::Profile {
            embedder: "alternate".to_string(),
            qdrant_url: "http://localhost:6334".to_string(),
            qdrant_api_key: None,
            default_collection_name: Some("test_alt_col".to_string()),
            quantization: None,
            target_chunk_size: None,
            max_chunk_bytes: None,
            chunk_overlap: None,
            resolved_profile_name: "alternate".to_string(),
        },
    );

    config.collections.insert(
        "test_alt_col".to_string(),
        vecdb_core::config::CollectionConfig {
            name: "test_alt_col".to_string(),
            description: None,
            profile: Some("alternate".to_string()),
            embedder: None,
            qdrant_url: None,
            qdrant_api_key: None,
            target_chunk_size: None,
            chunk_overlap: None,
            max_chunk_bytes: None,
            quantization: None,
        },
    );
    if let Some(p) = config.profiles.get_mut("default") {
        p.default_collection_name = Some(FOREIGN_COLLECTION.to_string());
    }

    // ── Pre-seed registry with both Cores ────────────────────────────────────
    let key_default = CoreKey::from_resolution(&config.resolve(Some("default"), None).unwrap());
    let key_alternate = CoreKey::from_resolution(&config.resolve(Some("alternate"), None).unwrap());
    assert_ne!(
        key_default, key_alternate,
        "two embedders differing only in model must not share a Core"
    );

    let mut cores = HashMap::new();
    cores.insert(key_default, core_default);
    cores.insert(key_alternate, core_alternate);

    let registry = Arc::new(CoreRegistry::from_map(cores, "default"));
    let config = Arc::new(config);

    // ── Test: searching "docs" routes to default backend (MARKER_DEFAULT) ────
    let req = JsonRpcRequest {
        jsonrpc: "2.0".to_string(),
        method: "tools/call".to_string(),
        params: Some(json!({
            "name": "search_vectors",
            "arguments": {
                "query": "test",
                "collection": FOREIGN_COLLECTION,
                "json": false,
                "smart": false
            }
        })),
        id: Some(json!(1)),
    };
    let res = handle_request(&registry, &config, &req, false, "default")
        .await
        .unwrap();
    let text = res["content"][0]["text"].as_str().unwrap();
    assert!(
        text.contains("MARKER_DEFAULT"),
        "Expected MARKER_DEFAULT in 'docs' search result, got: {}",
        text
    );
    assert!(
        !text.contains("MARKER_ALTERNATE"),
        "docs search must NOT return alternate backend results"
    );

    // ── Test: searching "test_alt_col" routes to alternate backend (MARKER_ALTERNATE)
    let req = JsonRpcRequest {
        jsonrpc: "2.0".to_string(),
        method: "tools/call".to_string(),
        params: Some(json!({
            "name": "search_vectors",
            "arguments": {
                "query": "test",
                "collection": "test_alt_col",
                "json": false,
                "smart": false
            }
        })),
        id: Some(json!(2)),
    };
    let res = handle_request(&registry, &config, &req, false, "default")
        .await
        .unwrap();
    let text = res["content"][0]["text"].as_str().unwrap();
    assert!(
        text.contains("MARKER_ALTERNATE"),
        "Expected MARKER_ALTERNATE in 'alt-col' search result, got: {}",
        text
    );
    assert!(
        !text.contains("MARKER_DEFAULT"),
        "alt-col search must NOT return default backend results"
    );
}
