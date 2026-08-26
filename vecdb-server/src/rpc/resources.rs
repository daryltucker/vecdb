// Resource handlers for vecdb-server JSON-RPC interface
// Handles resources/list and resources/read requests

use crate::core_registry::CoreRegistry;
use crate::rpc::types::{JsonRpcError, JsonRpcRequest};
use serde_json::json;
use std::sync::Arc;
use vecdb_core::config::Config;

/// Handle resources/list request
pub async fn handle_resources_list(
    registry: &Arc<CoreRegistry>,
    config: &Arc<Config>,
) -> Result<serde_json::Value, JsonRpcError> {
    let core = registry.boot_core(config).await.map_err(|e| JsonRpcError {
        code: -32000,
        message: e.to_string(),
        data: None,
    })?;

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

/// Handle resources/read request
pub async fn handle_resources_read(
    registry: &Arc<CoreRegistry>,
    config: &Arc<Config>,
    req: &JsonRpcRequest,
    active_profile_name: &str,
) -> Result<serde_json::Value, JsonRpcError> {
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
                    "text": include_str!("../../../vecdb-cli/src/docs/man_agent.md")
                }
            ]
        }));
    }

    if uri == "vecdb://registry" || uri == "vecdb://services" {
        let core = registry.boot_core(config).await.map_err(|e| JsonRpcError {
            code: -32000,
            message: e.to_string(),
            data: None,
        })?;

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

        let registry_data = json!({
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
                    "text": serde_json::to_string_pretty(&registry_data).map_err(|e| JsonRpcError {
                        code: -32603,
                        message: format!("Serialization error: {}", e),
                        data: None,
                    })?

                }
            ]
        }));
    }

    // Handle collection-specific URIs
    if let Some(collection_name) = uri.strip_prefix("vecdb://collections/") {
        let core = registry.boot_core(config).await.map_err(|e| JsonRpcError {
            code: -32000,
            message: e.to_string(),
            data: None,
        })?;

        // Read genesis alongside the collection: compatibility is a claim about
        // the embedding space, which only the genesis record carries.
        let collections = core
            .list_collections_with_genesis()
            .await
            .map_err(|e| JsonRpcError {
                code: -32000,
                message: e.to_string(),
                data: None,
            })?;

        if let Some((collection, genesis)) = collections
            .into_iter()
            .find(|(c, _)| c.name == collection_name)
        {
            let resolution = config
                .resolve(Some(active_profile_name), Some(collection_name))
                .or_else(|_| config.resolve(None, Some(collection_name)))
                .map_err(|e| JsonRpcError {
                    code: -32000,
                    message: format!("Profile resolution failed: {e}"),
                    data: None,
                })?;

            // `is_compatible` used to be hardcoded `true`.
            //
            // This resource is what an agent reads to decide whether it may use
            // a collection, so a hardcoded `true` is not a missing feature — it
            // is an answer that is wrong exactly when it matters: a foreign
            // collection, or one written by a different model, reported as
            // usable right up until the write is refused.
            let identity = core.embedder().identity().await.ok();
            let report = identity.as_ref().map(|id| {
                vecdb_core::types::compare_spaces(
                    &genesis.model,
                    genesis.dimension,
                    id,
                    resolution.embedder.dimension.map(|d| d as u64),
                )
            });

            let is_vecdb = genesis.is_vecdb();
            let is_compatible =
                is_vecdb && report.as_ref().map(|r| r.permits_read()).unwrap_or(false);

            let collection_info = json!({
                "name": collection.name,
                "vector_count": collection.vector_count,
                "vector_size": collection.vector_size,
                "is_active": resolution.collection.as_deref() == Some(collection.name.as_str()),
                "is_vecdb": is_vecdb,
                "is_compatible": is_compatible,
                "model": if is_vecdb { Some(genesis.model.describe()) } else { None },
                "reason": report.as_ref().map(|r| r.reason.clone()),
            });

            return Ok(json!({
                "contents": [
                    {
                        "uri": uri,
                        "mimeType": "application/json",
                        "text": serde_json::to_string_pretty(&collection_info).map_err(|e| JsonRpcError {
                            code: -32603,
                            message: format!("Serialization error: {}", e),
                            data: None,
                        })?
                    }
                ]
            }));
        } else {
            return Err(JsonRpcError {
                code: -32000,
                message: format!("Collection '{}' not found", collection_name),
                data: None,
            });
        }
    }

    Err(JsonRpcError {
        code: -32601,
        message: format!("Resource '{}' not found", uri),
        data: None,
    })
}
