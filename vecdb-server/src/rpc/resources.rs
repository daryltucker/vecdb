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

        let collections = core.list_collections().await.map_err(|e| JsonRpcError {
            code: -32000,
            message: e.to_string(),
            data: None,
        })?;

        if let Some(collection) = collections.into_iter().find(|c| c.name == collection_name) {
            let profile = config
                .get_profile(Some(active_profile_name))
                .or_else(|_| config.get_profile(None))
                .unwrap_or_else(|_| config.get_profile(None).unwrap());

            let is_compatible = true; // TODO: Implement proper compatibility check

            let collection_info = json!({
                "name": collection.name,
                "vector_count": collection.vector_count,
                "vector_size": collection.vector_size,
                "is_active": profile.default_collection_name.as_deref() == Some(collection.name.as_str()),
                "is_compatible": is_compatible
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
