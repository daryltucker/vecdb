// Main JSON-RPC request dispatcher for vecdb-server
// Routes incoming requests to appropriate handlers

use crate::core_registry::CoreRegistry;
use crate::rpc::resources;
use crate::rpc::tools;
use crate::rpc::types::{JsonRpcError, JsonRpcRequest, json_rpc_error};
use schemars::schema_for;
use serde_json::{json, Value};
use std::sync::Arc;
use vecdb_core::config::Config;
use vecdb_core::tools::{
    EmbedArgs, IngestHistoryArgs, IngestPathArgs, JobStatusArgs, SearchArgs, VecqToolArgs,
};

/// Main entry point for handling JSON-RPC requests
pub async fn handle_request(
    registry: &Arc<CoreRegistry>,
    config: &Arc<Config>,
    req: &JsonRpcRequest,
    allow_local_fs: bool,
    active_profile_name: &str,
) -> Result<Value, JsonRpcError> {
    match req.method.as_str() {
        "initialize" => handle_initialize(),
        "notifications/initialized" => Ok(Value::Null),
        "tools/list" => handle_tools_list(),
        "resources/list" => resources::handle_resources_list(registry, config).await,
        "resources/read" => {
            resources::handle_resources_read(registry, config, req, active_profile_name).await
        }
        "tools/call" => {
            tools::handle_tools_call(registry, config, req, allow_local_fs, active_profile_name).await
        }
        _ => Err(json_rpc_error(
            -32601,
            format!("Method '{}' not found", req.method),
        )),
    }
}

/// Handle MCP initialization
fn handle_initialize() -> Result<Value, JsonRpcError> {
    Ok(json!({
        "protocolVersion": "2025-11-25",
        "serverInfo": {
            "name": "vecdb-mcp",
            "version": "0.1.0"
        },
        "capabilities": {
            "tools": {},
            "resources": {}
        }
    }))
}

/// Handle tools/list request
fn handle_tools_list() -> Result<Value, JsonRpcError> {
    let search_schema = schema_for!(SearchArgs);
    let embed_schema = schema_for!(EmbedArgs);
    let ingest_schema = schema_for!(IngestPathArgs);
    let history_schema = schema_for!(IngestHistoryArgs);
    let vecq_schema = schema_for!(VecqToolArgs);
    let job_status_schema = schema_for!(JobStatusArgs);

    let to_json = |val| {
        serde_json::to_value(val).map_err(|e| JsonRpcError {
            code: -32603,
            message: format!("Internal JSON error: {}", e),
            data: None,
        })
    };

    Ok(json!({
        "tools": [
            {
                "name": "search_vectors",
                "description": "Semantic search against vector collections. Returns chunks with content, metadata, and relevance scores.\n\nExample: search_vectors(collection='docs', query='authentication implementation')\n\nWorkflow: Call list_collections first if unsure which collection to query.\nTip: Use specific queries for better results ('implement CORS' vs 'security').",
                "inputSchema": to_json(search_schema)?
            },
            {
                "name": "delete_collection",
                "description": "Delete a collection. Requires explicit confirmation to prevent accidental deletion.\n\nSafety: First call WITHOUT confirmation_code will fail and return the required code.\nExample: delete_collection(collection='old-docs') → Error: 'Use confirmation_code=\"old-docs-DELETE\"'\nThen: delete_collection(collection='old-docs', confirmation_code='old-docs-DELETE') → Success",
                "inputSchema": json!({
                    "type": "object",
                    "properties": {
                        "collection": {
                            "type": "string",
                            "description": "Name of the collection to delete"
                        },
                        "confirmation_code": {
                            "type": "string",
                            "description": "Safety confirmation code. Must be '{collection}-DELETE'. Call once without this to get the required code."
                        }
                    },
                    "required": ["collection"]
                })
            },
            {
                "name": "list_collections",
                "description": "List all available vector collections with metadata (vector count, dimensions).\n\nUse this to discover collections before searching.\n\nWorkflow: Start here → identify target collection → search_vectors.\nReturns: Collection names, dimension checks (is_compatible), and active status.",
                "inputSchema": json!({
                    "type": "object",
                    "properties": {},
                    "required": []
                })
            },
            {
                "name": "embed",
                "description": "Generate embedding vectors from text using the configured model.\n\nExample: embed(texts=['hello world', 'test document'])\n\nUse case: Compare semantic similarity of custom text against collection embeddings.\nReturns: Array of float vectors (dimension depends on model).",
                "inputSchema": to_json(embed_schema)?
            },
            {
                "name": "ingest_path",
                "description": "Ingest local file/directory into a collection. Chunks, embeds, and stores content.\n\nSecurity: Requires server started with --allow-local-fs flag.\n\nExample: ingest_path(path='./docs', collection='my-docs')\n\nWorkflow: Ingest → list_collections (verify) → search_vectors (query).",
                "inputSchema": to_json(ingest_schema)?
            },
            {
                "name": "ingest_history",
                "description": "Ingest a specific git revision ('Time Travel'). Query historical code states.\n\nExample: ingest_history(repo_path='https://github.com/user/repo', git_ref='v1.0.0', collection='repo-v1')\n\nUse case: Compare implementations across versions, understand evolution.\nSafety: Sandboxed execution for security.",
                "inputSchema": to_json(history_schema)?
            },
            {
                "name": "code_query",
                "description": "Execute AST-aware structural queries (jq-style) against code files.\n\nExample: code_query(path='src/main.rs', query='.functions[] | select(.name == \"main\")')\n\nUse case: High-precision extraction of specific code elements (classes, functions, docs) without full file ingestion.\nSupported: Rust (rs), Python (py), Markdown (md), Javascript (js/ts), SQL, Go.",
                "inputSchema": to_json(vecq_schema)?
            },
            {
                "name": "get_job_status",
                "description": "Check the status of background jobs (ingestion, optimization). Returns progress, PID, and status.\n\nExample: get_job_status(id='abc-123')\n\nWorkflow: Start ingestion → use job ID to track progress → search once complete.",
                "inputSchema": to_json(job_status_schema)?
            }
        ]
    }))
}