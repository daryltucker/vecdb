// JSON-RPC request/response types for the vecdb-server

use serde::{Deserialize, Serialize};
use serde_json::Value;

/// JSON-RPC 2.0 request structure
#[derive(Debug, Deserialize)]
#[allow(dead_code)]
pub struct JsonRpcRequest {
    pub jsonrpc: String,
    pub method: String,
    pub params: Option<Value>,
    pub id: Option<Value>,
}

/// JSON-RPC 2.0 response structure
#[derive(Debug, Serialize)]
pub struct JsonRpcResponse {
    pub jsonrpc: String,
    pub id: Value,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub result: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<JsonRpcError>,
}

/// JSON-RPC 2.0 error structure
#[derive(Debug, Serialize)]
pub struct JsonRpcError {
    pub code: i32,
    pub message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub data: Option<Value>,
}

/// Helper function to create a JSON-RPC error
pub fn json_rpc_error(code: i32, message: impl Into<String>) -> JsonRpcError {
    JsonRpcError {
        code,
        message: message.into(),
        data: None,
    }
}

/// Helper function to create a JSON-RPC error with data
pub fn json_rpc_error_with_data(
    code: i32,
    message: impl Into<String>,
    data: Value,
) -> JsonRpcError {
    JsonRpcError {
        code,
        message: message.into(),
        data: Some(data),
    }
}
