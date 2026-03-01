// RPC module for vecdb-server
// Provides JSON-RPC interface for the vecdb MCP server

pub mod types;
pub mod dispatcher;
pub mod tools;
pub mod resources;

// Re-export the main entry point
pub use dispatcher::handle_request;