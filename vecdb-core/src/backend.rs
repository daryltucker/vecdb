/*
 * PURPOSE:
 *   Defines the `Backend` trait, which abstracts the underlying vector database
 *   implementation. This allows vecdb-mcp to support multiple storage engines
 *   (Qdrant, Milvus, Postgres/pgvector) without changing core logic.
 *
 * REQUIREMENTS:
 *   User-specified:
 *   - Must be backend-agnostic (R-006, R-007)
 *   - Must support "Pluggable Storage Backends" (Architecture)
 *
 *   Implementation-discovered:
 *   - Needs `async-trait` as Rust traits don't support async methods natively yet
 *   - Needs `Send + Sync` for thread safety in async runtime
 *   - Needs standardized error handling (Anyhow/Result)
 *
 * IMPLEMENTATION RULES:
 *   1. Use `async_trait` macro
 *      Rationale: Essential for I/O bound database operations
 *
 *   2. Return `anyhow::Result`
 *      Rationale: Backends may fail for diverse reasons (network, disk, auth);
 *      caller just needs to know it failed and why.
 *
 *   3. Filter is optional `serde_json::Value`
 *      Rationale: Different DBs have different filter syntaxes. We pass raw JSON
 *      and let the implementation parsers translate it (Law #1).
 *
 * USAGE:
 *   struct MyBackend;
 *   #[async_trait]
 *   impl Backend for MyBackend { ... }
 *
 * SELF-HEALING INSTRUCTIONS:
 *   - If new DB operations needed: Add default implementation returning "Not Implemented" error
 *     to avoid breaking existing backends immediately.
 *   - If trait becomes too large: Split into `BackendRead` and `BackendWrite`
 *
 * RELATED FILES:
 *   - src/types.rs - Defines data structures exchanged via this trait
 *   - src/lib.rs - Exports this trait
 *
 * MAINTENANCE:
 *   Update when:
 *   - New core database features required (e.g., hybrid search, sparse vectors)
 */

use crate::types::{Chunk, SearchResult};
use anyhow::Result;
use async_trait::async_trait;

#[async_trait]
pub trait Backend: Send + Sync {
    /// Initialize or get a connection to the backend.
    /// This is often done at struct creation, but a health check method is useful.
    async fn health_check(&self) -> Result<()>;

    /// Create a new collection (index) with the specified vector dimension.
    async fn create_collection(&self, name: &str, vector_size: u64, quantization: Option<crate::config::QuantizationType>) -> Result<()>;

    /// Update collection configuration (specifically quantization)
    async fn update_collection_quantization(&self, name: &str, quantization: crate::config::QuantizationType) -> Result<()>;

    /// Check if a collection exists.
    async fn collection_exists(&self, name: &str) -> Result<bool>;

    /// Delete a collection and all its data.
    async fn delete_collection(&self, name: &str) -> Result<()>;

    /// Upsert (Update or Insert) chunks into the collection.
    /// Operations should be idempotent.
    async fn upsert(&self, collection: &str, chunks: Vec<Chunk>) -> Result<()>;

    /// Perform a semantic search.
    /// 
    /// # Arguments
    /// * `collection` - Name of the collection to search
    /// * `vector` - Query vector (embedding)
    /// * `limit` - Maximum number of results to return
    /// * `filter` - Optional JSON filter query (backend-specific syntax)
    async fn search(
        &self,
        collection: &str,
        vector: &[f32],
        limit: u64,
        filter: Option<serde_json::Value>,
    ) -> Result<Vec<SearchResult>>;
    
    /// Check if a set of points (by ID) exist in the collection.
    /// Returns a list of IDs that DO exist.
    async fn points_exists(&self, collection: &str, ids: Vec<String>) -> Result<Vec<String>>;

    /// List all available collections.
    async fn list_collections(&self) -> Result<Vec<String>>;

    /// Get detailed information about a collection.
    async fn get_collection_info(&self, name: &str) -> Result<crate::types::CollectionInfo>;

    /// List unique values for a specific metadata key in a collection.
    /// Used for dynamic discovery of versions, themes, etc.
    async fn list_metadata_values(&self, collection: &str, key: &str) -> Result<Vec<String>>;
}
