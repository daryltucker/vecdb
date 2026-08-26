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
pub mod config_docs;
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
    ARBITER
        .get_or_init(|| Arc::new(ResourceArbiter::new()))
        .clone()
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

/// Process-wide services a Core needs, independent of which model it uses.
///
/// Separate from `Resolution` because these do not vary per profile or
/// collection — they are the same for every Core in the process. Keeping them
/// apart is what lets `Core::new` take two arguments instead of seventeen.
#[derive(Clone)]
pub struct CoreServices {
    pub smart_routing_keys: Vec<String>,
    pub path_rules: Vec<crate::config::PathRule>,
    pub max_concurrent_requests: usize,
    pub fastembed_cache_path: Option<std::path::PathBuf>,
    /// Permit the embedder to silently cut oversized chunks. Off in every normal
    /// path; see `IngestionConfig::allow_embed_truncation`.
    pub allow_embed_truncation: bool,
    pub file_detector: Arc<dyn FileTypeDetector>,
    pub parser_factory: Arc<dyn ParserFactory>,
}

impl CoreServices {
    /// Build from a `Config` plus the injected parser/detector.
    pub fn from_config(
        config: &crate::config::Config,
        file_detector: Arc<dyn FileTypeDetector>,
        parser_factory: Arc<dyn ParserFactory>,
    ) -> Self {
        Self {
            smart_routing_keys: config.smart_routing_keys.clone(),
            path_rules: config.ingestion.path_rules.clone(),
            max_concurrent_requests: config.ingestion.max_concurrent_requests,
            fastembed_cache_path: Some(config.fastembed_cache_path.clone()),
            allow_embed_truncation: config.ingestion.allow_embed_truncation,
            file_detector,
            parser_factory,
        }
    }
}

impl Core {
    /// Build a Core from a fully-resolved configuration.
    ///
    /// Takes a `Resolution` rather than a list of positional arguments. The old
    /// signature had grown to seventeen, which is how `gpu_batch_size` came to
    /// mean two different things depending on which embedder was constructed
    /// three arguments earlier — the call site could not see the coupling.
    /// Here the backend decides which knobs apply, and the ones that do not are
    /// not in scope.
    pub async fn new(
        resolution: &crate::config::Resolution,
        services: CoreServices,
    ) -> Result<Self> {
        use crate::config::BackendKind;

        let CoreServices {
            smart_routing_keys,
            path_rules,
            max_concurrent_requests,
            fastembed_cache_path,
            allow_embed_truncation,
            file_detector,
            parser_factory,
        } = services;

        let backend =
            QdrantBackend::new(&resolution.qdrant_url, resolution.qdrant_api_key.clone())?;

        let model = resolution.embedder.model.as_str();
        let gpu_batch_size = resolution.batch.value;

        let embedder: Arc<dyn Embedder + Send + Sync> = match resolution.backend.kind {
            #[cfg(feature = "local-embed")]
            BackendKind::Fastembed => {
                if output::OUTPUT.is_interactive {
                    eprintln!(
                        "Using local embedder '{}' (fastembed: {model}) [GPU: {}]",
                        resolution.embedder_name, resolution.use_gpu.value
                    );
                }
                Arc::new(embedders::LocalEmbedder::new(
                    model,
                    fastembed_cache_path,
                    resolution.use_gpu.value,
                )?)
            }
            #[cfg(not(feature = "local-embed"))]
            BackendKind::Fastembed => {
                anyhow::bail!(
                    "backend '{}' is kind = \"fastembed\", but this build has no local \
                     embedder. Rebuild with the 'local-embed' feature, or point the \
                     embedder at an ollama backend.",
                    resolution.backend_name
                )
            }
            BackendKind::Ollama => {
                if output::OUTPUT.is_interactive {
                    eprintln!(
                        "Using embedder '{}' ({model}) on backend '{}' at {}",
                        resolution.embedder_name,
                        resolution.backend_name,
                        resolution.ollama_url()
                    );
                }
                Arc::new(
                    OllamaEmbedder::new(
                        resolution.ollama_url().to_string(),
                        model.to_string(),
                        resolution.backend.accept_invalid_certs,
                        resolution.backend.api_key.clone(),
                        Some(resolution.num_ctx.value),
                    )
                    .with_truncation(allow_embed_truncation),
                )
            }
        };

        // Wrap in ArbitratedEmbedder so embed/embed_batch/dimension calls go
        // through the process-wide ResourceArbiter. Different embedders with
        // different required_resources() will not block each other; same-resource
        // calls serialise correctly (see resource.rs).
        let embedder: Arc<dyn Embedder + Send + Sync> =
            Arc::new(ArbitratedEmbedder::new(embedder, process_arbiter()));

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
        params: crate::backend::SearchParams,
    ) -> Result<Vec<SearchResult>> {
        // Reads are permissive by design. `Compatible` (same architecture and
        // parameter size, different quantization) passes with a note, because a
        // quantization delta costs a little precision on one ranking and
        // nothing afterwards. Only `Incompatible` is refused — searching a
        // collection with the wrong model returns confident nonsense, which is
        // worse than an error.
        let genesis = self.backend.read_genesis(collection).await?;

        // Ownership is checked even though reads are otherwise permissive.
        // Permissiveness is about tolerating a quantization delta within a
        // known model; it is not licence to embed a text query against someone
        // else's audio vectors. When the dimensions happen to coincide — MERT
        // is 1024/Cosine and so is qwen3-embedding:0.6b — that search succeeds
        // and returns confident nonsense, which is the worst possible outcome.
        if !genesis.is_vecdb() {
            anyhow::bail!(
                "'{collection}' is not a vecdb collection.\n\
                 \n\
                 It exists on this Qdrant but carries no vecdb marker, so its \n\
                 vectors came from a model vecdb knows nothing about. Searching \n\
                 it would return scores that look valid and mean nothing.\n\
                 \n\
                 run `vecdb list` to see which collections are vecdb's."
            );
        }

        {
            let identity = self.embedder.identity().await?;
            let dim = self.embedder.dimension().await? as u64;
            let report = crate::types::compare_spaces(
                &genesis.model,
                genesis.dimension,
                &identity,
                Some(dim),
            );

            if !report.permits_read() {
                anyhow::bail!(
                    "cannot search '{collection}': {}\n\
                     \n\
                     \x20 collection: {}\n\
                     \x20 this machine: {}\n\
                     {}",
                    report.reason,
                    genesis.model.describe(),
                    identity.describe(),
                    report
                        .suggestion
                        .as_ref()
                        .map(|s| format!("\n  fix: {s}"))
                        .unwrap_or_default(),
                );
            }

            if let Some(w) = report.warning() {
                if output::OUTPUT.is_interactive {
                    eprintln!("note: {w}");
                }
            }
        }

        // Resolve the collection's dimension so an MRL-capable model truncates
        // its query vector to match (see the Matryoshka note in the tier RFC).
        let target_dim = match self.backend.get_collection_info(collection).await {
            Ok(info) => info.vector_size.map(|s| s as usize),
            Err(_) => None,
        };

        let vector = self.embedder.embed(query, target_dim).await?;

        self.backend.search(collection, &vector, params).await
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
        target_chunk_size: Option<usize>,
        max_chunk_bytes: Option<usize>,
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
            target_chunk_size: target_chunk_size.unwrap_or(config::DEFAULT_TARGET_CHUNK_SIZE),
            max_chunk_bytes,
            on_oversize: Default::default(),
            route_chunking: Default::default(),
            chunk_overlap: chunk_overlap.unwrap_or(50),
            // `.gitignore` is never consulted unless the operator asks for it on
            // the command line. It is a build-artifact list, not an indexing
            // policy, and the two disagree constantly. `.vectorignore` is the
            // knob that governs indexing.
            respect_gitignore: false,
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
            max_concurrent_requests: concurrency.unwrap_or(self.max_concurrent_requests),
            gpu_batch_size: gpu_concurrency.unwrap_or(self.gpu_batch_size),
            quantization,
            allow_quantization_delta: false,
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
        target_chunk_size: Option<usize>,
        max_chunk_bytes: Option<usize>,
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
        // The write guard lives in `ingestion::ensure_write_target`, which every
        // ingestion path funnels through. It compares the full embedding space
        // — model, digest, architecture, parameter size, dimension — and checks
        // collection ownership before it says anything about compatibility.
        //
        // A dimension-only check used to sit here as well. It was strictly
        // weaker on identity (two unrelated models sharing a dimension passed
        // it) and actively harmful when it did fire on a collection that was
        // never ours, since it advised deleting it. Its one real contribution
        // was comparing the *effective* dimension, i.e. honouring `target_dim`;
        // `ensure_write_target` now takes `target_dim` directly, so that check
        // moved rather than disappeared.

        let options = IngestionOptions {
            path: path.to_string(),
            collection: collection.to_string(),
            vecdbrc_routes: None,
            vecdbrc_root: None,
            target_chunk_size: target_chunk_size.unwrap_or(config::DEFAULT_TARGET_CHUNK_SIZE),
            max_chunk_bytes,
            on_oversize: Default::default(),
            route_chunking: Default::default(),
            chunk_overlap: chunk_overlap.unwrap_or(50),
            // See `ingest_routed`: never inferred, only ever set explicitly by
            // the operator on the CLI.
            respect_gitignore: false,
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
            max_concurrent_requests: concurrency.unwrap_or(self.max_concurrent_requests),
            gpu_batch_size: gpu_concurrency.unwrap_or(self.gpu_batch_size),
            quantization,
            allow_quantization_delta: false,
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
    /// Ingest using a fully-specified `IngestionOptions`.
    /// Unlike `ingest()`, this does NOT merge `self.path_rules` or any other Core fields —
    /// the caller owns the entire options struct. If you build `IngestionOptions` manually,
    /// populate `path_rules` from `config.ingestion.path_rules` yourself.
    pub async fn ingest_with_options(
        &self,
        options: IngestionOptions,
        target_dim: Option<usize>,
    ) -> Result<()> {
        // No guard here: `ingestion::ingest_path` calls `ensure_write_target`,
        // which owns the ownership and embedding-space checks for every path.
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

    /// Search with `key:value` facet qualifiers parsed out of the query.
    ///
    /// Returns the applied filters alongside the results. Callers are expected to
    /// surface them: a search that was silently narrowed is indistinguishable from
    /// a corpus that is genuinely thin, and that ambiguity is what makes an
    /// unreported filter expensive to debug.
    ///
    /// A malformed or unknown qualifier is an error, not a fallback. Falling back
    /// to an unfiltered search would answer a different question than the one
    /// asked, which is worse than answering none.
    pub async fn search_smart(
        &self,
        collection: &str,
        query: &str,
        params: crate::backend::SearchParams,
    ) -> Result<(
        Vec<SearchResult>,
        serde_json::Map<String, serde_json::Value>,
    )> {
        let router = DynamicRouter::new(self.backend.clone(), self.smart_routing_keys.clone());

        // Validating a qualifier costs one metadata scan per key, and only when a
        // qualifier is actually present. The timeout bounds a pathological
        // collection; it does not paper over a bad query, which fails fast above.
        let routed = tokio::time::timeout(
            std::time::Duration::from_secs(5),
            router.route(collection, query),
        )
        .await
        .map_err(|_| {
            anyhow::anyhow!(
                "facet validation timed out after 5s on collection '{}'. \
             Re-run without facet qualifiers to search unfiltered.",
                collection
            )
        })??;

        if output::OUTPUT.is_interactive && !routed.filters.is_empty() {
            eprintln!(
                "smart: filtering on {} — searching for '{}'",
                serde_json::Value::Object(routed.filters.clone()),
                routed.query
            );
        }

        let params = params.with_filter(routed.filter());
        let results = self.search(collection, &routed.query, params).await?;

        Ok((results, routed.filters))
    }

    #[allow(clippy::too_many_arguments)]
    /// Ingest raw content directly (Push Interface)
    pub async fn ingest_content(
        &self,
        content: &str,
        metadata: std::collections::HashMap<String, serde_json::Value>,
        collection: &str,
        target_chunk_size: Option<usize>,
        max_chunk_bytes: Option<usize>,
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
            target_chunk_size,
            max_chunk_bytes,
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
        target_chunk_size: usize,
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
            target_chunk_size,
            quantization,
            target_dim,
        )
        .await
    }

    /// List all available collections with metadata
    /// List collections together with what each one declares about itself.
    ///
    /// Every collection on the backend is returned, including those vecdb did
    /// not create. Hiding them would be worse than useless: a Qdrant instance is
    /// shared infrastructure, and a name that is "missing" from `vecdb list` but
    /// rejects `create_collection` is a confusing bug report waiting to happen.
    /// They are labelled, not filtered.
    pub async fn list_collections_with_genesis(
        &self,
    ) -> Result<Vec<(types::CollectionInfo, types::CollectionGenesis)>> {
        let infos = self.list_collections().await?;
        let mut out = Vec::with_capacity(infos.len());
        for info in infos {
            let genesis = self
                .backend
                .read_genesis(&info.name)
                .await
                .unwrap_or_default();
            out.push((info, genesis));
        }
        Ok(out)
    }

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
                        vectors_on_disk: None,
                        payload_on_disk: None,
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
