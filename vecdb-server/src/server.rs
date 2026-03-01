use crate::rpc::{handle_request, types::{JsonRpcRequest, JsonRpcResponse}};
use axum::{
    extract::{Query, State},
    response::{sse::{Event, Sse}, IntoResponse},
    routing::{get, post},
    Json, Router,
};
use std::{collections::HashMap, convert::Infallible, sync::Arc};
use tokio::sync::{mpsc, RwLock};
use tokio_stream::wrappers::ReceiverStream;
use tower_http::cors::CorsLayer;
use tracing::{error, info};
use uuid::Uuid;
use vecdb_core::config::Config;
use vecdb_core::Core;

#[derive(Clone)]
pub struct AppState {
    pub core: Arc<Core>,
    pub config: Arc<Config>,
    pub allow_local_fs: bool,
    pub target_profile: String,
    // Session ID -> Sender for SSE events
    pub sessions: Arc<RwLock<HashMap<String, mpsc::Sender<Result<Event, Infallible>>>>>,
}

pub async fn run_http_server(
    core: Arc<Core>,
    config: Arc<Config>,
    allow_local_fs: bool,
    target_profile: String,
    port: u16,
) -> anyhow::Result<()> {
    let state = AppState {
        core,
        config,
        allow_local_fs,
        target_profile,
        sessions: Arc::new(RwLock::new(HashMap::new())),
    };

    let app = Router::new()
        .route("/", post(rpc_handler))          // Legacy sync endpoint
        .route("/sse", get(sse_handler).post(message_handler)) // MCP SSE connection AND potential POST fallback
        .route("/message", post(message_handler)) // MCP SSE message input
        .layer(
            CorsLayer::new()
                .allow_origin(tower_http::cors::Any)
                .allow_methods(tower_http::cors::Any)
                .allow_headers(tower_http::cors::Any),
        )
        .layer(tower_http::trace::TraceLayer::new_for_http())
        .with_state(state);

    let addr = std::net::SocketAddr::from(([0, 0, 0, 0], port));
    info!("Starting HTTP/JSON-RPC server on {}", addr);

    let listener = tokio::net::TcpListener::bind(addr).await?;
    axum::serve(listener, app).await?;

    Ok(())
}

async fn rpc_handler(
    State(state): State<AppState>,
    Json(req): Json<JsonRpcRequest>,
) -> Json<JsonRpcResponse> {
    let id = req.id.clone().unwrap_or(serde_json::Value::Null);

    match handle_request(
        &state.core,
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

async fn sse_handler(
    State(state): State<AppState>,
) -> Sse<impl tokio_stream::Stream<Item = Result<Event, Infallible>>> {
    let session_id = Uuid::new_v4().to_string();
    let (tx, rx) = mpsc::channel(100);

    state.sessions.write().await.insert(session_id.clone(), tx.clone());

    // Send the required "endpoint" event to let the client know where to POST messages
    let endpoint_uri = format!("/message?sessionId={}", session_id);
    let _ = tx.send(Ok(Event::default().event("endpoint").data(endpoint_uri))).await;

    info!("New SSE Connection established: {}", session_id);

    // Clean up when the client disconnects
    // We can't trivially wait for disconnect in axum SSE without a custom stream wrapper,
    // but the receiver will drop when the connection is closed, and we can periodically
    // clean up or rely on the channel send failing in message_handler.
    
    let stream = ReceiverStream::new(rx);
    Sse::new(stream).keep_alive(axum::response::sse::KeepAlive::default())
}

#[derive(serde::Deserialize)]
struct MessageQuery {
    #[serde(rename = "sessionId")]
    session_id: String,
}

// Responds with 202 Accepted and processes the request asynchronously,
// sending the response back via the SSE channel.
async fn message_handler(
    State(state): State<AppState>,
    Query(query): Query<MessageQuery>,
    Json(req): Json<JsonRpcRequest>,
) -> axum::response::Response {
    let session_id = query.session_id;

    let target_tx = {
        let sessions = state.sessions.read().await;
        if let Some(tx) = sessions.get(&session_id) {
            tx.clone() // Clone the sender so we can drop the read lock
        } else {
            return (
                axum::http::StatusCode::BAD_REQUEST,
                "Invalid or expired sessionId",
            )
                .into_response();
        }
    };

    let req_id = req.id.clone().unwrap_or(serde_json::Value::Null);

    // Spawn an async task to process the request so we can return 202 immediately
    tokio::spawn(async move {
        // Execute the RPC logic
        let response = match handle_request(
            &state.core,
            &state.config,
            &req,
            state.allow_local_fs,
            &state.target_profile,
        )
        .await
        {
            Ok(res) => JsonRpcResponse {
                jsonrpc: "2.0".to_string(),
                id: req_id,
                result: Some(res),
                error: None,
            },
            Err(err) => JsonRpcResponse {
                jsonrpc: "2.0".to_string(),
                id: req_id,
                result: None,
                error: Some(err),
            },
        };

        // Format to JSON
        let json_str = match serde_json::to_string(&response) {
            Ok(s) => s,
            Err(e) => {
                error!("Failed to serialize JSON-RPC response: {}", e);
                return;
            }
        };

        // Send over SSE (MCP specification requires event name "message")
        let event = Event::default().event("message").data(json_str);
        if target_tx.send(Ok(event)).await.is_err() {
            info!("SSE Session {} disconnected, cleaning up.", session_id);
            // Clean up the session since the send failed
            state.sessions.write().await.remove(&session_id);
        }
    });

    axum::http::StatusCode::ACCEPTED.into_response()
}
