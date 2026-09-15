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
    // `path_rules`, `max_concurrent_requests` and `gpu_batch_size` were removed
    // 2026-257 with `Core::ingest`/`ingest_routed`. They existed only to fill in
    // `IngestionOptions` defaults for those wrappers; `ingest_with_options`
    // takes them from the caller, which is what stopped the MCP path silently
    // using a different granularity from the CLI.
}

/// Process-wide services a Core needs, independent of which model it uses.
///
/// Separate from `Resolution` because these do not vary per profile or
/// collection — they are the same for every Core in the process. Keeping them
/// apart is what lets `Core::new` take two arguments instead of seventeen.
#[derive(Clone)]
pub struct CoreServices {
    pub smart_routing_keys: Vec<String>,
    // `path_rules` and `max_concurrent_requests` were removed 2026-257. They
    // reached nothing: `Core` held them only to fill in `IngestionOptions` for
    // the deleted `ingest`/`ingest_routed` wrappers. Every caller of
    // `ingest_with_options` already supplies both from `config.ingestion`, which
    // is the point — a value the caller states cannot silently differ per entry
    // point, and that difference is what broke MCP ingest.
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
        // `ort_dylib_path` from config → ORT_DYLIB_PATH, unless the user
        // already set the variable — an explicit environment always wins
        // (same idiom as the ORT_INTRA_OP_NUM_THREADS auto-cap). This is the
        // single config→runtime bridge every binary crosses, and it runs
        // before any ort symbol is touched, which is what makes the
        // cuda-dynamic install a one-time config edit instead of an env var
        // that every shell, hook, and MCP block must remember.
        if let Some(path) = &config.ort_dylib_path {
            if std::env::var_os("ORT_DYLIB_PATH").is_none() {
                unsafe {
                    std::env::set_var("ORT_DYLIB_PATH", path);
                }
            }
        }

        Self {
            smart_routing_keys: config.smart_routing_keys.clone(),
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
            fastembed_cache_path,
            allow_embed_truncation,
            file_detector,
            parser_factory,
        } = services;

        let backend =
            QdrantBackend::new(&resolution.qdrant_url, resolution.qdrant_api_key.clone())?;

        let model = resolution.embedder.model.as_str();

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
        })
    }

    /// Borrow the embedder for lifecycle operations (release, model_name probes).
    /// Used by the server's idle-eviction watchdog.
    pub fn embedder(&self) -> &Arc<dyn Embedder + Send + Sync> {
        &self.embedder
    }

    /// Create a new Core instance from existing backends
    pub fn with_backends(
        backend: Arc<dyn Backend + Send + Sync>,
        embedder: Arc<dyn Embedder + Send + Sync>,
        file_detector: Arc<dyn FileTypeDetector>,
        parser_factory: Arc<dyn ParserFactory>,
        smart_routing_keys: Vec<String>,
    ) -> Self {
        Self {
            backend,
            embedder,
            file_detector,
            parser_factory,
            smart_routing_keys,
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

    // `Core::ingest` and `Core::ingest_routed` were removed 2026-257.
    //
    // Both were convenience wrappers that built `IngestionOptions` on the
    // caller's behalf — and what they filled in was wrong. Each pinned
    // `pack_target_bytes: None`, so `chunking_identity()` recorded the 2048
    // default rather than the destination's configured granularity, and each
    // hardcoded `strategy: "recursive"` and `tokenizer: "cl100k_base"` past the
    // config. `ingest_routed` had no callers at all; `ingest` had exactly one,
    // the MCP `ingest_path` handler, which the defect broke outright once the
    // chunking guard started comparing what it recorded.
    //
    // `ingest_with_options` below is the single entry point. A caller states
    // every parameter it means, which is what makes omitting one impossible
    // rather than merely discouraged. See vecdb-server/src/rpc/tools.rs.

    /// Ingest with full control over IngestionOptions.
    /// Allows passing `file_allowlist` for multi-file glob batching and
    /// `project_root` for topographic metadata. The standard `ingest()`
    /// method sets these to None; use this when you need them.
    #[allow(clippy::too_many_arguments)]
    /// Ingest using a fully-specified `IngestionOptions`.
    /// The caller owns the entire options struct: nothing is merged in from
    /// `Core`. Populate `path_rules`, `strategy`, `tokenizer` and the chunk
    /// parameters from the resolved config yourself — stating them is what keeps
    /// two entry points from quietly disagreeing.
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
        chunking: ingestion::options::ChunkSpec,
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
            chunking,
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

    /// Does this collection exist at the resolved store?
    ///
    /// Needed because Qdrant treats deleting an absent collection as success,
    /// so a caller cannot distinguish "removed it" from "there was nothing
    /// here" — which turns a wrong endpoint into a silent no-op reported as a
    /// deletion.
    pub async fn collection_exists(&self, collection: &str) -> Result<bool> {
        self.backend.collection_exists(collection).await
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

/// Re-exec with an absolute argv\[0\] so ONNX Runtime can find its CUDA
/// provider libraries. Call first thing in `main()`.
///
/// ORT resolves `libonnxruntime_providers_shared.so` /
/// `libonnxruntime_providers_cuda.so` relative to `dirname(argv[0])` — dladdr
/// on the main executable reports argv\[0\], and a bare name (normal PATH
/// invocation: `vecdb ...`) degrades the anchor to the CWD, where the
/// libraries never are. The same binary invoked by absolute path (e.g.
/// `~/.cargo/bin/vecdb`) anchors correctly (BUG-2026-254; both behaviours
/// verified 2026-09-11).
///
/// So: if argv\[0\] is bare AND the provider libraries are installed next to
/// the real executable, replace the process with an absolute-path invocation
/// of itself. No-op in every other case — in particular CPU-only installs
/// (no provider libs present) never re-exec. Loop-safe: the re-exec'd child's
/// argv\[0\] contains a separator, so it returns at the first check.
#[cfg(unix)]
pub fn reexec_for_ort_provider_anchor() {
    use std::os::unix::process::CommandExt;

    let Some(argv0) = std::env::args_os().next() else {
        return;
    };
    if std::path::Path::new(&argv0).components().count() != 1 {
        return; // invoked via a path — ORT's anchor is already the binary's directory
    }
    let Ok(exe) = std::env::current_exe() else {
        return;
    };
    let Some(dir) = exe.parent() else { return };
    if !dir.join("libonnxruntime_providers_shared.so").exists() {
        return; // CPU-only install — nothing for ORT to anchor on anyway
    }
    let err = std::process::Command::new(&exe)
        .args(std::env::args_os().skip(1))
        .exec();
    // exec only returns on failure; continue un-anchored rather than dying.
    eprintln!("⚠️  re-exec for GPU provider resolution failed ({err}); if use_gpu = true, invoke {} by absolute path", exe.display());
}

/// Retrieve the version of the underlying ONNX Runtime (if available).
///
/// This used to return the literal `"1.23.2"`, commented "environmental truth
/// verified via strings/nm". It was true once, and then the prebuilt moved to
/// 1.24.2 while the literal did not — so `vecdb --version` published a number
/// nobody had measured, and docs/GPU.md copied it. A version we report must be
/// one we *asked for*, never one we remembered.
///
/// Statically linked builds can ask safely: `OrtGetApiBase` is a linked symbol,
/// so reading the version string touches no loader and cannot fail.
///
/// `cuda-dynamic` builds deliberately do NOT ask. There the runtime is whatever
/// `ORT_DYLIB_PATH` names, resolving it means a dlopen, and under ort
/// 2.0.0-rc.12 an API-version mismatch *hangs* instead of erroring
/// (BUG-2026-254). `--version` must never be able to hang, so it reports the
/// contract the binary requires and leaves the concrete answer to `vecdb
/// status`, which is already allowed to initialize an embedder.
pub fn get_ort_version() -> String {
    #[cfg(all(feature = "cuda", not(feature = "cuda-dynamic")))]
    {
        // SAFETY: statically linked ORT — the symbol is present, takes no
        // arguments, and returns a pointer to a 'static NUL-terminated string
        // owned by the runtime ("Do not deallocate the returned buffer").
        unsafe {
            let base = ort::sys::OrtGetApiBase();
            if base.is_null() {
                return "unknown (OrtGetApiBase returned null)".to_string();
            }
            let ptr = ((*base).GetVersionString)();
            if ptr.is_null() {
                return "unknown (GetVersionString returned null)".to_string();
            }
            std::ffi::CStr::from_ptr(ptr).to_string_lossy().into_owned()
        }
    }
    #[cfg(feature = "cuda-dynamic")]
    {
        format!(
            "dynamic — requires API {} via ORT_DYLIB_PATH (see docs/GPU_LEGACY.md)",
            ort::MINOR_VERSION
        )
    }
    #[cfg(not(feature = "cuda"))]
    {
        "N/A (No CUDA/ORT)".to_string()
    }
}

/// Retrieve the Execution Providers *compiled into* the ONNX Runtime build.
///
/// CAUTION: this is `GetAvailableProviders`, which reports build-time
/// availability only. A CUDA-enabled build lists `CUDAExecutionProvider` here
/// even when `libonnxruntime_providers_cuda.so` is absent and every session
/// runs on CPU. It must never be used to decide whether a session is actually
/// GPU-accelerated (BUG-2026-254); registration truth comes from creating the
/// session with `error_on_failure` on the dispatch (see `embedders/local.rs`).
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
