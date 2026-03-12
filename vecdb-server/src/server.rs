#![allow(clippy::result_large_err)]
use crate::core_registry::CoreRegistry;
use crate::rpc::{handle_request, types::{JsonRpcRequest, JsonRpcResponse}};
use axum::{
    extract::State,
    http::{HeaderMap, HeaderName, HeaderValue, StatusCode},
    response::IntoResponse,
    routing::post,
    Json, Router,
};
use std::{collections::HashMap, sync::Arc};
use tokio::sync::RwLock;
use tracing::info;
use uuid::Uuid;
use vecdb_core::config::Config;

/// MCP protocol versions this server supports.
/// Per 2025-11-25 spec: if MCP-Protocol-Version header is absent, assume 2025-03-26.
const SUPPORTED_PROTOCOL_VERSIONS: &[&str] = &["2025-11-25", "2025-03-26"];

/// Lowercase header names as normalised by hyper/axum.
const HDR_SESSION_ID: &str = "mcp-session-id";
const HDR_PROTOCOL_VERSION: &str = "mcp-protocol-version";

/// Minimal session record. POST /mcp returns synchronous JSON, so we don't
/// need a channel here — session state is just presence in the map.
struct McpSession;

#[derive(Clone)]
pub struct AppState {
    pub registry: Arc<CoreRegistry>,
    pub config: Arc<Config>,
    pub allow_local_fs: bool,
    pub target_profile: String,
    sessions: Arc<RwLock<HashMap<String, McpSession>>>,
}

pub async fn run_http_server(
    registry: Arc<CoreRegistry>,
    config: Arc<Config>,
    allow_local_fs: bool,
    target_profile: String,
    port: u16,
) -> anyhow::Result<()> {
    let state = AppState {
        registry,
        config,
        allow_local_fs,
        target_profile,
        sessions: Arc::new(RwLock::new(HashMap::new())),
    };

    let app = Router::new()
        // Legacy sync endpoint — kept for existing curl / script consumers.
        .route("/", post(rpc_handler))
        // MCP 2025-11-25 Streamable HTTP — single endpoint, three methods.
        .route(
            "/mcp",
            post(mcp_post_handler)
                .get(mcp_get_handler)
                .delete(mcp_delete_handler),
        )
        .layer(tower_http::trace::TraceLayer::new_for_http())
        .with_state(state);

    // Spec (2025-11-25): servers SHOULD bind to 127.0.0.1, not 0.0.0.0.
    let addr = std::net::SocketAddr::from(([127, 0, 0, 1], port));
    info!("Starting HTTP/JSON-RPC server on {} (loopback only)", addr);
    info!("  Legacy sync:      POST http://{}/", addr);
    info!("  Streamable HTTP:  POST/GET/DELETE http://{}/mcp", addr);

    let listener = tokio::net::TcpListener::bind(addr).await?;
    axum::serve(listener, app).await?;

    Ok(())
}

// ─────────────────────────────────────────────────────────────────────────────
// Security helpers
// ─────────────────────────────────────────────────────────────────────────────

/// Validate the `Origin` header to prevent DNS rebinding attacks.
///
/// Per spec (2025-11-25 §Security Warning): servers MUST validate Origin on all
/// incoming connections. If present and not an allowed origin, respond 403.
/// If absent the request is not browser-initiated — allow it.
fn validate_origin(headers: &HeaderMap) -> Result<(), axum::response::Response> {
    let Some(origin) = headers.get("origin") else {
        return Ok(());
    };
    let o = origin.to_str().unwrap_or("");
    if o == "null"
        || o.starts_with("http://localhost")
        || o.starts_with("https://localhost")
        || o.starts_with("http://127.0.0.1")
        || o.starts_with("https://127.0.0.1")
    {
        return Ok(());
    }
    Err((StatusCode::FORBIDDEN, "Origin not allowed").into_response())
}

/// Validate the `MCP-Protocol-Version` header introduced in the 2025-11-25 spec.
///
/// If present it must be a version this server supports.
/// If absent we assume 2025-03-26 per the backwards-compatibility rule.
/// Unsupported value → 400 Bad Request.
fn validate_protocol_version(headers: &HeaderMap) -> Result<(), axum::response::Response> {
    let Some(version) = headers.get(HDR_PROTOCOL_VERSION) else {
        return Ok(());
    };
    let v = version.to_str().unwrap_or("");
    if SUPPORTED_PROTOCOL_VERSIONS.contains(&v) {
        return Ok(());
    }
    Err((
        StatusCode::BAD_REQUEST,
        format!("Unsupported MCP-Protocol-Version: {v}"),
    )
        .into_response())
}

// ─────────────────────────────────────────────────────────────────────────────
// Legacy POST / — unchanged from v1.0.0, kept for curl / script consumers
// ─────────────────────────────────────────────────────────────────────────────

async fn rpc_handler(
    State(state): State<AppState>,
    Json(req): Json<JsonRpcRequest>,
) -> Json<JsonRpcResponse> {
    let id = req.id.clone().unwrap_or(serde_json::Value::Null);
    match handle_request(
        &state.registry,
        &state.config,
        &req,
        state.allow_local_fs,
        &state.target_profile,
    )
    .await
    {
        Ok(res) => Json(JsonRpcResponse {
            jsonrpc: "2.0".to_string(),
            id,
            result: Some(res),
            error: None,
        }),
        Err(err) => Json(JsonRpcResponse {
            jsonrpc: "2.0".to_string(),
            id,
            result: None,
            error: Some(err),
        }),
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// POST /mcp — client sends JSON-RPC to server
// ─────────────────────────────────────────────────────────────────────────────

async fn mcp_post_handler(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(req): Json<JsonRpcRequest>,
) -> axum::response::Response {
    // Security: Origin and protocol version must be valid before anything else.
    if let Err(r) = validate_origin(&headers) {
        return r;
    }
    if let Err(r) = validate_protocol_version(&headers) {
        return r;
    }

    let is_initialize = req.method == "initialize";

    // All non-initialize requests require a valid session.
    // Per spec: server SHOULD respond 400 if MCP-Session-Id is absent (non-init).
    //           server MUST respond 404 if the session is expired/invalid.
    if !is_initialize {
        match headers.get(HDR_SESSION_ID) {
            None => {
                return (StatusCode::BAD_REQUEST, "MCP-Session-Id header required")
                    .into_response();
            }
            Some(sid) => {
                let sid_str = sid.to_str().unwrap_or("");
                if !state.sessions.read().await.contains_key(sid_str) {
                    return (StatusCode::NOT_FOUND, "Session not found or expired")
                        .into_response();
                }
            }
        }
    }

    // Notifications (id absent) and JSON-RPC responses have no reply body.
    // Per spec: server MUST return 202 Accepted with no body.
    if req.id.is_none() {
        return StatusCode::ACCEPTED.into_response();
    }

    // JSON-RPC request — dispatch and return application/json.
    // Per spec: server may return application/json OR text/event-stream.
    // We always return application/json; clients MUST support both (spec §POST).
    let id = req.id.clone().unwrap_or(serde_json::Value::Null);
    let result = handle_request(
        &state.registry,
        &state.config,
        &req,
        state.allow_local_fs,
        &state.target_profile,
    )
    .await;

    let body = match result {
        Ok(res) => JsonRpcResponse {
            jsonrpc: "2.0".to_string(),
            id,
            result: Some(res),
            error: None,
        },
        Err(err) => JsonRpcResponse {
            jsonrpc: "2.0".to_string(),
            id,
            result: None,
            error: Some(err),
        },
    };

    if is_initialize {
        // Create a new session and attach MCP-Session-Id to the response.
        // Per spec: session ID must be globally unique and cryptographically secure.
        // UUID v4 via the `uuid` crate satisfies both requirements.
        let session_id = Uuid::new_v4().to_string();
        state
            .sessions
            .write()
            .await
            .insert(session_id.clone(), McpSession);
        info!("MCP session created: {}", session_id);

        let mut response = Json(body).into_response();
        if let Ok(hv) = HeaderValue::from_str(&session_id) {
            response
                .headers_mut()
                .insert(HeaderName::from_static(HDR_SESSION_ID), hv);
        }
        return response;
    }

    Json(body).into_response()
}

// ─────────────────────────────────────────────────────────────────────────────
// GET /mcp — server-initiated SSE stream
// ─────────────────────────────────────────────────────────────────────────────

/// Server-initiated streaming is not implemented in v1.0.1.
/// Per spec (2025-11-25 §GET): server MUST return text/event-stream OR 405.
/// We return 405; all MCP clients that need this will fall back gracefully.
async fn mcp_get_handler() -> StatusCode {
    StatusCode::METHOD_NOT_ALLOWED
}

// ─────────────────────────────────────────────────────────────────────────────
// DELETE /mcp — client-initiated session termination
// ─────────────────────────────────────────────────────────────────────────────

async fn mcp_delete_handler(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> axum::response::Response {
    if let Err(r) = validate_origin(&headers) {
        return r;
    }
    let Some(sid) = headers.get(HDR_SESSION_ID) else {
        return (StatusCode::BAD_REQUEST, "MCP-Session-Id header required").into_response();
    };
    let sid_str = sid.to_str().unwrap_or("").to_string();
    let removed = state.sessions.write().await.remove(&sid_str).is_some();
    if removed {
        info!("MCP session terminated by client: {}", sid_str);
        StatusCode::OK.into_response()
    } else {
        StatusCode::NOT_FOUND.into_response()
    }
}
