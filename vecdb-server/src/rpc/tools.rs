// Tool call handlers for vecdb-server JSON-RPC interface
// Handles all tools/call requests by dispatching to individual tool handlers

use crate::core_registry::CoreRegistry;
use crate::rpc::types::{JsonRpcError, JsonRpcRequest};
use serde_json::{json, Value};
use std::sync::Arc;
use vecdb_core::backend::Backend;
use vecdb_core::backends::qdrant::QdrantBackend;
use vecdb_core::config::Config;
use vecdb_core::tools::{
    EmbedArgs, IngestHistoryArgs, IngestPathArgs, JobStatusArgs, ProjectOverviewArgs, SearchArgs,
    VecqToolArgs,
};

/// Handle tools/call request by dispatching to individual tool handlers
pub async fn handle_tools_call(
    registry: &Arc<CoreRegistry>,
    config: &Arc<Config>,
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
        "search_vectors" => {
            handle_search_vectors(registry, config, params, active_profile_name).await
        }
        "delete_collection" => handle_delete_collection(registry, config, params).await,
        "list_collections" => handle_list_collections(registry, config, active_profile_name).await,
        "embed" => handle_embed(registry, config, params).await,
        "ingest_path" => {
            handle_ingest_path(
                registry,
                config,
                params,
                allow_local_fs,
                active_profile_name,
            )
            .await
        }
        "ingest_history" => {
            handle_ingest_history(
                registry,
                config,
                params,
                allow_local_fs,
                active_profile_name,
            )
            .await
        }
        "code_query" => handle_code_query(params, allow_local_fs).await,
        "project_overview" => handle_project_overview(params, allow_local_fs).await,
        "get_job_status" => handle_get_job_status(registry, config, params).await,
        _ => Err(JsonRpcError {
            code: -32601,
            message: format!("Tool not found: {}", name),
            data: None,
        }),
    }
}

/// Handle search_vectors tool.
///
/// Resolves the collection's configured profile and uses the matching Core
/// (embedder + backend). This is the fix for the single-boot-embedder bug:
/// each collection is searched with its own embedder, not the boot embedder.
async fn handle_search_vectors(
    registry: &Arc<CoreRegistry>,
    config: &Arc<Config>,
    params: &Value,
    active_profile_name: &str,
) -> Result<Value, JsonRpcError> {
    let args_val = &params["arguments"];
    let args: SearchArgs = serde_json::from_value(args_val.clone()).map_err(|e| JsonRpcError {
        code: -32602,
        message: format!("Invalid arguments for search: {}", e),
        data: None,
    })?;

    // Resolve the user's current profile context for finding the default collection name.
    // This uses active_profile_name as fallback (the user's current session context).
    let context_profile_name = args.profile.as_deref().unwrap_or(active_profile_name);
    let context_profile = config
        .get_profile(Some(context_profile_name))
        .map_err(|e| JsonRpcError {
            code: -32000,
            message: format!("Profile '{}' not found: {}", context_profile_name, e),
            data: None,
        })?;

    let collection = args
        .collection
        .as_deref()
        .or(context_profile.default_collection_name.as_deref())
        .ok_or_else(|| JsonRpcError {
            code: -32602,
            message: "collection is required: provide it in the request or configure a collection with this profile".into(),
            data: None,
        })?
        .to_string();

    // Resolve the Core for this collection.
    // CRITICAL: pass only the user's EXPLICIT profile (args.profile), not the fallback.
    // If no explicit profile is given, pass None so that config.resolve_profile() reads
    // the collection's own configured profile. This is the fix for the single-boot-embedder
    // bug: the collection config determines the embedder, not the server boot profile.
    let core = registry
        .get_for_collection(config, Some(&collection), args.profile.as_deref())
        .await
        .map_err(|e| JsonRpcError {
            code: -32000,
            message: format!(
                "Failed to resolve embedder for collection '{}': {}",
                collection, e
            ),
            data: None,
        })?;

    // Resolved once, shared with the CLI path. `min_score` rides along inside
    // the params so Qdrant applies it during traversal; the previous code
    // retained on the client after a fixed limit of 10, which turned a score
    // threshold into an unannounced reduction in result count.
    let params = args.to_search_params();

    let (results, applied_filters) = if args.use_smart() {
        core.search_smart(&collection, &args.query, params).await
    } else {
        core.search(&collection, &args.query, params)
            .await
            .map(|r| (r, serde_json::Map::new()))
    }
    .map_err(|e| JsonRpcError {
        code: -32000,
        message: e.to_string(),
        data: None,
    })?;

    // Report the retrieval parameters back alongside the hits. A model cannot
    // reason about whether to broaden its search unless it can see that the
    // result set was capped, thresholded, or scoped.
    let payload = json!({
        "collection": collection,
        "query": args.query,
        "limit": args.limit.unwrap_or(vecdb_core::config::DEFAULT_SEARCH_LIMIT),
        "min_score": args.min_score,
        "applied_filters": applied_filters,
        "result_count": results.len(),
        "results": results,
    });

    Ok(json!({
        "content": [
            {
                "type": "text",
                "text": serde_json::to_string(&payload).map_err(|e| JsonRpcError {
                    code: -32603,
                    message: format!("Serialization error: {}", e),
                    data: None,
                })?
            }
        ]
    }))
}

/// Handle delete_collection tool.
///
/// Resolves the collection's own Core, so a collection on a Qdrant other than the
/// boot instance is deleted from the instance it actually lives on. Falls back to
/// the boot Core only when that resolution fails (e.g. its embedder is
/// unreachable) — deletion needs a backend, not a working embedder.
async fn handle_delete_collection(
    registry: &Arc<CoreRegistry>,
    config: &Arc<Config>,
    params: &Value,
) -> Result<Value, JsonRpcError> {
    let args_val = &params["arguments"];
    let args: Value = serde_json::from_value(args_val.clone()).map_err(|e| JsonRpcError {
        code: -32602,
        message: format!("Invalid arguments for delete_collection: {}", e),
        data: None,
    })?;

    let collection = args
        .get("collection")
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

    // Attempt to use the collection-specific Core (correct backend for remote Qdrant).
    // Fall back to boot Core if the collection's profile can't be resolved (e.g., Ollama down).
    let core = match registry
        .get_for_collection(config, Some(collection), None)
        .await
    {
        Ok(core) => core,
        Err(_) => registry.boot_core(config).await.map_err(|e| JsonRpcError {
            code: -32000,
            message: format!(
                "Failed to resolve backend for collection '{}': {}",
                collection, e
            ),
            data: None,
        })?,
    };

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

/// Handle list_collections tool.
///
/// Queries ALL configured Qdrant backends (not just the boot core) by connecting
/// directly to each unique Qdrant URL with a lightweight client — no embedder
/// initialization required. This ensures remote collections (e.g. `notes-archive` on
/// a separate Qdrant instance) are visible to agents even when their profile's
/// embedder type (Ollama) differs from the boot profile.
async fn handle_list_collections(
    _registry: &Arc<CoreRegistry>,
    config: &Arc<Config>,
    active_profile_name: &str,
) -> Result<Value, JsonRpcError> {
    // Collect all unique (qdrant_url, qdrant_api_key) pairs from config
    #[derive(Hash, Eq, PartialEq, Clone)]
    struct QdrantEndpoint {
        url: String,
        api_key: Option<String>,
    }

    let mut endpoints: std::collections::HashSet<QdrantEndpoint> = std::collections::HashSet::new();

    for (name, prof) in &config.profiles {
        // A dangling store name is a load-time error, so this only skips for
        // configs that bypassed validation; the sweep is best-effort anyway.
        if let Ok((url, api_key)) = config.profile_endpoint(name, prof) {
            endpoints.insert(QdrantEndpoint { url, api_key });
        }
    }
    // Ensure default profile is included even if not explicitly listed
    if let Ok(default_prof) = config.get_profile(None) {
        if let Ok((url, api_key)) = config.profile_endpoint(&config.default_profile, default_prof) {
            endpoints.insert(QdrantEndpoint { url, api_key });
        }
    }

    let mut all_results: Vec<serde_json::Value> = Vec::new();

    for endpoint in endpoints {
        // Lightweight: connect directly to Qdrant without initializing any embedder.
        // This is the fix — list_collections previously went through get_core_for_profile
        // which creates a full Core including embedder initialization, causing failures
        // for profiles with embedder types (Ollama) that may not be reachable.
        let backend = match QdrantBackend::new(&endpoint.url, endpoint.api_key.clone()) {
            Ok(b) => b,
            Err(_) => continue,
        };

        let collection_names = match backend.list_collections().await {
            Ok(names) => names,
            Err(_) => continue,
        };

        for name in collection_names {
            let info: Option<vecdb_core::types::CollectionInfo> =
                backend.get_collection_info(&name).await.ok();
            let (count, dim) = info
                .map(|i| (i.vector_count, i.vector_size))
                .unwrap_or((None, None));

            all_results.push(json!({
                "name": name,
                "count": count,
                "dimension": dim,
                "is_compatible": false, // can't determine without an embedder
                "backend": endpoint.url,
                "is_local": endpoint.url.contains("localhost") ||
                           endpoint.url.contains("127.0.0.1") ||
                           endpoint.url.contains("0.0.0.0")
            }));
        }
    }

    // Get active profile info
    let profile = config
        .get_profile(Some(active_profile_name))
        .or_else(|_| config.get_profile(None))
        .map_err(|e| JsonRpcError {
            code: -32000,
            message: format!("Failed to resolve profile: {}", e),
            data: None,
        })?;

    let response_data = json!({
        "active_profile": active_profile_name,
        "default_collection": profile.default_collection_name,
        "collections": all_results
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

/// Handle embed tool.
/// Uses the boot Core's embedder (no collection context — caller gets the boot profile's model).
async fn handle_embed(
    registry: &Arc<CoreRegistry>,
    config: &Arc<Config>,
    params: &Value,
) -> Result<Value, JsonRpcError> {
    let args_val = &params["arguments"];
    let args: EmbedArgs = serde_json::from_value(args_val.clone()).map_err(|e| JsonRpcError {
        code: -32602,
        message: format!("Invalid arguments for embed: {}", e),
        data: None,
    })?;

    let core = registry.boot_core(config).await.map_err(|e| JsonRpcError {
        code: -32000,
        message: e.to_string(),
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

/// Handle ingest_path tool.
///
/// Resolves the collection's configured profile and uses the matching Core
/// (same fix as search_vectors — ingest must use the collection's own embedder).
async fn handle_ingest_path(
    registry: &Arc<CoreRegistry>,
    config: &Arc<Config>,
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

    // Resolve using the collection's own profile (not the boot default).
    // Pass args.profile as the explicit override (or None to let collection config win).
    let context_profile_name = args.profile.as_deref().unwrap_or(active_profile_name);
    let resolution = config
        .resolve(args.profile.as_deref(), args.collection.as_deref())
        .or_else(|_| config.resolve(Some(context_profile_name), args.collection.as_deref()))
        .map_err(|e| JsonRpcError {
            code: -32000,
            message: format!("Profile resolution failed: {}", e),
            data: None,
        })?;

    let collection = args
        .collection
        .as_deref()
        .or(resolution.collection.as_deref())
        .ok_or_else(|| JsonRpcError {
            code: -32602,
            message: "collection is required: provide it in the request or configure a collection with this profile".into(),
            data: None,
        })?
        .to_string();

    // Get the Core whose embedder matches the collection's configured profile.
    // Pass only the explicit profile (not the fallback) so collection config wins.
    let core = registry
        .get_for_collection(config, Some(&collection), args.profile.as_deref())
        .await
        .map_err(|e| JsonRpcError {
            code: -32000,
            message: format!(
                "Failed to resolve embedder for collection '{}': {}",
                collection, e
            ),
            data: None,
        })?;

    // Build the options here, as the CLI does, rather than through a convenience
    // wrapper that fills in chunking for us.
    //
    // `Core::ingest` did that, and what it filled in was wrong: it pinned
    // `pack_target_bytes: None`, so `chunking_identity()` recorded the 2048
    // default while the CLI recorded the collection's configured value. Since
    // `pack_target_bytes` is a GRANULARITY field, the chunking guard in
    // `ensure_write_target` then refused every MCP ingest into a collection the
    // CLI had created — and before that guard existed, the same gap silently
    // produced one collection holding two granularities.
    //
    // It also hardcoded `strategy` and `tokenizer` past the config. There is one
    // ingestion entry point now, `ingest_with_options`, and the caller states
    // every parameter it means.
    let opts = vecdb_core::ingestion::IngestionOptions {
        path: args.path.clone(),
        collection: collection.clone(),
        target_chunk_size: resolution.target_chunk_size.value,
        // Clamped to what the model can embed, exactly as the CLI does: the
        // oversize policy splits DOWN TO this ceiling, so a ceiling above the
        // model's capacity makes the split produce parts that still do not fit.
        max_chunk_bytes: Some(resolution.effective_max_chunk_bytes()),
        chunk_overlap: resolution.chunk_overlap.value,
        // The knob that actually governs granularity for parsed content.
        pack_target_bytes: Some(resolution.pack_target_bytes.value),
        on_oversize: config.resolve_oversize_policy().value,
        strategy: config.ingestion.default_strategy.clone(),
        tokenizer: config.ingestion.tokenizer.clone(),
        respect_gitignore: config.ingestion.respect_gitignore,
        ignore_vectorignore: args.ignore_vectorignore,
        path_rules: config.ingestion.path_rules.clone(),
        max_concurrent_requests: args
            .concurrency
            .unwrap_or(config.ingestion.max_concurrent_requests),
        gpu_batch_size: args.gpu_concurrency.unwrap_or(2),
        quantization: resolution.quantization.clone(),
        // MCP has no `.vecdbrc` routing, no allowlist and no dry run.
        vecdbrc_routes: None,
        vecdbrc_root: None,
        route_chunking: Default::default(),
        only_collection: None,
        route_default_collection: None,
        file_allowlist: None,
        project_root: None,
        git_ref: None,
        extensions: None,
        excludes: None,
        dry_run: false,
        metadata: None,
        allow_quantization_delta: false,
    };

    core.ingest_with_options(opts, None)
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

/// Handle ingest_history tool.
async fn handle_ingest_history(
    registry: &Arc<CoreRegistry>,
    config: &Arc<Config>,
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
    let is_remote = args.repo_path.starts_with("http") || args.repo_path.starts_with("git@");
    if !is_remote && !allow_local_fs {
        return Err(JsonRpcError {
            code: -32000,
            message: "Security Error: Local filesystem access is disabled. Cannot ingest local repo history.".into(),
            data: None,
        });
    }

    let context_profile_name = args.profile.as_deref().unwrap_or(active_profile_name);
    let resolution = config
        .resolve(args.profile.as_deref(), args.collection.as_deref())
        .or_else(|_| config.resolve(Some(context_profile_name), args.collection.as_deref()))
        .map_err(|e| JsonRpcError {
            code: -32000,
            message: format!("Profile resolution failed: {}", e),
            data: None,
        })?;

    let collection = args
        .collection
        .as_deref()
        .or(resolution.collection.as_deref())
        .ok_or_else(|| JsonRpcError {
            code: -32602,
            message: "collection is required: provide it in the request or configure a collection with this profile".into(),
            data: None,
        })?
        .to_string();

    // Get the Core whose embedder matches the collection's configured profile.
    // Pass only the explicit profile (not the fallback) so collection config wins.
    let core = registry
        .get_for_collection(config, Some(&collection), args.profile.as_deref())
        .await
        .map_err(|e| JsonRpcError {
            code: -32000,
            message: format!(
                "Failed to resolve embedder for collection '{}': {}",
                collection, e
            ),
            data: None,
        })?;

    core.ingest_history(
        &args.repo_path,
        &args.git_ref,
        &collection,
        // The collection's own resolved chunking, not a literal. Same defect as
        // the CLI carried: `512` here wrote the collection at a granularity
        // nothing in the config mentioned, while a plain `ingest` used the
        // configured one. `ensure_write_target`'s chunking guard refuses that.
        vecdb_core::ingestion::options::ChunkSpec {
            target_chunk_size: resolution.target_chunk_size.value,
            chunk_overlap: resolution.chunk_overlap.value,
            // Clamped to the model's capacity, exactly as `ingest` does.
            max_chunk_bytes: Some(resolution.effective_max_chunk_bytes()),
            pack_target_bytes: Some(resolution.pack_target_bytes.value),
        },
        resolution.quantization.clone(),
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
async fn handle_code_query(params: &Value, allow_local_fs: bool) -> Result<Value, JsonRpcError> {
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
            message:
                "Security Error: Local filesystem access is disabled. Cannot query local files."
                    .into(),
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

        let parsed = vecq::parse_file(&content, file_type)
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
                    // `e` already names itself and the offending query;
                    // prefixing would render "Query error: Query error in ...".
                    message: e.to_string(),
                    data: None,
                });
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

/// Handle project_overview tool.
///
/// Walks a directory (respecting .vectorignore), parses each supported source file
/// with vecq, and returns a JGF v2 architectural graph and Mermaid diagram via the
/// graph_src / graph_to_architecture jq normalizers already built into vecq's query engine.
async fn handle_project_overview(
    params: &Value,
    allow_local_fs: bool,
) -> Result<Value, JsonRpcError> {
    if !allow_local_fs {
        return Err(JsonRpcError {
            code: -32000,
            message: "Security Error: Local filesystem access is disabled. Cannot analyze local projects.".into(),
            data: None,
        });
    }

    let args_val = &params["arguments"];
    let args: ProjectOverviewArgs =
        serde_json::from_value(args_val.clone()).map_err(|e| JsonRpcError {
            code: -32602,
            message: format!("Invalid arguments for project_overview: {}", e),
            data: None,
        })?;

    let path = std::path::Path::new(&args.path);
    if !path.exists() {
        return Err(JsonRpcError {
            code: -32000,
            message: format!("Path not found: {}", args.path),
            data: None,
        });
    }
    if !path.is_dir() {
        return Err(JsonRpcError {
            code: -32000,
            message: format!("Path is not a directory: {}", args.path),
            data: None,
        });
    }

    let vecq_args = vecq::ProjectOverviewArgs {
        path: path.to_path_buf(),
        max_depth: args.max_depth,
        ignore_patterns: args.ignore_patterns,
        respect_gitignore: args.respect_gitignore.unwrap_or(false),
        ignore_vectorignore: args.ignore_vectorignore.unwrap_or(false),
        skip_hidden: args.skip_hidden.unwrap_or(true),
    };

    let overview = vecq::project_overview(vecq_args)
        .await
        .map_err(|e| JsonRpcError {
            code: -32000,
            message: format!("project_overview failed: {}", e),
            data: None,
        })?;

    let summary = format!(
        "Project: {}\nFiles analyzed: {}\nFiles skipped: {}\n\n{}",
        overview.project_root, overview.files_analyzed, overview.files_skipped, overview.mermaid
    );

    Ok(json!({
        "content": [
            {
                "type": "text",
                "text": summary
            }
        ],
        "_meta": {
            "project_root": overview.project_root,
            "files_analyzed": overview.files_analyzed,
            "files_skipped": overview.files_skipped,
            "graph": overview.graph
        }
    }))
}

/// Handle get_job_status tool.
/// Uses the boot Core.
async fn handle_get_job_status(
    registry: &Arc<CoreRegistry>,
    config: &Arc<Config>,
    params: &Value,
) -> Result<Value, JsonRpcError> {
    let args_val = &params["arguments"];
    let args: JobStatusArgs =
        serde_json::from_value(args_val.clone()).map_err(|e| JsonRpcError {
            code: -32602,
            message: format!("Invalid arguments for get_job_status: {}", e),
            data: None,
        })?;

    let core = registry.boot_core(config).await.map_err(|e| JsonRpcError {
        code: -32000,
        message: e.to_string(),
        data: None,
    })?;

    let job_registry = vecdb_core::jobs::JobRegistry::new().ok();
    let local_jobs = job_registry
        .as_ref()
        .and_then(|r| r.load().ok())
        .unwrap_or_default();
    // Distinguish "none in flight" from "the backend cannot tell us".
    let (remote_tasks, remote_tasks_error) = match core.list_tasks().await {
        Ok(t) => (t, None),
        Err(e) => (Vec::new(), Some(e.to_string())),
    };

    if let Some(target_id) = args.id {
        let job = local_jobs.into_iter().find(|j| j.id == target_id);
        Ok(json!({
            "id": target_id,
            "local_job": job,
            "remote_tasks": remote_tasks.into_iter().filter(|t| t.id == target_id).collect::<Vec<_>>(),
            "remote_tasks_error": remote_tasks_error
        }))
    } else {
        Ok(json!({
            "local_jobs": local_jobs,
            "remote_tasks": remote_tasks,
            "remote_tasks_error": remote_tasks_error
        }))
    }
}
