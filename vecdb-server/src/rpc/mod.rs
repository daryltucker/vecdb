// RPC module for vecdb-server
// Provides JSON-RPC interface for the vecdb MCP server

pub mod dispatcher;
pub mod resources;
pub mod tools;
pub mod types;

// Re-export the main entry point
pub use dispatcher::handle_request;
