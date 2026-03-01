// Tool call handlers for vecdb-server JSON-RPC interface
// Handles all tools/call requests by dispatching to individual tool handlers

use crate::rpc::types::{JsonRpcError, JsonRpcRequest};
use serde_json::{json, Value};
use std::sync::Arc;
use vecdb_core::config::Config;
use vecdb_core::tools::{
    EmbedArgs, IngestHistoryArgs, IngestPathArgs, JobStatusArgs, SearchArgs, VecqToolArgs,
};
use vecdb_core::Core;

/// Handle tools/call request by dispatching to individual tool handlers
pub async fn handle_tools_call(
    core: &Arc<Core>,
    config: &Config,
    req: &JsonRpcRequest,
    allow_local_fs: bool,
    active_profile_name: &str,
) -> Result<Value, JsonRpcError> {
    let params = req.params.as_ref().ok_or(JsonRpcError {
        code: -32602,
        message: "Missing params".into(),
        data: None,
    })?;

    let name = params["name"].as_str().ok_or(JsonRpcError {
        code: -32602,
        message: "Missing name".into(),
        data: None,
    })?;

    match name {
        "search_vectors" => handle_search_vectors(core, config, params, active_profile_name).await,
        "delete_collection" => handle_delete_collection(core, params).await,
        "list_collections" => handle_list_collections(core, config, active_profile_name).await,
        "embed" => handle_embed(core, params).await,
        "ingest_path" => handle_ingest_path(core, config, params, allow_local_fs, active_profile_name).await,
        "ingest_history" => handle_ingest_history(core, config, params, allow_local_fs, active_profile_name).await,
        "code_query" => handle_code_query(params, allow_local_fs).await,
        "get_job_status" => handle_get_job_status(core, params).await,
        _ => Err(JsonRpcError {
            code: -32601,
            message: format!("Tool not found: {}", name),
            data: None,
        }),
    }
}

/// Handle search_vectors tool
async fn handle_search_vectors(
    core: &Arc<Core>,
    config: &Config,
    params: &Value,
    active_profile_name: &str,
) -> Result<Value, JsonRpcError> {
    let args_val = &params["arguments"];
    let args: SearchArgs =
        serde_json::from_value(args_val.clone()).map_err(|e| JsonRpcError {
            code: -32602,
            message: format!("Invalid arguments for search: {}", e),
            data: None,
        })?;

    // Resolve collection using profile argument (if specified) or server default
    // Note: Server uses the BOOT embedder for all operations (single embedder per process)
    // Profile argument is used only for collection namespace resolution
    let profile_name = args.profile.as_deref().unwrap_or(active_profile_name);
    let profile = config
        .get_profile(Some(profile_name))
        .map_err(|e| JsonRpcError {
            code: -32000,
            message: format!("Profile '{}' not found: {}", profile_name, e),
            data: None,
        })?;

    // Resolve final collection: explicit > profile default
    let collection = args
        .collection
        .as_deref()
        .or(profile.default_collection_name.as_deref())
        .ok_or_else(|| JsonRpcError {
            code: -32602,
            message: "collection is required: provide it in the request or configure a collection with this profile".into(),
            data: None,
        })?;

    let results = if args.smart {
        core.search_smart(collection, &args.query, 10).await
    } else {
        core.search(collection, &args.query, 10, None).await
    }
    .map_err(|e| JsonRpcError {
        code: -32000,
        message: e.to_string(),
        data: None,
    })?;

    Ok(json!({
        "content": [
            {
                "type": "text",
                "text": serde_json::to_string(&results).map_err(|e| JsonRpcError {
                    code: -32603,
                    message: format!("Serialization error: {}", e),
                    data: None,
                })?
            }
        ]
    }))
}

/// Handle delete_collection tool
async fn handle_delete_collection(
    core: &Arc<Core>,
    params: &Value,
) -> Result<Value, JsonRpcError> {
    let args_val = &params["arguments"];
    let args: Value =
        serde_json::from_value(args_val.clone()).map_err(|e| JsonRpcError {
            code: -32602,
            message: format!("Invalid arguments for delete_collection: {}", e),
            data: None,
        })?;

    let collection =
        args.get("collection")
            .and_then(|v| v.as_str())
            .ok_or_else(|| JsonRpcError {
                code: -32602,
                message: "collection argument is required".into(),
                data: None,
            })?;
    let confirmation = args
        .get("confirmation_code")
        .and_then(|v| v.as_str())
        .unwrap_or("");

    let expected_code = format!("{}-DELETE", collection);

    if confirmation != expected_code {
        return Err(JsonRpcError {
            code: -32000,
            message: format!(
                "SAFETY LOCK ACTIVE. To confirm deletion of '{}', re-run this tool with confirmation_code='{}'.",
                collection, expected_code
            ),
            data: None,
        });
    }

    core.delete_collection(collection)
        .await
        .map_err(|e| JsonRpcError {
            code: -32000,
            message: e.to_string(),
            data: None,
        })?;

    Ok(json!({
        "status": "success",
        "message": format!("Collection '{}' deleted successfully", collection)
    }))
}

/// Handle list_collections tool
async fn handle_list_collections(
    core: &Arc<Core>,
    config: &Config,
    active_profile_name: &str,
) -> Result<Value, JsonRpcError> {
    let collections = core.list_collections().await.map_err(|e| JsonRpcError {
        code: -32000,
        message: e.to_string(),
        data: None,
    })?;

    // Get default collection for the active profile
    let profile = config
        .get_profile(Some(active_profile_name))
        .or_else(|_| config.get_profile(None))
        .map_err(|e| JsonRpcError {
            code: -32000,
            message: format!("Failed to resolve profile: {}", e),
            data: None,
        })?;

    // Probe current embedding dimension
    // If this fails (e.g. Ollama down), we can't determine compatibility, so default to None/false
    let current_dim = core.get_embedding_dimension().await.ok();

    let response_data = json!({
        "active_profile": active_profile_name,
        "default_collection": profile.default_collection_name,
        "collections": collections.into_iter().map(|c| {
            let is_compatible = match (current_dim, c.vector_size) {
                (Some(curr), Some(stored)) => curr as u64 == stored,
                _ => false, // Cannot determine compatibility
            };

            json!({
                "name": c.name,
                "count": c.vector_count,
                "dimension": c.vector_size,
                "is_active": profile.default_collection_name.as_deref() == Some(c.name.as_str()),
                "is_compatible": is_compatible
            })
        }).collect::<Vec<_>>()
    });

    Ok(json!({
        "content": [
            {
                "type": "text",
                "text": serde_json::to_string_pretty(&response_data).map_err(|e| JsonRpcError {
                    code: -32603,
                    message: format!("Serialization error: {}", e),
                    data: None,
                })?
            }
        ]
    }))
}

/// Handle embed tool
async fn handle_embed(
    core: &Arc<Core>,
    params: &Value,
) -> Result<Value, JsonRpcError> {
    let args_val = &params["arguments"];
    let args: EmbedArgs =
        serde_json::from_value(args_val.clone()).map_err(|e| JsonRpcError {
            code: -32602,
            message: format!("Invalid arguments for embed: {}", e),
            data: None,
        })?;

    let embeddings = core.embed(args.texts).await.map_err(|e| JsonRpcError {
        code: -32000,
        message: e.to_string(),
        data: None,
    })?;

    Ok(json!({
        "content": [
            {
                "type": "text",
                "text": serde_json::to_string(&embeddings).map_err(|e| JsonRpcError {
                    code: -32603,
                    message: format!("Serialization error: {}", e),
                    data: None,
                })?
            }
        ]
    }))
}

/// Handle ingest_path tool
async fn handle_ingest_path(
    core: &Arc<Core>,
    config: &Config,
    params: &Value,
    allow_local_fs: bool,
    active_profile_name: &str,
) -> Result<Value, JsonRpcError> {
    if !allow_local_fs {
        return Err(JsonRpcError {
            code: -32000,
            message: "Security Error: Local filesystem access is disabled. Start server with --allow-local-fs to enable.".into(),
            data: None,
        });
    }

    let args_val = &params["arguments"];
    let args: IngestPathArgs =
        serde_json::from_value(args_val.clone()).map_err(|e| JsonRpcError {
            code: -32602,
            message: format!("Invalid arguments for ingest_path: {}", e),
            data: None,
        })?;

    // Resolve collection using profile argument (if specified) or server default
    let profile_name = args.profile.as_deref().unwrap_or(active_profile_name);
    let profile = config
        .resolve_profile(Some(profile_name), args.collection.as_deref())
        .map_err(|e| JsonRpcError {
            code: -32000,
            message: format!("Profile '{}' not found: {}", profile_name, e),
            data: None,
        })?;

    // Resolve max_chunk_size and overlap from config if available (server config)
    let max_chunk_size = config.resolve_max_chunk_size(&profile, args.collection.as_deref());
    let chunk_overlap = config.resolve_chunk_overlap(&profile, args.collection.as_deref());

    // Resolve final collection: explicit > profile default
    let collection = args
        .collection
        .as_deref()
        .or(profile.default_collection_name.as_deref())
        .ok_or_else(|| JsonRpcError {
            code: -32602,
            message: "collection is required: provide it in the request or configure a collection with this profile".into(),
            data: None,
        })?;

    core.ingest(
        &args.path,
        collection,
        true,
        None,
        max_chunk_size,
        Some(chunk_overlap),
        None,
        None,
        false,
        None,
        args.concurrency,
        args.gpu_concurrency,
        profile.quantization.clone(),
        None,
    )
    .await
    .map_err(|e| JsonRpcError {
        code: -32000,
        message: e.to_string(),
        data: None,
    })?;

    Ok(json!({
        "content": [
            {
                "type": "text",
                "text": format!("Successfully ingested {}", args.path)
            }
        ]
    }))
}

/// Handle ingest_history tool
async fn handle_ingest_history(
    core: &Arc<Core>,
    config: &Config,
    params: &Value,
    allow_local_fs: bool,
    active_profile_name: &str,
) -> Result<Value, JsonRpcError> {
    let args_val = &params["arguments"];
    let args: IngestHistoryArgs =
        serde_json::from_value(args_val.clone()).map_err(|e| JsonRpcError {
            code: -32602,
            message: format!("Invalid arguments for {}: {}", "ingest_history", e),
            data: None,
        })?;

    // Simple security check
    let is_remote =
        args.repo_path.starts_with("http") || args.repo_path.starts_with("git@");
    if !is_remote && !allow_local_fs {
        return Err(JsonRpcError {
            code: -32000,
            message: "Security Error: Local filesystem access is disabled. Cannot ingest local repo history.".into(),
            data: None,
        });
    }

    // Resolve collection using profile argument (if specified) or server default
    let profile_name = args.profile.as_deref().unwrap_or(active_profile_name);
    let profile = config
        .resolve_profile(Some(profile_name), args.collection.as_deref())
        .map_err(|e| JsonRpcError {
            code: -32000,
            message: format!("Profile '{}' not found: {}", profile_name, e),
            data: None,
        })?;

    // Resolve final collection: explicit > profile default
    let collection = args
        .collection
        .as_deref()
        .or(profile.default_collection_name.as_deref())
        .ok_or_else(|| JsonRpcError {
            code: -32602,
            message: "collection is required: provide it in the request or configure a collection with this profile".into(),
            data: None,
        })?;

    core.ingest_history(
        &args.repo_path,
        &args.git_ref,
        collection,
        512,
        profile.quantization.clone(),
        None,
    )
    .await
    .map_err(|e| JsonRpcError {
        code: -32000,
        message: e.to_string(),
        data: None,
    })?;

    Ok(json!({
        "content": [
            {
                "type": "text",
                "text": format!("Successfully ingested history version {} from {}", args.git_ref, args.repo_path)
            }
        ]
    }))
}

/// Handle code_query tool
async fn handle_code_query(
    params: &Value,
    allow_local_fs: bool,
) -> Result<Value, JsonRpcError> {
    let args_val = &params["arguments"];
    let args: VecqToolArgs =
        serde_json::from_value(args_val.clone()).map_err(|e| JsonRpcError {
            code: -32602,
            message: format!("Invalid arguments for code_query: {}", e),
            data: None,
        })?;

    if args.source.as_deref().unwrap_or("local") == "local" && !allow_local_fs {
        return Err(JsonRpcError {
            code: -32000,
            message: "Security Error: Local filesystem access is disabled. Cannot query local files.".into(),
            data: None,
        });
    }

    let result = if args.source.as_deref().unwrap_or("local") == "local" {
        let path = std::path::Path::new(&args.path);
        if !path.exists() {
            return Err(JsonRpcError {
                code: -32000,
                message: format!("File not found: {}", args.path),
                data: None,
            });
        }

        let file_type = vecq::detect_file_type(&args.path);
        let content = std::fs::read_to_string(path).map_err(|e| JsonRpcError {
            code: -32000,
            message: format!("Failed to read file: {}", e),
            data: None,
        })?;

        let parsed =
            vecq::parse_file(&content, file_type)
                .await
                .map_err(|e| JsonRpcError {
                    code: -32000,
                    message: format!("Parse error: {}", e),
                    data: None,
                })?;

        let json = vecq::convert_to_json(parsed).map_err(|e| JsonRpcError {
            code: -32000,
            message: format!("Json conversion error: {}", e),
            data: None,
        })?;

        match vecq::query_json(&json, &args.query) {
            Ok(results) => results
                .iter()
                .map(|v| v.as_str().unwrap_or(&v.to_string()).to_string())
                .collect::<Vec<_>>()
                .join("\n"),
            Err(e) => {
                return Err(JsonRpcError {
                    code: -32000,
                    message: format!("Query error: {}", e),
                    data: None,
                })
            }
        }
    } else {
        return Err(JsonRpcError {
            code: -32000,
            message: "Remote git query not yet implemented in decoupled server.".into(),
            data: None,
        });
    };

    Ok(json!({
        "content": [
            {
                "type": "text",
                "text": result
            }
        ]
    }))
}

/// Handle get_job_status tool
async fn handle_get_job_status(
    core: &Arc<Core>,
    params: &Value,
) -> Result<Value, JsonRpcError> {
    let args_val = &params["arguments"];
    let args: JobStatusArgs =
        serde_json::from_value(args_val.clone()).map_err(|e| JsonRpcError {
            code: -32602,
            message: format!("Invalid arguments for get_job_status: {}", e),
            data: None,
        })?;

    let job_registry = vecdb_core::jobs::JobRegistry::new().ok();
    let local_jobs = job_registry
        .as_ref()
        .and_then(|r| r.load().ok())
        .unwrap_or_default();
    let remote_tasks = core.list_tasks().await.unwrap_or_default();

    if let Some(target_id) = args.id {
        let job = local_jobs.into_iter().find(|j| j.id == target_id);
        Ok(json!({
            "id": target_id,
            "local_job": job,
            "remote_tasks": remote_tasks.into_iter().filter(|t| t.id == target_id).collect::<Vec<_>>()
        }))
    } else {
        Ok(json!({
            "local_jobs": local_jobs,
            "remote_tasks": remote_tasks
        }))
    }
}