use schemars::schema_for;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::sync::Arc;
use vecdb_core::config::Config;
use vecdb_core::tools::{
    EmbedArgs, IngestHistoryArgs, IngestPathArgs, JobStatusArgs, SearchArgs, VecqToolArgs,
};
use vecdb_core::Core;
use vecq; // Direct access to vecq logic

// JSON-RPC Types
#[derive(Debug, Deserialize)]
#[allow(dead_code)]
pub struct JsonRpcRequest {
    pub jsonrpc: String,
    pub method: String,
    pub params: Option<Value>,
    pub id: Option<Value>,
}

#[derive(Debug, Serialize)]
pub struct JsonRpcResponse {
    pub jsonrpc: String,
    pub id: Value,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub result: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<JsonRpcError>,
}

#[derive(Debug, Serialize)]
pub struct JsonRpcError {
    pub code: i32,
    pub message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub data: Option<Value>,
}

pub async fn handle_request(
    core: &Arc<Core>,
    config: &Config,
    req: &JsonRpcRequest,
    allow_local_fs: bool,
    active_profile_name: &str,
) -> Result<Value, JsonRpcError> {
    match req.method.as_str() {
        "initialize" => Ok(json!({
            "protocolVersion": "2024-11-05",
            "serverInfo": {
                "name": "vecdb-mcp",
                "version": "0.1.0"
            },
            "capabilities": {
                "tools": {},
                "resources": {}
            }
        })),
        "notifications/initialized" => Ok(Value::Null),
        "tools/list" => {
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
        "resources/list" => {
            let collections = core.list_collections().await.map_err(|e| JsonRpcError {
                code: -32000,
                message: e.to_string(),
                data: None,
            })?;

            let resources = vec![
                json!({
                    "uri": "vecdb://registry",
                    "name": "Server Registry",
                    "mimeType": "application/json",
                    "description": "Consolidated overview of active profile, collections, and system status"
                }),
                json!({
                    "uri": "vecdb://services",
                    "name": "Service Discovery",
                    "mimeType": "application/json",
                    "description": "Compatibility alias for registry summary"
                }),
                json!({
                    "uri": "vecdb://manual",
                    "name": "User Manual",
                    "mimeType": "text/markdown",
                    "description": "Agent Interface Specification and Workflow Guide"
                })
            ].into_iter().chain(collections.into_iter().map(|c| {
                json!({
                    "uri": format!("vecdb://collections/{}", c.name),
                    "name": format!("Collection: {}", c.name),
                    "mimeType": "application/json",
                    "description": format!("Vector Collection: {} vectors, {}d", c.vector_count.unwrap_or(0), c.vector_size.unwrap_or(0))
                })
            })).collect::<Vec<_>>();

            Ok(json!({
                "resources": resources
            }))
        }
        "resources/read" => {
            let params = req.params.as_ref().ok_or(JsonRpcError {
                code: -32602,
                message: "Missing params".into(),
                data: None,
            })?;

            let uri = params["uri"].as_str().ok_or(JsonRpcError {
                code: -32602,
                message: "Missing uri".into(),
                data: None,
            })?;

            if uri == "vecdb://manual" {
                return Ok(json!({
                    "contents": [
                        {
                            "uri": "vecdb://manual",
                            "mimeType": "text/markdown",
                            "text": include_str!("../../vecdb-cli/src/docs/man_agent.md")
                        }
                    ]
                }));
            }

            if uri == "vecdb://registry" || uri == "vecdb://services" {
                let collections = core.list_collections().await.map_err(|e| JsonRpcError {
                    code: -32000,
                    message: e.to_string(),
                    data: None,
                })?;

                let profile = config
                    .get_profile(Some(active_profile_name))
                    .or_else(|_| config.get_profile(None))
                    .map_err(|e| JsonRpcError {
                        code: -32000,
                        message: format!("Failed to resolve profile: {}", e),
                        data: None,
                    })?;

                let registry = json!({
                    "status": "online",
                    "active_profile": active_profile_name,
                    "default_collection": profile.default_collection_name,
                    "collections_count": collections.len(),
                    "collections": collections.iter().map(|c| &c.name).collect::<Vec<_>>(),
                    "version": env!("CARGO_PKG_VERSION")
                });

                return Ok(json!({
                    "contents": [
                        {
                            "uri": uri,
                            "mimeType": "application/json",
                            "text": serde_json::to_string_pretty(&registry).map_err(|e| JsonRpcError {
                                code: -32603,
                                message: format!("Serialization error: {}", e),
                                data: None,
                            })?

                        }
                    ]
                }));
            }

            if !uri.starts_with("vecdb://collections/") {
                return Err(JsonRpcError {
                    code: -32602,
                    message: "Invalid URI scheme. Expected vecdb://collections/{name} or vecdb://registry".into(),
                    data: None,
                });
            }

            let collection_name = &uri[20..];

            // Get collection stats
            let collections = core.list_collections().await.map_err(|e| JsonRpcError {
                code: -32000,
                message: e.to_string(),
                data: None,
            })?;

            let collection = collections
                .iter()
                .find(|c| c.name == collection_name)
                .ok_or(JsonRpcError {
                    code: -404, // Not found
                    message: format!("Collection '{}' not found", collection_name),
                    data: None,
                })?;

            Ok(json!({
                "contents": [
                    {
                        "uri": uri,
                        "mimeType": "application/json",
                        "text": serde_json::to_string_pretty(&collection).map_err(|e| JsonRpcError {
                            code: -32603,
                            message: format!("Serialization error: {}", e),
                            data: None,
                        })?

                    }
                ]
            }))
        }
        "tools/call" => {
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

            if name == "search_vectors" {
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
                    .unwrap_or(&profile.default_collection_name);

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
            } else if name == "delete_collection" {
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
            } else if name == "list_collections" {
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
                            "is_active": c.name == profile.default_collection_name,
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
            } else if name == "embed" {
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
            } else if name == "ingest_path" {
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
                let max_chunk_size = config.resolve_max_chunk_size(args.collection.as_deref());
                let chunk_overlap = config.resolve_chunk_overlap(args.collection.as_deref());

                // Resolve final collection: explicit > profile default
                let collection = args
                    .collection
                    .as_deref()
                    .unwrap_or(&profile.default_collection_name);

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
            } else if name == "ingest_history" {
                let args_val = &params["arguments"];
                let args: IngestHistoryArgs =
                    serde_json::from_value(args_val.clone()).map_err(|e| JsonRpcError {
                        code: -32602,
                        message: format!("Invalid arguments for {}: {}", name, e),
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
                    .unwrap_or(&profile.default_collection_name);

                core.ingest_history(
                    &args.repo_path,
                    &args.git_ref,
                    collection,
                    512,
                    profile.quantization.clone(),
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
            } else if name == "code_query" {
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

                /*
                let result = core.code_query(
                    &args.path,
                    &args.query,
                    None,
                    args.source,
                    args.git_ref,
                    args.repo_path
                ).await.map_err(|e| JsonRpcError {
                   code: -32000,
                   message: e.to_string(),
                   data: None,
                })?;
                */

                Ok(json!({
                   "content": [
                       {
                           "type": "text",
                           "text": result
                       }
                   ]
                }))
            } else if name == "get_job_status" {
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
            } else {
                Err(JsonRpcError {
                    code: -32601,
                    message: format!("Tool not found: {}", name),
                    data: None,
                })
            }
        }
        _ => Err(JsonRpcError {
            code: -32601,
            message: "Method not found".into(),
            data: None,
        }),
    }
}
