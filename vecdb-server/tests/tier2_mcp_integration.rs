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
fn make_single_registry(
    core: Arc<Core>,
    config: &Config,
    profile_name: &str,
) -> Arc<CoreRegistry> {
    let profile = config.get_profile(Some(profile_name)).unwrap();
    let key = CoreKey::from_resolved(&profile, config);
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
            char_start: 0,
            char_end: 5,
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
    if let Some(profile) = config.profiles.get_mut("default") {
        profile.default_collection_name = Some("docs".to_string());
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
    assert!(content.contains(r#""is_compatible": true"#));
    assert!(content.contains(r#""is_active": true"#));
    assert!(content.contains("docs"));

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
                "collection": "docs",
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
}

// ──────────────────────────────────────────────────────────────────────────────
// Test 2: Multi-profile dispatch — the core BUG 1 regression test.
//
// Server boots with "default" profile (dim=3 embedder, backend A).
// "alt-col" is configured with "alternate" profile (dim=7 embedder, backend B).
// Verifies that searching "alt-col" routes to backend B, not backend A.
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
        char_start: 0,
        char_end: 14,
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
        char_start: 0,
        char_end: 16,
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

    // "alternate" profile — note: embedder_type must be "mock" or something that
    // produces a distinct CoreKey. The key field is embedding_model, so we just
    // use a different model name to ensure a distinct key.
    config.profiles.insert(
        "alternate".to_string(),
        vecdb_core::config::Profile {
            qdrant_url: "http://localhost:6334".to_string(),
            default_collection_name: Some("alt-col".to_string()),
            embedding_model: "alternate-model".to_string(),
            accept_invalid_certs: false,
            ollama_url: "http://localhost:11434".to_string(),
            embedder_type: "mock".to_string(),
            qdrant_api_key: None,
            ollama_api_key: None,
            num_ctx: None,
            gpu_batch_size: None,
            quantization: None,
            chunk_size: None,
            max_chunk_size: None,
            chunk_overlap: None,
            resolved_profile_name: "alternate".to_string(),
        },
    );

    // Map "alt-col" to the "alternate" profile.
    config.collections.insert(
        "alt-col".to_string(),
        vecdb_core::config::CollectionConfig {
            name: "alt-col".to_string(),
            description: None,
            profile: Some("alternate".to_string()),
            qdrant_url: None,
            embedder_type: None,
            embedding_model: None,
            num_ctx: None,
            gpu_batch_size: None,
            ollama_url: None,
            chunk_size: None,
            chunk_overlap: None,
            max_chunk_size: None,
            use_gpu: None,
            qdrant_api_key: None,
            ollama_api_key: None,
            quantization: None,
        },
    );
    if let Some(p) = config.profiles.get_mut("default") {
        p.default_collection_name = Some("docs".to_string());
    }

    // ── Pre-seed registry with both Cores ────────────────────────────────────
    let default_profile = config.get_profile(Some("default")).unwrap();
    let key_default = CoreKey::from_resolved(&default_profile, &config);

    let alternate_profile = config.get_profile(Some("alternate")).unwrap();
    let key_alternate = CoreKey::from_resolved(&alternate_profile, &config);

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
                "collection": "docs",
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

    // ── Test: searching "alt-col" routes to alternate backend (MARKER_ALTERNATE)
    let req = JsonRpcRequest {
        jsonrpc: "2.0".to_string(),
        method: "tools/call".to_string(),
        params: Some(json!({
            "name": "search_vectors",
            "arguments": {
                "query": "test",
                "collection": "alt-col",
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
