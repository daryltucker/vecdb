/*
 * PURPOSE:
 *   Regression test for the HTTP transport's panic boundary (audit finding C-5).
 *
 * THE BUG:
 *   `vecdb-server` had asymmetric panic safety. The stdio transport
 *   (`main.rs`) spawns each request as a tokio task and converts
 *   `JoinError::is_panic()` into a JSON-RPC -32000 error, so the loop survives.
 *   The HTTP transport (`server.rs`) awaited `handle_request` directly with no
 *   `catch_unwind` and no `CatchPanicLayer` in the stack — a panicking tool
 *   handler aborted the connection and the client got nothing parseable.
 *
 *   README recommends HTTP transport for multi-agent setups, so the
 *   *recommended* path was the *less robust* one.
 *
 * WHAT THIS ASSERTS:
 *   1. A panic raised deep inside a tool handler produces a well-formed
 *      JSON-RPC 2.0 error body, not a dropped connection or a bare-text 500.
 *   2. The error `code` is -32000 and the `message` prefix matches the string
 *      the stdio boundary emits in `main.rs` — transport parity.
 *   3. The boundary covers ALL routes, not just `rpc_handler`: `POST /` and
 *      `POST /mcp` are both exercised. (`GET`/`DELETE /mcp` share the same
 *      router-level layer by construction.)
 *
 * WHY `oneshot` AND NOT A REAL SOCKET:
 *   `build_router()` is the single construction site used by
 *   `run_http_server`, so driving it with `tower::ServiceExt::oneshot`
 *   exercises the *shipped* middleware stack while binding no port — which
 *   matters because this suite must run with no Qdrant and no network.
 */

use async_trait::async_trait;
use axum::body::Body;
use axum::http::{Request, StatusCode};
use serde_json::json;
use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use tower::ServiceExt; // for `oneshot`
use vecdb_core::backend::Backend;
use vecdb_core::config::Config;
use vecdb_core::parsers::ParserFactory;
use vecdb_core::types::{CollectionInfo, SearchResult};
use vecdb_core::Core;
use vecdb_server::core_registry::{CoreKey, CoreRegistry};
use vecdb_server::server::{build_router, AppState};
use vecq::detection::HybridDetector;

mod common;
use common::{MockBackend, MockEmbedder};

/// Never a production collection name. `tier0_qdrant_isolation.py` scans test
/// sources for bare names like `docs` and fails the run, because a collection
/// without the prefix cannot be safely purged between runs.
const TEST_COLLECTION: &str = "test_http_panic_boundary";

/// MockEmbedder's dimension. The genesis must claim the same number.
const MOCK_DIM: u64 = 3;

/// Panic marker. Asserted on verbatim so a future refactor that swallows the
/// payload (returning only "unknown panic") fails loudly instead of silently
/// degrading the diagnostic.
const PANIC_MARKER: &str = "deliberate test panic in tool handler";

struct MockParserFactory;
impl ParserFactory for MockParserFactory {
    fn get_parser(
        &self,
        _file_type: vecdb_common::FileType,
    ) -> Option<Box<dyn vecdb_core::parsers::Parser>> {
        None
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// A backend that panics on `list_collections`.
//
// This is the realistic shape of the bug: not a panic in the transport layer,
// but one raised several frames deep inside a tool implementation (a bug in a
// dependency, an unexpected file path, a slice index). It must not escape as a
// connection abort.
// ─────────────────────────────────────────────────────────────────────────────
struct PanickingBackend;

#[async_trait]
impl Backend for PanickingBackend {
    async fn health_check(&self) -> anyhow::Result<()> {
        Ok(())
    }
    async fn create_collection(
        &self,
        _n: &str,
        _v: u64,
        _q: Option<vecdb_core::config::QuantizationType>,
    ) -> anyhow::Result<()> {
        Ok(())
    }
    async fn collection_exists(&self, _n: &str) -> anyhow::Result<bool> {
        Ok(true)
    }
    async fn delete_collection(&self, _n: &str) -> anyhow::Result<()> {
        Ok(())
    }
    async fn upsert(&self, _c: &str, _chunks: Vec<vecdb_core::types::Chunk>) -> anyhow::Result<()> {
        Ok(())
    }
    /// The panic site.
    ///
    /// `search` is chosen deliberately over `list_collections`: the latter
    /// bypasses the registry's Core and opens its own Qdrant clients from
    /// config, which would both miss this mock *and* touch a live Qdrant. This
    /// test must run with no network and no Qdrant of any kind.
    async fn search(
        &self,
        _c: &str,
        _v: &[f32],
        _params: vecdb_core::backend::SearchParams,
    ) -> anyhow::Result<Vec<SearchResult>> {
        panic!("{}", PANIC_MARKER);
    }

    async fn points_exists(&self, _c: &str, _ids: Vec<String>) -> anyhow::Result<Vec<String>> {
        Ok(vec![])
    }
    async fn list_collections(&self) -> anyhow::Result<Vec<String>> {
        Ok(vec![TEST_COLLECTION.to_string()])
    }

    async fn get_collection_info(&self, name: &str) -> anyhow::Result<CollectionInfo> {
        Ok(CollectionInfo {
            name: name.to_string(),
            vector_count: Some(0),
            vector_size: Some(3),
            quantization: None,
            vectors_on_disk: None,
            payload_on_disk: None,
        })
    }
    async fn list_metadata_values(&self, _: &str, _: &str) -> anyhow::Result<Vec<String>> {
        Ok(vec![])
    }
    async fn update_collection_quantization(
        &self,
        _: &str,
        _: vecdb_core::config::QuantizationType,
    ) -> anyhow::Result<()> {
        Ok(())
    }
    async fn get_collection_id(&self, _c: &str) -> anyhow::Result<Option<String>> {
        Ok(None)
    }
    async fn set_collection_id(&self, _c: &str, _id: &str) -> anyhow::Result<()> {
        Ok(())
    }
    // Renamed in v1.1.0: write_genesis_metadata -> write_genesis,
    // get_collection_metadata -> read_genesis.
    async fn write_genesis(
        &self,
        _c: &str,
        _m: &vecdb_core::types::GenesisMetadata,
    ) -> anyhow::Result<()> {
        Ok(())
    }
    /// Must report a PRESENT genesis.
    ///
    /// `CollectionGenesis::default()` has `vecdb_version: None`, which means
    /// "this collection is not vecdb's". The search path rejects that before it
    /// ever reaches `Backend::search`, so the handler returned a tidy
    /// "not a vecdb collection" error and the panic under test never fired —
    /// the test passed through the guard it was trying to get past.
    async fn read_genesis(&self, _c: &str) -> anyhow::Result<vecdb_core::types::CollectionGenesis> {
        Ok(vecdb_core::types::CollectionGenesis {
            vecdb_version: Some(env!("CARGO_PKG_VERSION").to_string()),
            // Must agree with MockEmbedder, or the embedding-space guard
            // rejects the search on a dimension mismatch — again short of the
            // panic this test exists to observe.
            dimension: Some(MOCK_DIM),
            // Mirrors MockEmbedder::identity() exactly. The guard compares
            // digests first (Tier 1); without one it refuses with
            // "insufficient identity to compare" rather than guessing, so a
            // near-match here still never reaches the panic.
            model: vecdb_core::types::ModelIdentity {
                name: "mock-embedder".to_string(),
                digest: Some("mock:test-double".to_string()),
                architecture: Some("mock".to_string()),
                family: Some("mock".to_string()),
                parameter_size: Some("0".to_string()),
                quantization_level: Some("none".to_string()),
                embedding_length: None,
                context_length: Some(8192),
            },
            ..Default::default()
        })
    }
    // Added in v1.1.0; the mock never ingests, so nothing is ever stale.
    async fn delete_stale_points(
        &self,
        _collection: &str,
        _document_id: &str,
        _keep: &[String],
    ) -> anyhow::Result<usize> {
        Ok(0)
    }
    async fn list_tasks(&self) -> anyhow::Result<Vec<vecdb_core::types::TaskInfo>> {
        Ok(vec![])
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Helpers
// ─────────────────────────────────────────────────────────────────────────────

fn make_state(backend: Arc<dyn Backend>) -> AppState {
    let core = Arc::new(Core::with_backends(
        backend,
        Arc::new(MockEmbedder),
        Arc::new(HybridDetector::new()),
        Arc::new(MockParserFactory),
        Vec::new(),
        Vec::new(),
        1,
        10,
    ));

    // v1.1.0: `default_collection_name` is retired and everything resolves
    // through Config::resolve, which returns the value AND its source layer.
    let config = Config::default();
    let resolution = config
        .resolve(Some("default"), Some(TEST_COLLECTION))
        .expect("default profile must resolve");
    let key = CoreKey::from_resolution(&resolution);
    let mut cores = HashMap::new();
    cores.insert(key, core);
    let registry = Arc::new(CoreRegistry::from_map(cores, "default"));

    AppState::new(registry, Arc::new(config), false, "default".to_string())
}

/// Build a `search_vectors` tools/call — the request that reaches
/// `Backend::search` on the mock Core, and therefore the panic site.
fn search_request(uri: &str) -> Request<Body> {
    let body = json!({
        "jsonrpc": "2.0",
        "method": "tools/call",
        "params": {
            "name": "search_vectors",
            "arguments": {
                "query": "anything",
                "collection": TEST_COLLECTION,
                "profile": "default",
                "json": false,
                "smart": false
            }
        },
        "id": 1
    });
    Request::builder()
        .method("POST")
        .uri(uri)
        .header("content-type", "application/json")
        .body(Body::from(body.to_string()))
        .unwrap()
}

async fn body_json(res: axum::response::Response) -> serde_json::Value {
    let bytes = axum::body::to_bytes(res.into_body(), 1024 * 1024)
        .await
        .expect("response body must be readable — a dropped connection fails here");
    serde_json::from_slice(&bytes).expect("response body must be valid JSON, not bare text")
}

/// Shared assertions: the response must be a JSON-RPC 2.0 error object whose
/// shape matches what the stdio transport produces in `main.rs`.
fn assert_json_rpc_panic_parity(v: &serde_json::Value) {
    assert_eq!(v["jsonrpc"], "2.0", "must be a JSON-RPC 2.0 envelope");
    assert!(
        v.get("result").is_none() || v["result"].is_null(),
        "a panic must not produce a result field"
    );

    let err = v.get("error").expect("must carry a JSON-RPC error object");

    // Parity point 1: same code as main.rs:292.
    assert_eq!(err["code"], -32000, "code must match the stdio boundary");

    // Parity point 2: same message construction as main.rs:293
    // (`format!("Internal error (panic): {msg}")`).
    let msg = err["message"].as_str().expect("message must be a string");
    assert!(
        msg.starts_with("Internal error (panic): "),
        "message prefix must match the stdio boundary, got: {msg}"
    );

    // Parity point 3: the original panic payload survives, so the error is
    // actually diagnostic rather than a generic 500.
    assert!(
        msg.contains(PANIC_MARKER),
        "panic payload must be preserved in the message, got: {msg}"
    );
}

// ─────────────────────────────────────────────────────────────────────────────
// Route coverage: POST /  (legacy sync endpoint, `rpc_handler`)
// ─────────────────────────────────────────────────────────────────────────────
#[tokio::test]
async fn panic_in_handler_returns_json_rpc_error_on_legacy_root() {
    let app = build_router(make_state(Arc::new(PanickingBackend)));

    let res = app
        .oneshot(search_request("/"))
        .await
        .expect("the service must return a response, not an error, on panic");

    // 200, not 500, and deliberately so: a JSON-RPC error is a SUCCESSFUL
    // transport carrying an application-level failure. Every other error on
    // this path already returns 200 with an error object, and a 500 would tell
    // a JSON-RPC client to stop parsing the body that holds the actual answer.
    // Parity with the stdio boundary is about the ERROR OBJECT, asserted below.
    assert_eq!(
        res.status(),
        StatusCode::OK,
        "a JSON-RPC error must still be delivered as a parseable 200 response"
    );

    let v = body_json(res).await;
    assert_json_rpc_panic_parity(&v);
}

// ─────────────────────────────────────────────────────────────────────────────
// Route coverage: POST /mcp  (`mcp_post_handler`)
//
// C-5 explicitly called out that `mcp_post_handler` and `mcp_delete_handler`
// are separate handlers from `rpc_handler`. The layer is applied at the Router
// level so all of them are covered; this test proves it for /mcp rather than
// assuming it.
// ─────────────────────────────────────────────────────────────────────────────
#[tokio::test]
async fn panic_in_handler_returns_json_rpc_error_on_mcp_endpoint() {
    let app = build_router(make_state(Arc::new(PanickingBackend)));

    // `tools/call` is a non-initialize method, so /mcp demands a session.
    // Drive `initialize` first to mint one.
    let init = Request::builder()
        .method("POST")
        .uri("/mcp")
        .header("content-type", "application/json")
        .body(Body::from(
            json!({"jsonrpc":"2.0","method":"initialize","params":{},"id":0}).to_string(),
        ))
        .unwrap();

    let init_res = app.clone().oneshot(init).await.unwrap();
    assert_eq!(init_res.status(), StatusCode::OK);
    let session_id = init_res
        .headers()
        .get("mcp-session-id")
        .expect("initialize must mint a session id")
        .to_str()
        .unwrap()
        .to_string();

    let mut req = search_request("/mcp");
    req.headers_mut()
        .insert("mcp-session-id", session_id.parse().unwrap());

    let res = app
        .oneshot(req)
        .await
        .expect("the service must return a response, not an error, on panic");

    // Same reasoning as the legacy endpoint: a JSON-RPC error rides a 200.
    assert_eq!(
        res.status(),
        StatusCode::OK,
        "both endpoints must deliver the error identically"
    );

    let v = body_json(res).await;
    assert_json_rpc_panic_parity(&v);
}

// ─────────────────────────────────────────────────────────────────────────────
// Negative control: without a panic the boundary must be invisible.
//
// A CatchPanicLayer that turned healthy responses into 500s would pass the two
// tests above while destroying the server. This pins the happy path.
// ─────────────────────────────────────────────────────────────────────────────
#[tokio::test]
async fn non_panicking_handler_is_unaffected_by_the_layer() {
    let backend = Arc::new(MockBackend {
        storage: Arc::new(Mutex::new(Vec::new())),
    });
    let app = build_router(make_state(backend));

    let res = app.oneshot(search_request("/")).await.unwrap();

    assert_eq!(res.status(), StatusCode::OK);
    let v = body_json(res).await;
    assert_eq!(v["jsonrpc"], "2.0");
    assert!(
        v.get("error").is_none() || v["error"].is_null(),
        "healthy request must not carry an error: {v}"
    );
}
