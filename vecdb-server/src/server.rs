use axum::{
    extract::State,
    routing::post,
    Json, Router,
};
use std::sync::Arc;
use vecdb_core::Core;
use vecdb_core::config::Config;
use crate::handler::{handle_request, JsonRpcRequest, JsonRpcResponse};
use tower_http::cors::CorsLayer;
use tracing::info;

#[derive(Clone)]
pub struct AppState {
    pub core: Arc<Core>,
    pub config: Arc<Config>,
    pub allow_local_fs: bool,
    pub target_profile: String,
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
    };

    let app = Router::new()
        .route("/", post(rpc_handler))
        .layer(CorsLayer::permissive())
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
    ).await {
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
