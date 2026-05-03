/*
 * PURPOSE:
 *   Root library definition for vecdb-core.
 *   Exposes the core abstractions (Types, Backend) and logic
 *   to the server and CLI consumers.
 *
 * REQUIREMENTS:
 *   User-specified:
 *   - Shared functional core logic (Architecture)
 *
 *   Implementation-discovered:
 *   - Must expose modules publically
 *
 * IMPLEMENTATION RULES:
 *   1. Re-export key types for ergonomics (facade pattern optional but usually good)
 *      Rationale: `use vecdb_core::Document` is cleaner than `vecdb_core::types::Document`
 *
 * USAGE:
 *   - Unified interface for all backends
 */

pub mod backend;
pub mod backends;
pub mod chunking;
pub mod config;
pub mod embedder;
pub mod embedders;
pub mod git;
pub mod history;
pub mod ingestion;
pub mod jobs;
pub mod parsers;
pub mod resource;
pub mod router;
pub mod snapshot;
pub mod state;
pub mod tools;
pub mod types;
pub mod vecdbrc;

// Re-export output from vecdb-common for backwards compatibility
pub use vecdb_common::output;

use anyhow::Result;
use backend::Backend;
use backends::qdrant::QdrantBackend;
use embedder::Embedder;
use embedders::{ArbitratedEmbedder, OllamaEmbedder};
use ingestion::IngestionOptions;
use parsers::ParserFactory;
use resource::ResourceArbiter;
use router::DynamicRouter;
use std::sync::Arc;
use std::sync::OnceLock;

/// Process-singleton arbiter shared by every Core constructed in this process.
///
/// Why a singleton: two Core instances in the same MCP server process that both
/// use the local GPU must serialise via the *same* semaphore, otherwise we
/// regress to the pre-arbiter behaviour where two paths fight CUDA over OOM.
/// An arbiter local to each Core would defeat the purpose.
fn process_arbiter() -> Arc<ResourceArbiter> {
    static ARBITER: OnceLock<Arc<ResourceArbiter>> = OnceLock::new();
    ARBITER.get_or_init(|| Arc::new(ResourceArbiter::new())).clone()
}
use types::SearchResult;
use vecdb_common::FileTypeDetector;
// use serde_json::json;

/// The main entry point for the Vector Database logic.
/// Wraps a concrete Backend implementation and Embedder.
pub struct Core {
    backend: Arc<dyn Backend + Send + Sync>,
    embedder: Arc<dyn Embedder + Send + Sync>,
    file_detector: Arc<dyn FileTypeDetector>,
    parser_factory: Arc<dyn ParserFactory>,
    smart_routing_keys: Vec<String>,
    path_rules: Vec<crate::config::PathRule>,
    max_concurrent_requests: usize,
    gpu_batch_size: usize,
}

impl Core {
    /// Create a new Core instance with Qdrant backend and configurable embedder.
    ///
    /// # Arguments
    /// * `embedder_type` - "local" for fastembed (no external deps) or "ollama" for Ollama API
    #[allow(clippy::too_many_arguments)]
    pub async fn new(
        qdrant_url: &str,
        ollama_url: &str,
        model: &str,
        accept_invalid_certs: bool,
        embedder_type: &str,
        fastembed_cache_path: Option<std::path::PathBuf>,
        use_gpu: bool,
        // API Keys
        qdrant_api_key: Option<String>,
        ollama_api_key: Option<String>,
        // Routing
        smart_routing_keys: Vec<String>,
        path_rules: Vec<crate::config::PathRule>,
        max_concurrent_requests: usize,
        gpu_batch_size: usize,
        num_ctx: Option<usize>,
        // Dependency Injection
        file_detector: Arc<dyn FileTypeDetector>,
        parser_factory: Arc<dyn ParserFactory>,
    ) -> Result<Self> {
        let backend = QdrantBackend::new(qdrant_url, qdrant_api_key)?;

        let embedder: Arc<dyn Embedder + Send + Sync> = match embedder_type {
            #[cfg(feature = "local-embed")]
            "local" | "fastembed" => {
                if output::OUTPUT.is_interactive {
                    eprintln!("Using local embedder (fastembed: {}) [GPU: {}]", model, use_gpu);
                }
                Arc::new(embedders::LocalEmbedder::new(
                    model,
                    fastembed_cache_path,
                    use_gpu,
                )?)
            }
            "ollama" => {
                if output::OUTPUT.is_interactive {
                    eprintln!(
                        "Using Ollama embedder at {} with model {}",
                        ollama_url, model
                    );
                }
                Arc::new(OllamaEmbedder::new(
                    ollama_url.to_string(),
                    model.to_string(),
                    accept_invalid_certs,
                    ollama_api_key,
                    num_ctx,
                ))
            }
            #[cfg(not(feature = "local-embed"))]
            "local" => {
                anyhow::bail!("Local embedder not available. Compile with 'local-embed' feature or use embedder_type = 'ollama'")
            }
            "mock" => {
                if output::OUTPUT.is_interactive {
                    eprintln!("Using Mock Embedder (Deterministic)");
                }
                Arc::new(embedders::MockEmbedder::new(384)) // Default dim
            }
            _ => {
                if output::OUTPUT.is_interactive {
                    eprintln!(
                        "Unknown embedder type '{}', falling back to Ollama",
                        embedder_type
                    );
                }
                Arc::new(OllamaEmbedder::new(
                    ollama_url.to_string(),
                    model.to_string(),
                    accept_invalid_certs,
                    ollama_api_key,
                    num_ctx,
                ))
            }
        };

        // Wrap in ArbitratedEmbedder so embed/embed_batch/dimension calls go
        // through the process-wide ResourceArbiter. Different embedders with
        // different required_resources() will not block each other; same-resource
        // calls serialise correctly (see resource.rs).
        let embedder: Arc<dyn Embedder + Send + Sync> = Arc::new(
            ArbitratedEmbedder::new(embedder, process_arbiter()),
        );

        // Upfront Connection Validation: If the user explicitly asks for Ollama or Local,
        // we strictly prove it's alive AND that the specific model can be loaded into memory.
        // This prevents the application from deadlocking or silently failing later.
        // OPT-OUT: VECDB_SKIP_PROBE=true allows listing collections without a live embedder.
        if std::env::var("VECDB_SKIP_PROBE").is_err() {
            embedder.dimension().await.map_err(|e| anyhow::anyhow!(
                "CRITICAL: Failed to initialize embedder: {}\n\
                The configured service is unreachable, or the model failed to load into memory.\n\
                 >> If using Ollama, verify that the 'ollama' service is running on the configured port.\n\
                 >> Verify that the requested model name is exact and the weights are downloaded.", e
            ))?;
        }

        Ok(Self {
            backend: Arc::new(backend),
            embedder,
            file_detector,
            parser_factory,
            smart_routing_keys,
            path_rules,
            max_concurrent_requests,
            gpu_batch_size,
        })
    }

    /// Borrow the embedder for lifecycle operations (release, model_name probes).
    /// Used by the server's idle-eviction watchdog.
    pub fn embedder(&self) -> &Arc<dyn Embedder + Send + Sync> {
        &self.embedder
    }

    #[allow(clippy::too_many_arguments)]
    /// Create a new Core instance from existing backends
    pub fn with_backends(
        backend: Arc<dyn Backend + Send + Sync>,
        embedder: Arc<dyn Embedder + Send + Sync>,
        file_detector: Arc<dyn FileTypeDetector>,
        parser_factory: Arc<dyn ParserFactory>,
        smart_routing_keys: Vec<String>,
        path_rules: Vec<crate::config::PathRule>,
        max_concurrent_requests: usize,
        gpu_batch_size: usize,
    ) -> Self {
        Self {
            backend,
            embedder,
            file_detector,
            parser_factory,
            smart_routing_keys,
            path_rules,
            max_concurrent_requests,
            gpu_batch_size,
        }
    }

    /// Passthrough to Backend::search with automatic embedding
    pub async fn search(
        &self,
        collection: &str,
        query: &str,
        limit: u64,
        filter: Option<serde_json::Value>,
    ) -> Result<Vec<SearchResult>> {
        // Automatically resolve collection dimension for self-healing (Matryoshka support)
        let target_dim = match self.backend.get_collection_info(collection).await {
            Ok(info) => info.vector_size.map(|s| s as usize),
            Err(_) => None,
        };

        // Embed the query with the target dimension
        let vector = self.embedder.embed(query, target_dim).await?;

        // Search
        self.backend
            .search(collection, &vector, limit, filter)
            .await
    }

    /// Ingest a file or directory with per-file .vecdbrc routing.
    /// When `routes` is provided, each file is routed to its matching collection
    /// instead of using a single collection for everything.
    /// `collection` serves as the fallback when no route matches.
    #[allow(clippy::too_many_arguments)]
    pub async fn ingest_routed(
        &self,
        path: &str,
        collection: &str,
        routes: Vec<crate::vecdbrc::Route>,
        vecdbrc_root: std::path::PathBuf,
        recursive: bool,
        chunk_size: Option<usize>,
        max_chunk_size: Option<usize>,
        chunk_overlap: Option<usize>,
        extensions: Option<Vec<String>>,
        excludes: Option<Vec<String>>,
        dry_run: bool,
        metadata: Option<std::collections::HashMap<String, serde_json::Value>>,
        concurrency: Option<usize>,
        gpu_concurrency: Option<usize>,
        quantization: Option<config::QuantizationType>,
        target_dim: Option<usize>,
        ignore_vectorignore: bool,
    ) -> Result<()> {
        let options = IngestionOptions {
            path: path.to_string(),
            collection: collection.to_string(),
            vecdbrc_routes: Some(routes),
            vecdbrc_root: Some(vecdbrc_root),
            chunk_size: chunk_size.unwrap_or(config::DEFAULT_CHUNK_SIZE),
            max_chunk_size,
            chunk_overlap: chunk_overlap.unwrap_or(50),
            respect_gitignore: recursive,
            ignore_vectorignore,
            strategy: "recursive".to_string(),
            tokenizer: "cl100k_base".to_string(),
            git_ref: None,
            extensions,
            excludes,
            dry_run,
            metadata,
            file_allowlist: None,
            project_root: None,
            path_rules: self.path_rules.clone(),
            max_concurrent_requests: concurrency
                .unwrap_or(self.max_concurrent_requests),
            gpu_batch_size: gpu_concurrency.unwrap_or(self.gpu_batch_size),
            quantization,
        };

        ingestion::ingest_path(
            &self.backend,
            &self.embedder,
            &self.file_detector,
            &self.parser_factory,
            options,
            target_dim,
        )
        .await
    }

    /// Ingest a file or directory
    #[allow(clippy::too_many_arguments)]
    pub async fn ingest(
        &self,
        path: &str,
        collection: &str,
        recursive: bool,
        chunk_size: Option<usize>,
        max_chunk_size: Option<usize>,
        chunk_overlap: Option<usize>,
        extensions: Option<Vec<String>>,
        excludes: Option<Vec<String>>,
        dry_run: bool,
        metadata: Option<std::collections::HashMap<String, serde_json::Value>>,
        concurrency: Option<usize>,
        gpu_concurrency: Option<usize>,
        quantization: Option<config::QuantizationType>,
        target_dim: Option<usize>,
        ignore_vectorignore: bool,
    ) -> Result<()> {
        // DIMENSION SAFETY GUARD: Before ingesting, verify the embedder dimension
        // is compatible with any existing collection. This prevents silently mixing
        // vectors of different dimensions, which produces garbage search results.
        if let Ok(info) = self.backend.get_collection_info(collection).await {
            if let Some(stored_dim) = info.vector_size {
                let embedder_dim = self.embedder.dimension().await? as u64;
                let effective_dim = target_dim.map(|d| d as u64).unwrap_or(embedder_dim);

                if effective_dim != stored_dim && info.vector_count.unwrap_or(0) > 0 {
                    return Err(anyhow::anyhow!(
                        "Dimension mismatch: embedder produces {}-dim vectors (effective: {}-dim) \
                         but collection '{}' already contains {} vectors at {}-dim. \
                         To fix: either (1) delete the collection and re-ingest, \
                         or (2) change your local_embedding_model in config.toml to match.",
                        embedder_dim, effective_dim, collection,
                        info.vector_count.unwrap_or(0), stored_dim
                    ));
                }
            }
        }
        // Guard passed (or collection doesn't exist yet — it will be created at correct dim)

        let options = IngestionOptions {
            path: path.to_string(),
            collection: collection.to_string(),
            vecdbrc_routes: None,
            vecdbrc_root: None,
            chunk_size: chunk_size.unwrap_or(config::DEFAULT_CHUNK_SIZE),
            max_chunk_size,
            chunk_overlap: chunk_overlap.unwrap_or(50),
            respect_gitignore: recursive,
            ignore_vectorignore,
            strategy: "recursive".to_string(),
            tokenizer: "cl100k_base".to_string(),
            git_ref: None,
            extensions,
            excludes,
            dry_run,
            metadata,
            file_allowlist: None,
            project_root: None,
            path_rules: self.path_rules.clone(),
            max_concurrent_requests: concurrency
                .unwrap_or(self.max_concurrent_requests),
            gpu_batch_size: gpu_concurrency.unwrap_or(self.gpu_batch_size),
            quantization,
        };

        ingestion::ingest_path(
            &self.backend,
            &self.embedder,
            &self.file_detector,
            &self.parser_factory,
            options,
            target_dim,
        )
        .await
    }

    /// Ingest with full control over IngestionOptions.
    /// Allows passing `file_allowlist` for multi-file glob batching and
    /// `project_root` for topographic metadata. The standard `ingest()`
    /// method sets these to None; use this when you need them.
    #[allow(clippy::too_many_arguments)]
    pub async fn ingest_with_options(
        &self,
        options: IngestionOptions,
        target_dim: Option<usize>,
    ) -> Result<()> {
        // Dimension safety guard (same as ingest)
        if let Ok(info) = self.backend.get_collection_info(&options.collection).await {
            if let Some(stored_dim) = info.vector_size {
                let embedder_dim = self.embedder.dimension().await? as u64;
                let effective_dim = target_dim.map(|d| d as u64).unwrap_or(embedder_dim);
                if effective_dim != stored_dim && info.vector_count.unwrap_or(0) > 0 {
                    return Err(anyhow::anyhow!(
                        "Dimension mismatch: embedder produces {}-dim vectors (effective: {}-dim) \
                         but collection '{}' already contains {} vectors at {}-dim. \
                         To fix: either (1) delete the collection and re-ingest, \
                         or (2) change your local_embedding_model in config.toml to match.",
                        embedder_dim, effective_dim, options.collection,
                        info.vector_count.unwrap_or(0), stored_dim
                    ));
                }
            }
        }
        ingestion::ingest_path(
            &self.backend,
            &self.embedder,
            &self.file_detector,
            &self.parser_factory,
            options,
            target_dim,
        )
        .await
    }

/// Smart search that routes to specific collections or applies filters based on metadata facets.
/// Uses DynamicRouter with a built-in timeout to prevent hanging on complex facet queries.
pub async fn search_smart(
    &self,
    collection: &str,
    query: &str,
    limit: u64,
) -> Result<Vec<SearchResult>> {
    // Use DynamicRouter to detect version/theme facets
    // NOW monitoring keys defined in Config (defaults: version, language, source_type)
    let router = DynamicRouter::new(self.backend.clone(), self.smart_routing_keys.clone());

    // Apply timeout to smart routing: if it takes > 5s, fall back to plain search
    let (detected_filters, clean_query) = match tokio::time::timeout(
        std::time::Duration::from_secs(5),
        router.route(collection, query)
    ).await {
        Ok(Ok(result)) => result,
        _ => {
            // Timeout or error - fall back to plain search
            if output::OUTPUT.is_interactive {
                eprintln!("Smart Route: Timeout on facet detection, falling back to plain search");
            }
            (serde_json::Map::new(), query.to_string())
        }
    };

    let filter = if !detected_filters.is_empty() {
        Some(serde_json::Value::Object(detected_filters))
    } else {
        None
    };

    if let Some(f) = &filter {
        if output::OUTPUT.is_interactive {
            eprintln!(
                "Smart Route: Applying filter {:?} to query '{}'",
                f, clean_query
            );
        }
    }

    self.search(collection, &clean_query, limit, filter).await
}

    #[allow(clippy::too_many_arguments)]
    /// Ingest raw content directly (Push Interface)
    pub async fn ingest_content(
        &self,
        content: &str,
        metadata: std::collections::HashMap<String, serde_json::Value>,
        collection: &str,
        chunk_size: Option<usize>,
        max_chunk_size: Option<usize>,
        chunk_overlap: Option<usize>,
        quantization: Option<config::QuantizationType>,
        target_dim: Option<usize>,
    ) -> Result<()> {
        // We need to update ingestion::ingest_memory signature too or IngestionOptions just needs it set?
        // ingestion::ingest_memory creates its own IngestionOptions. I need to update it to accept quantization arg effectively or pass it.
        // Wait, ingest_memory signature in lib.rs calls ingestion::ingest_memory.
        // I need to update ingestion::ingest_memory signature in `ingestion/mod.rs` first?
        // I already updated mod.rs? No, I updated `ingest_path` call usage, but `ingest_memory` function signature in `mod.rs` was likely NOT updated to take the arg, only its *internal* struct init.
        // Checking my memory/logs on Step 123...
        // I updated `backend.create_collection` call in `ingest_memory`, but did I update the function arguments? No.
        // I updated `options` struct creation to `quantization: None`.
        // So I need to update `ingestion::ingest_memory` signature in `mod.rs` as well.
        // Let's assume I will do that in next step or use multi_replace here if possible? No, different file.
        // I will update this file to assume `ingestion::ingest_memory` takes it.
        ingestion::ingest_memory(
            &self.backend,
            &self.embedder,
            content,
            metadata,
            collection,
            chunk_size,
            max_chunk_size,
            chunk_overlap,
            quantization,
            target_dim,
        )
        .await
    }

    /// Generate embeddings for a list of texts (Tool Access)
    pub async fn embed(&self, texts: Vec<String>) -> Result<Vec<Vec<f32>>> {
        self.embedder.embed_batch(&texts, None).await
    }

    /// Ingest a historic version of a repository (Time Travel)
    pub async fn ingest_history(
        &self,
        path: &str,
        git_ref: &str,
        collection: &str,
        chunk_size: usize,
        quantization: Option<config::QuantizationType>,
        target_dim: Option<usize>,
    ) -> Result<()> {
        // history::ingest_history also needs update
        crate::history::ingest_history(
            &self.backend,
            &self.embedder,
            &self.file_detector,
            &self.parser_factory,
            path,
            git_ref,
            collection,
            chunk_size,
            quantization,
            target_dim,
        )
        .await
    }

    /// List all available collections with metadata
    pub async fn list_collections(&self) -> Result<Vec<types::CollectionInfo>> {
        let names = self.backend.list_collections().await?;
        let mut infos = Vec::new();

        for name in names {
            match self.backend.get_collection_info(&name).await {
                Ok(info) => infos.push(info),
                Err(_) => {
                    // If we can't get info, still include the collection with minimal data
                    infos.push(types::CollectionInfo {
                        name,
                        vector_count: None,
                        vector_size: None,
                        quantization: None,
                    });
                }
            }
        }

        Ok(infos)
    }

    /// Delete a collection.
    ///
    /// Removes the collection from Qdrant.  Local `.vecdb/state.toml` files
    /// referencing this collection become stale — they are not removed here
    /// because the re-ingest path detects the UUID mismatch and clears them
    /// automatically.  A future `vecdb cleanup` command can surface and prune
    /// orphaned state entries if desired.
    pub async fn delete_collection(&self, collection: &str) -> Result<()> {
        self.backend.delete_collection(collection).await
    }

    /// Get the dimension of the configured embedding model
    pub async fn get_embedding_dimension(&self) -> Result<usize> {
        self.embedder.dimension().await
    }

    // Removed misplaces doc comment
    // code_query removed from Core - use vecq directly in CLI/Server

    /// Optimize collection (apply quantization)
    pub async fn optimize_collection(
        &self,
        collection: &str,
        quantization: config::QuantizationType,
    ) -> Result<()> {
        self.backend
            .update_collection_quantization(collection, quantization)
            .await
    }

    /// List background tasks from the backend
    pub async fn list_tasks(&self) -> Result<Vec<types::TaskInfo>> {
        self.backend.list_tasks().await
    }
}

/// Retrieve the version of the underlying ONNX Runtime (if available)
pub fn get_ort_version() -> String {
    #[cfg(feature = "cuda")]
    {
        // Environmental truth verified via strings/nm
        "1.23.2".to_string()
    }
    #[cfg(not(feature = "cuda"))]
    {
        "N/A (No CUDA/ORT)".to_string()
    }
}

/// Retrieve the active ONNX Runtime Execution Providers
pub fn get_ort_providers() -> Vec<String> {
    #[cfg(feature = "cuda")]
    {
        // If copy-device-mem exposed the full table, maybe this exists now
        // match ort::api().get_available_providers() { ... }

        // Falling back to raw call which we confirmed exists (as field)
        use std::ffi::CStr;
        let api = ort::api();
        let mut providers = Vec::new();
        unsafe {
            let mut out_ptr: *mut *mut std::ffi::c_char = std::ptr::null_mut();
            let mut count: i32 = 0;
            let _ = (api.GetAvailableProviders)(&mut out_ptr as *mut _ as *mut _, &mut count);
            if !out_ptr.is_null() && count > 0 {
                for i in 0..count {
                    let p_ptr = *out_ptr.offset(i as isize);
                    if !p_ptr.is_null() {
                        providers.push(CStr::from_ptr(p_ptr).to_string_lossy().into_owned());
                    }
                }
            }
        }
        if providers.is_empty() {
            providers.push("CPUExecutionProvider".to_string());
        }
        providers
    }
    #[cfg(not(feature = "cuda"))]
    {
        vec!["CPU (Default)".to_string()]
    }
}

// Optional: Facade re-exports if we want a flat namespace
// pub use backend::Backend;
// pub use types::{Document, Chunk, SearchResult};
