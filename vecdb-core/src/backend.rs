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

/// Parameters for a single semantic search.
///
/// This is a struct rather than a positional argument list because the set of
/// retrieval knobs grows (offset, consistency, sparse vectors) and every
/// addition would otherwise churn every `Backend` implementor including the
/// test mocks. `Default` gives `limit = 10`, no filter, no threshold.
#[derive(Debug, Clone)]
pub struct SearchParams {
    /// Maximum number of results to return.
    pub limit: u64,

    /// Optional JSON filter query (backend-specific syntax).
    pub filter: Option<serde_json::Value>,

    /// Minimum similarity score. Applied by the backend *before* `limit`
    /// truncation, so a threshold never silently shortens a full result page.
    pub score_threshold: Option<f32>,
}

impl Default for SearchParams {
    fn default() -> Self {
        Self {
            limit: crate::config::DEFAULT_SEARCH_LIMIT,
            filter: None,
            score_threshold: None,
        }
    }
}

impl SearchParams {
    /// Construct with an explicit limit, leaving filter and threshold unset.
    pub fn new(limit: u64) -> Self {
        Self {
            limit,
            ..Default::default()
        }
    }

    pub fn with_filter(mut self, filter: Option<serde_json::Value>) -> Self {
        self.filter = filter;
        self
    }

    pub fn with_score_threshold(mut self, threshold: Option<f32>) -> Self {
        self.score_threshold = threshold;
        self
    }
}

#[async_trait]
pub trait Backend: Send + Sync {
    /// Initialize or get a connection to the backend.
    /// This is often done at struct creation, but a health check method is useful.
    async fn health_check(&self) -> Result<()>;

    /// Create a new collection (index) with the specified vector dimension.
    async fn create_collection(
        &self,
        name: &str,
        vector_size: u64,
        quantization: Option<crate::config::QuantizationType>,
    ) -> Result<()>;

    /// Update collection configuration (specifically quantization)
    async fn update_collection_quantization(
        &self,
        name: &str,
        quantization: crate::config::QuantizationType,
    ) -> Result<()>;

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
    /// * `params` - Limit, optional filter, optional score threshold
    ///
    /// Implementations MUST return at most `params.limit` results, MUST apply
    /// `score_threshold` server-side before truncating to `limit`, and MUST NOT
    /// let internal bookkeeping points (e.g. the genesis point) consume a slot
    /// of the caller's `limit`.
    async fn search(
        &self,
        collection: &str,
        vector: &[f32],
        params: SearchParams,
    ) -> Result<Vec<SearchResult>>;

    /// Check if a set of points (by ID) exist in the collection.
    /// Returns a list of IDs that DO exist.
    async fn points_exists(&self, collection: &str, ids: Vec<String>) -> Result<Vec<String>>;

    /// Drop every point belonging to `document_id` except the ones in `keep`.
    ///
    /// Re-ingesting a changed file writes new chunks under new IDs — the ID is a
    /// UUIDv5 over the content — but nothing ever removed the points the old
    /// content occupied. Editing a function left both versions in the index, and
    /// deleting one left it there permanently; searches returned code that no
    /// longer exists, with no way to tell which hit was current.
    ///
    /// This was invisible until Python and Go stopped emitting a constant
    /// signature label as their content (see `tier1_language_fidelity`): while
    /// the content never changed, neither did the ID, so nothing accumulated.
    /// Fixing the parsers exposed it immediately.
    ///
    /// Called with the *complete* chunk set for one document, and always before
    /// that set is upserted, so a point that is about to be rewritten is never
    /// deleted. Returns how many points were removed.
    async fn delete_stale_points(
        &self,
        collection: &str,
        document_id: &str,
        keep: &[String],
    ) -> Result<usize>;

    /// List all available collections.
    async fn list_collections(&self) -> Result<Vec<String>>;

    /// Get detailed information about a collection.
    async fn get_collection_info(&self, name: &str) -> Result<crate::types::CollectionInfo>;

    /// List unique values for a specific metadata key in a collection.
    /// Used for dynamic discovery of versions, themes, etc.
    async fn list_metadata_values(&self, collection: &str, key: &str) -> Result<Vec<String>>;

    /// Set the unique Collection ID (UUID) for a collection.
    /// Should be idempotent.
    async fn set_collection_id(&self, collection: &str, id: &str) -> Result<()>;

    /// Get the unique Collection ID (UUID) for a collection if it exists.
    async fn get_collection_id(&self, collection: &str) -> Result<Option<String>>;

    /// Write the full embedding-space contract into the collection's genesis
    /// point. Called once at creation, before any content is upserted.
    async fn write_genesis(
        &self,
        collection: &str,
        meta: &crate::types::GenesisMetadata,
    ) -> Result<()>;

    /// Read the embedding-space contract back.
    ///
    /// Returns `present: false` when the collection has no genesis point. That
    /// is not an error and not a legacy case — the shared Qdrant holds
    /// collections written by other tools entirely. It means "not ours".
    async fn read_genesis(&self, collection: &str) -> Result<crate::types::CollectionGenesis>;

    /// List background tasks (optimization, etc.) from the backend.
    async fn list_tasks(&self) -> Result<Vec<crate::types::TaskInfo>>;
}
