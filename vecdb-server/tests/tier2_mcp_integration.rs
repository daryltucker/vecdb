
use vecdb_core::Core;
use vecdb_core::config::Config;
use std::sync::{Arc, Mutex};
use serde_json::json;
use vecdb_server::handler::{handle_request, JsonRpcRequest};
use vecdb_core::parsers::ParserFactory;
use vecq::detection::HybridDetector;

mod common;
use common::{MockBackend, MockEmbedder};

struct MockParserFactory;
impl ParserFactory for MockParserFactory {
    fn get_parser(&self, _file_type: vecdb_common::FileType) -> Option<Box<dyn vecdb_core::parsers::Parser>> {
        None
    }
}

#[tokio::test]
async fn test_mcp_full_lifecycle() {
    // 1. Setup Mock Core
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

    let backend = Arc::new(MockBackend { storage: storage.clone() });
    let embedder = Arc::new(MockEmbedder);
    let detector = Arc::new(HybridDetector::new());
    let parser_factory = Arc::new(MockParserFactory);
    
    let core = Arc::new(Core::with_backends(backend, embedder, detector, parser_factory));
    let config = Config::default();

    // 2. Initialize
    let req = JsonRpcRequest {
        jsonrpc: "2.0".to_string(),
        method: "initialize".to_string(),
        params: None,
        id: Some(json!(1)),
    };
    let res = handle_request(&core, &config, &req, false, "default").await.unwrap();
    assert_eq!(res["serverInfo"]["name"], "vecdb-mcp");

    // 3. List Collections (Mock verification)
    let req = JsonRpcRequest {
        jsonrpc: "2.0".to_string(),
        method: "tools/call".to_string(),
        params: Some(json!({
            "name": "list_collections",
            "arguments": {}
        })),
        id: Some(json!(2)),
    };
    let res = handle_request(&core, &config, &req, false, "default").await.unwrap();
    let content = res["content"][0]["text"].as_str().unwrap();
    
    // Validate Dimension Probe fields
    // MockBackend returns "test_collection" with vector_size 384
    // MockEmbedder returns dimension 384
    // So is_compatible should be true
    assert!(content.contains(r#""is_compatible": true"#));
    assert!(content.contains(r#""is_active": true"#));
    assert!(content.contains("docs"));

    // 4. Ingest (Should work with mock)
    // Note: ingest_path requires allow_local_fs=true usually, but here we control the flag
    // MockBackend does not really touch FS for "ingest", but Core's ingest logic MIGHT if we use ingest_path.
    // However, Core::ingest does file walking.
    // Instead of testing `ingest_path` (which hits FS), let's test `embed` which hits MockEmbedder.
    
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
    let res = handle_request(&core, &config, &req, false, "default").await.unwrap();
    let content = res["content"][0]["text"].as_str().unwrap();
    assert!(content.contains("0.1")); // Mock returns [0.1, 0.2, 0.3]

    // 5. Search (Mock verification)
    let req = JsonRpcRequest {
        jsonrpc: "2.0".to_string(),
        method: "tools/call".to_string(),
        params: Some(json!({
            "name": "search_vectors",
            "arguments": {
                "query": "something",
                "collection": "docs",
                "profile": "default", // Test explicit profile arg
                "json": false,
                "smart": false
            }
        })),
        id: Some(json!(4)),
    };
    let res = handle_request(&core, &config, &req, false, "default").await.unwrap();
    let content = res["content"][0]["text"].as_str().unwrap();
    assert!(content.contains("0.99")); // Mock score
}
