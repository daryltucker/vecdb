/*
 * PURPOSE:
 *   Local embedding implementation using fastembed (ONNX-based).
 *   Provides zero-config local embeddings without requiring Ollama.
 *
 * REQUIREMENTS:
 *   - CPU-based inference (GPU optional if available)
 *   - No external services required
 *   - Compatible with standard embedding models (AllMiniLM, BGE, etc.)
 *
 * IMPLEMENTATION RULES:
 *   1. Use fastembed's TextEmbedding for sync operations
 *   2. Wrap in tokio spawn_blocking for async compatibility
 *   3. Use Mutex for interior mutability (embed requires &mut self)
 */

use crate::embedder::Embedder;
use anyhow::{Context, Result};
use async_trait::async_trait;
use num_cpus;
use std::sync::Arc;

#[cfg(feature = "local-embed")]
use fastembed::{EmbeddingModel, InitOptions, TextEmbedding};
#[cfg(feature = "cuda")]
use ort::execution_providers::CUDAExecutionProvider;
#[cfg(feature = "local-embed")]
use std::sync::Mutex;

/// Local embedder using fastembed (ONNX Runtime).
/// Works out-of-the-box without Ollama or any external service.
///
/// Lifecycle: model weights load lazily on first embed() and can be released
/// via `release()` (e.g. by the server's idle-eviction watchdog). After release,
/// the next embed() reloads from `init_params` — those are persistent and never
/// consumed.
#[cfg(feature = "local-embed")]
pub struct LocalEmbedder {
    // Lazy: Model is None until first use, and may be set back to None by release().
    model: Arc<Mutex<Option<TextEmbedding>>>,
    // Persistent init params. Never consumed — used to (re)build the model on every
    // ensure_initialized() call, including after release(). Cheap to keep around.
    init_params: Arc<LocalEmbedderInitParams>,
    dimension: usize,
    model_name: String,
    use_gpu: bool,
}

#[cfg(feature = "local-embed")]
struct LocalEmbedderInitParams {
    model_type: EmbeddingModel,
    cache_path: Option<std::path::PathBuf>,
    use_gpu: bool,
}

#[cfg(feature = "local-embed")]
impl LocalEmbedder {
    /// Create a new LocalEmbedder with the specified model name.
    /// The model is downloaded automatically on first use.
    pub fn new(
        model_name: &str,
        cache_path: Option<std::path::PathBuf>,
        use_gpu: bool,
    ) -> Result<Self> {
        // Starvation Protection: Limit ONNX Runtime threads
        // Unless explicitly overridden by user, cap intra-op threads to a safe number (e.g., 4)
        // or 50% of logical cores, to prevent "System Lockup" during ingestion.
        if std::env::var("ORT_INTRA_OP_NUM_THREADS").is_err() {
            let num_cpus = num_cpus::get();
            let safe_threads = (num_cpus / 2).clamp(1, 2).to_string(); // Cap at 2 for background process stability
                                                                       // ORT (ONNX Runtime)
            unsafe {
                std::env::set_var("ORT_INTRA_OP_NUM_THREADS", &safe_threads);
            }
            // OpenMP (Torch/Many libs)
            unsafe {
                std::env::set_var("OMP_NUM_THREADS", &safe_threads);
            }
            // MKL (Math Kernel Library)
            unsafe {
                std::env::set_var("MKL_NUM_THREADS", &safe_threads);
            }

            if std::env::var("VECDB_DEBUG").is_ok() {
                eprintln!(
                    "[LocalEmbedder] Auto-limited math threads to {}",
                    safe_threads
                );
            }
        }

        // Map model name to fastembed enum.
        // IMPORTANT: Every alias MUST map to the correct underlying model.
        // DO NOT add aliases for models that fastembed-rs does not support.
        // If a model is not supported, it MUST produce an error, not a silent fallback.
        let model_type = match model_name.to_lowercase().as_str() {
            // all-MiniLM-L6-v2: 22M params, 384-dim, 256 tok context
            "all-minilm-l6-v2" | "minilm" | "default" | "" => EmbeddingModel::AllMiniLML6V2,
            // BGE Small EN v1.5: 33M params, 384-dim, 512 tok context
            "bge-small-en-v1.5" | "bge-small-en" | "baai/bge-small-en-v1.5" => {
                EmbeddingModel::BGESmallENV15
            }
            // BGE Base EN v1.5: 109M params, 768-dim, 512 tok context
            "bge-base-en-v1.5" | "bge-base-en" | "baai/bge-base-en-v1.5" => {
                EmbeddingModel::BGEBaseENV15
            }
            // BGE Large EN v1.5: 335M params, 1024-dim, 512 tok context
            "bge-large-en-v1.5" | "bge-large-en" | "baai/bge-large-en-v1.5" => {
                EmbeddingModel::BGELargeENV15
            }
            // Nomic Embed Text v1: 137M params, 768-dim, 8192 tok context
            "nomic-embed-text-v1" | "nomic-embed-text" | "nomic-v1" => {
                EmbeddingModel::NomicEmbedTextV1
            }
            // Nomic Embed Text v1.5: 137M params, 768-dim, 8192 tok context, Matryoshka-trained
            "nomic-embed-text-v1.5" | "nomic-v1.5" => EmbeddingModel::NomicEmbedTextV15,
            _ => {
                return Err(anyhow::anyhow!(
                    "Unknown local embedding model: '{}'. \
                     Supported models: all-minilm-l6-v2, bge-small-en-v1.5, bge-base-en-v1.5, \
                     bge-large-en-v1.5, nomic-embed-text-v1, nomic-embed-text-v1.5. \
                     Check your config.toml 'local_embedding_model' setting.",
                    model_name
                ));
            }
        };

        // Create the struct with lazy intent
        let instance = Self::with_model(model_type, cache_path, use_gpu)?;

        // CRITICAL: If GPU is requested, we MUST initialize EAGERLY.
        // This ensures that if CUDA fails and we fall back to CPU, the
        // warning messages are printed to stderr *HERE*, before any CLI
        // progress bars (like in `ingest`) are started.
        // If we wait for lazy init, the progress bar will swallow/overwrite the warnings.
        // However, we only do this in INTERACTIVE mode to prevent locking headless/MCP instances.
        // VECDB_SKIP_PROBE opt-out: commands like `list` and `delete` don't need an embedder at all.
        let skip_probe = std::env::var("VECDB_SKIP_PROBE").is_ok();
        if use_gpu && crate::output::OUTPUT.is_interactive && !skip_probe {
            tracing::debug!("Eagerly initializing LocalEmbedder for CUDA check...");
            if let Err(e) = instance.ensure_initialized() {
                // If init fails entirely (even fallback), we want to know now.
                // But ensure_initialized handles the fallback internally and effectively "succeeds"
                // with a CPU model if CUDA fails.
                // So if we get an Err here, it's a hard failure (network/disk).
                return Err(e).context("Failed eager initialization of LocalEmbedder");
            }
        }

        Ok(instance)
    }

    /// Create a LocalEmbedder with a specific model.
    pub fn with_model(
        model_type: EmbeddingModel,
        cache_path: Option<std::path::PathBuf>,
        use_gpu: bool,
    ) -> Result<Self> {
        // Get model info for dimension - this is lightweight
        let model_info =
            TextEmbedding::get_model_info(&model_type).context("Failed to get model info")?;
        let dimension = model_info.dim;
        let model_name = model_info.model_code.clone();

        // Store params for lazy init. Persistent — never consumed.
        let init_params = Arc::new(LocalEmbedderInitParams {
            model_type: model_type.clone(),
            cache_path: cache_path.clone(),
            use_gpu,
        });

        let instance = Self {
            model: Arc::new(Mutex::new(None)), // Uninitialized
            init_params,
            dimension,
            model_name,
            use_gpu,
        };

        // Eager init if GPU requested (consistency with ::new) and interactive.
        // Skipped if VECDB_SKIP_PROBE is set (non-embedding commands like list/delete).
        let skip_probe = std::env::var("VECDB_SKIP_PROBE").is_ok();
        if use_gpu && crate::output::OUTPUT.is_interactive && !skip_probe {
            tracing::debug!("Eagerly initializing LocalEmbedder (custom model) for CUDA check...");
            match instance.ensure_initialized() {
                Ok(_) => {}
                Err(e) => return Err(e).context("Failed eager initialization"),
            }
        }

        Ok(instance)
    }

    /// Internal helper to initialize the model on first use
    fn ensure_initialized(&self) -> Result<()> {
        let mut model_guard = self
            .model
            .lock()
            .map_err(|e| anyhow::anyhow!("Lock poisoned: {}", e))?;

        if model_guard.is_some() {
            return Ok(());
        }

        // Need to initialize. Params are persistent (Arc<LocalEmbedderInitParams>) —
        // borrow, don't consume, so subsequent reloads after release() also work.
        tracing::debug!("Lazy initializing LocalEmbedder...");

        let params = &*self.init_params;

        // Helper to construct base options
        let make_options = || {
            let mut options =
                InitOptions::new(params.model_type.clone()).with_show_download_progress(true);
            if let Some(path) = &params.cache_path {
                options = options.with_cache_dir(path.clone());
            }
            options
        };

        let model = {
            #[cfg(feature = "cuda")]
            if params.use_gpu {
                tracing::debug!("Initializing local embedder with CUDA acceleration");
                tracing::debug!("Attempting to create TextEmbedding with CUDA provider...");

                // Build GPU init closure so we can retry on OOM
                let try_init_gpu = || -> Result<TextEmbedding, anyhow::Error> {
                    let mut opts = make_options();
                    opts = opts
                        .with_execution_providers(vec![CUDAExecutionProvider::default().into()]);
                    TextEmbedding::try_new(opts).map_err(|e| anyhow::anyhow!("{}", e))
                };

                // Reduced from 30 to 5 in 2026-05 (Phase 2 resource arbitration).
                // The arbiter now serialises vecdb-vs-vecdb GPU contention via
                // flock, so this retry only needs to cover *external* VRAM
                // pressure (Ollama, training jobs). 5 attempts × exponential
                // backoff ≈ ~60s of patience, plenty for a foreign process to
                // free memory or for the user to react.
                let num_attempts = 5u32;
                let mut gpu_model: Option<Result<TextEmbedding, anyhow::Error>> = None;

                for attempt in 1..=num_attempts {
                    match try_init_gpu() {
                        Ok(m) => {
                            if attempt > 1 {
                                eprintln!(
                                    "✅ GPU available — initialized successfully after retry."
                                );
                            }
                            gpu_model = Some(Ok(m));
                            break;
                        }
                        Err(e) => {
                            let err_string = e.to_string();
                            let is_oom = err_string.contains("CUBLAS_STATUS_ALLOC_FAILED")
                                || err_string.contains("CUDA_ERROR_OUT_OF_MEMORY")
                                || err_string.contains("out of memory")
                                || err_string.contains("CUBLAS failure")
                                || err_string
                                    .contains("Failed to allocate memory for requested buffer");

                            if !is_oom {
                                gpu_model = Some(Err(e));
                                break;
                            }

                            if attempt < num_attempts {
                                let delay_secs = (2u64.pow(attempt.min(5))).min(30);
                                eprintln!(
                                    "\n⚠️  GPU busy (VRAM exhausted). Retrying in {}s (attempt {}/{})...",
                                    delay_secs, attempt, num_attempts
                                );
                                std::thread::sleep(std::time::Duration::from_secs(delay_secs));
                            } else {
                                gpu_model = Some(Err(e));
                                break;
                            }
                        }
                    }
                }

                match gpu_model
                    .unwrap_or_else(|| Err(anyhow::anyhow!("GPU init failed after all retries")))
                {
                    Ok(m) => {
                        // VERIFICATION: Even if try_new succeeds, ORT might have silently failed to register
                        // the CUDA provider and fallen back to CPU internally.
                        let active_providers = crate::get_ort_providers();
                        if active_providers.iter().any(|p| p.contains("CUDA")) {
                            eprintln!(
                                "✅ [CUDA] Local Embedder initialized successfully (GPU Accelerated)."
                            );
                        } else {
                            eprintln!("\n⚠️  [CUDA WARNING] GPU was requested but ORT initialization fell back to CPU.");
                            eprintln!("   This usually means libonnxruntime_providers_cuda.so is missing or incompatible.");
                            eprintln!("   Check docs/GPU.md for installation instructions.\n");
                        }
                        m
                    }
                    Err(e) => {
                        eprintln!(
                            "\n❌ [CUDA FAILURE] GPU initialization failed after {} retries.",
                            num_attempts
                        );
                        eprintln!("   Last error: {}", e);
                        eprintln!("   Troubleshooting:");
                        eprintln!("     1. GPU may be occupied by another process (Ollama, training job, etc.)");
                        eprintln!("     2. Run 'nvidia-smi' to check what's using VRAM");
                        eprintln!("     3. Set 'local_use_gpu = false' in config.toml to use CPU instead\n");

                        return Err(e).context(format!(
                            "Local embedder failed to initialize with GPU after {} retries",
                            num_attempts
                        ));
                    }
                }
            } else {
                TextEmbedding::try_new(make_options())
                    .context("Failed to initialize local embedding model")?
            }

            #[cfg(not(feature = "cuda"))]
            {
                if params.use_gpu {
                    tracing::warn!("GPU acceleration requested but 'cuda' feature not enabled. Falling back to CPU.");
                    eprintln!("\n⚠️  [CUDA WARNING] 'local_use_gpu = true' but binary was compiled without 'cuda' feature.");
                    eprintln!("   falling back to CPU.\n");
                    if !cfg!(test) {
                        std::thread::sleep(std::time::Duration::from_secs(3));
                    }
                }
                TextEmbedding::try_new(make_options())
                    .context("Failed to initialize local embedding model")?
            }
        };

        *model_guard = Some(model);
        tracing::debug!("LocalEmbedder initialized successfully.");
        Ok(())
    }
}

/// Release GPU VRAM by dropping the loaded model.
/// After calling this, the embedder will reload the model on the next embed() call.
#[cfg(feature = "local-embed")]
impl LocalEmbedder {
    pub fn release_gpu(&self) {
        if let Ok(mut guard) = self.model.lock() {
            if guard.is_some() {
                tracing::debug!("Releasing LocalEmbedder GPU model (freeing VRAM)...");
                *guard = None;
                tracing::debug!("LocalEmbedder GPU model released.");
            }
        }
    }
}

/// Check if an error is a CUDA/GPU memory failure and wrap with human-readable message.
#[cfg(feature = "local-embed")]
fn wrap_cuda_error(err: anyhow::Error) -> anyhow::Error {
    let msg = err.to_string();
    let is_cuda_oom = msg.contains("CUBLAS_STATUS_ALLOC_FAILED")
        || msg.contains("CUDA_ERROR_OUT_OF_MEMORY")
        || msg.contains("out of memory")
        || msg.contains("CUBLAS failure")
        || msg.contains("Failed to allocate memory for requested buffer");

    if is_cuda_oom {
        anyhow::anyhow!(
            "GPU out of memory (VRAM exhausted).\n\
             \n\
             The GPU does not have enough free VRAM to run the embedding model.\n\
             Common causes:\n\
               • Another process is using the GPU (Ollama, a training job, etc.)\n\
               • The model is too large for your GPU's VRAM\n\
             \n\
             To fix:\n\
               1. Free GPU memory: stop other GPU processes (e.g. 'docker stop ollama-...')\n\
               2. Check usage: run 'nvidia-smi' to see what's consuming VRAM\n\
               3. Fall back to CPU: set 'local_use_gpu = false' in config.toml\n\
             \n\
             Technical detail: {}",
            msg
        )
    } else {
        err.context("Embedding failed")
    }
}

#[cfg(feature = "local-embed")]
#[async_trait]
impl Embedder for LocalEmbedder {
    async fn embed(&self, text: &str, target_dim: Option<usize>) -> Result<Vec<f32>> {
        let myself = self.clone();
        let text_owned = text.to_string();

        let result = tokio::task::spawn_blocking(move || {
            // Lazy Init
            myself.ensure_initialized()?;

            let mut guard = myself
                .model
                .lock()
                .map_err(|e| anyhow::anyhow!("Lock poisoned: {}", e))?;
            let model = guard
                .as_mut()
                .ok_or_else(|| anyhow::anyhow!("Model not initialized"))?;
            model.embed(vec![text_owned], None).map_err(wrap_cuda_error)
        })
        .await
        .context("Embedding task panicked")??;

        let mut vec = result
            .into_iter()
            .next()
            .ok_or_else(|| anyhow::anyhow!("No embedding returned"))?;

        if let Some(dim) = target_dim {
            if dim < vec.len() {
                vec.truncate(dim);
                crate::embedder::l2_normalize(&mut vec);
            }
        }

        Ok(vec)
    }

    async fn embed_batch(
        &self,
        texts: &[String],
        target_dim: Option<usize>,
    ) -> Result<Vec<Vec<f32>>> {
        let myself = self.clone();
        let texts_owned: Vec<String> = texts.to_vec();

        let mut results = tokio::task::spawn_blocking(move || {
            // Lazy Init
            myself.ensure_initialized()?;

            let mut guard = myself
                .model
                .lock()
                .map_err(|e| anyhow::anyhow!("Lock poisoned: {}", e))?;
            let model = guard
                .as_mut()
                .ok_or_else(|| anyhow::anyhow!("Model not initialized"))?;
            model.embed(texts_owned, None).map_err(wrap_cuda_error)
        })
        .await
        .context("Embedding batch task panicked")??;

        if let Some(dim) = target_dim {
            for vec in results.iter_mut() {
                if dim < vec.len() {
                    vec.truncate(dim);
                    crate::embedder::l2_normalize(vec);
                }
            }
        }

        Ok(results)
    }

    async fn dimension(&self) -> Result<usize> {
        Ok(self.dimension)
    }

    fn model_name(&self) -> String {
        format!("fastembed:{}", self.model_name)
    }

    /// Identity for a fastembed model.
    ///
    /// Unlike an Ollama tag, `model_code` *is* the identity: each `EmbeddingModel`
    /// variant pins a specific ONNX artifact, so two `LocalEmbedder`s reporting
    /// the same code are running the same weights. There is no separate digest
    /// to fetch and no quantization axis the operator can vary, so the code is
    /// promoted into the digest slot directly.
    ///
    /// This must not fall through to the name-only default: the guard treats
    /// absent identity as "cannot establish compatibility" and refuses the
    /// write, which would reject a collection against the very model that
    /// created it.
    async fn identity(&self) -> Result<crate::types::ModelIdentity> {
        Ok(crate::types::ModelIdentity {
            name: self.model_name(),
            digest: Some(format!("fastembed:{}", self.model_name)),
            architecture: Some(fastembed_architecture(&self.model_name).to_string()),
            family: Some(fastembed_architecture(&self.model_name).to_string()),
            // fastembed exposes no parameter count, and none is needed: the
            // pinned artifact makes the digest authoritative on its own.
            parameter_size: None,
            quantization_level: None,
            embedding_length: Some(self.dimension as u64),
            context_length: None,
        })
    }

    /// Drops the loaded ONNX model, freeing the bulk of VRAM/RAM held by the
    /// session. The CUDA context (~80 MiB) is process-global and stays until
    /// the process exits — that's normal. The next embed() call reloads via
    /// `ensure_initialized()`, which is now safe to call multiple times.
    fn release(&self) {
        self.release_gpu();
    }

    fn required_resources(&self) -> Vec<crate::resource::Resource> {
        if self.use_gpu {
            // Multi-GPU is not modeled today — every CUDA install we ship
            // against indexes the primary device as 0. When multi-device
            // support lands, plumb the device index from config to here.
            vec![crate::resource::Resource::LocalGpu { device: 0 }]
        } else {
            vec![crate::resource::Resource::LocalCpu]
        }
    }
}

#[cfg(feature = "local-embed")]
impl Clone for LocalEmbedder {
    fn clone(&self) -> Self {
        Self {
            model: self.model.clone(),
            init_params: self.init_params.clone(),
            dimension: self.dimension,
            model_name: self.model_name.clone(),
            use_gpu: self.use_gpu,
        }
    }
}

/// Best-effort architecture label from a fastembed model code.
///
/// Only used for reporting and for the `Compatible` tier; correctness of the
/// guard rests on the digest, which is exact. An unrecognised code reports
/// itself rather than guessing.
#[cfg(feature = "local-embed")]
fn fastembed_architecture(model_code: &str) -> &str {
    let lower = model_code.to_lowercase();

    // Nomic and MPNet first: they are the exceptions. BGE, MiniLM and GTE are
    // all BERT-architecture encoders, so they deliberately share a label.
    if lower.contains("nomic") {
        "nomic-bert"
    } else if lower.contains("mpnet") {
        "mpnet"
    } else if lower.contains("bge") || lower.contains("minilm") || lower.contains("gte") {
        "bert"
    } else {
        "unknown"
    }
}

// Stub when feature is disabled
#[cfg(not(feature = "local-embed"))]
pub struct LocalEmbedder;

#[cfg(not(feature = "local-embed"))]
impl LocalEmbedder {
    pub fn new() -> Result<Self> {
        anyhow::bail!("Local embeddings not available. Enable 'local-embed' feature or use Ollama.")
    }

    pub fn model_name(&self) -> String {
        "disabled".to_string()
    }
}

#[cfg(not(feature = "local-embed"))]
#[async_trait]
impl Embedder for LocalEmbedder {
    async fn embed(&self, _text: &str) -> Result<Vec<f32>> {
        anyhow::bail!("Local embedder disabled")
    }
    async fn dimension(&self) -> Result<usize> {
        Ok(0)
    }
    fn model_name(&self) -> String {
        "disabled".to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_model_selection_nomic_v15() {
        // nomic-embed-text-v1.5: 137M params, 768-dim, Matryoshka-trained
        let nomic = LocalEmbedder::new("nomic-embed-text-v1.5", None, false).unwrap();
        assert_eq!(nomic.dimension().await.unwrap(), 768);
        assert!(nomic.model_name().contains("nomic-embed-text-v1.5"));

        // Also works with short alias
        let nomic_short = LocalEmbedder::new("nomic-v1.5", None, false).unwrap();
        assert_eq!(nomic_short.dimension().await.unwrap(), 768);
    }

    #[tokio::test]
    async fn test_model_selection_bge() {
        // BGE Small EN v1.5: 384-dim
        let bge = LocalEmbedder::new("bge-small-en-v1.5", None, false).unwrap();
        assert_eq!(bge.dimension().await.unwrap(), 384);
        assert!(bge.model_name().contains("bge-small-en-v1.5"));
    }

    #[tokio::test]
    async fn test_model_selection_default() {
        // Default (empty string) maps to AllMiniLML6V2: 384-dim
        let default = LocalEmbedder::new("", None, false).unwrap();
        assert_eq!(default.dimension().await.unwrap(), 384);
        assert!(default.model_name().contains("all-MiniLM-L6-v2"));
    }

    #[tokio::test]
    async fn test_unknown_model_returns_error() {
        // Unknown model names MUST return an error, not silently fall back.
        // This prevents misconfiguration from producing garbage search results.
        let result = LocalEmbedder::new("nomic-v2-moe", None, false);
        assert!(
            result.is_err(),
            "nomic-v2-moe is not a valid fastembed model and must error"
        );
        let err_msg = match result {
            Err(e) => e.to_string(),
            Ok(_) => panic!("Expected error for nomic-v2-moe"),
        };
        assert!(
            err_msg.contains("Unknown local embedding model"),
            "Error must be descriptive"
        );
        assert!(
            err_msg.contains("nomic-v2-moe"),
            "Error must include the bad model name"
        );

        // Also test a completely random name
        let result2 = LocalEmbedder::new("totally-fake-model", None, false);
        assert!(result2.is_err());
    }

    #[tokio::test]
    async fn test_removed_aliases_error() {
        // bge-micro-v2 was a misleading alias (mapped to bge-small-en-v1.5)
        let result = LocalEmbedder::new("bge-micro-v2", None, false);
        assert!(
            result.is_err(),
            "bge-micro-v2 was a misleading alias and must be removed"
        );
    }

    /// Regression test for the release → reload cycle.
    ///
    /// Before the params-lifecycle fix (E1, 2026-05-01), `ensure_initialized()`
    /// did `params_guard.take()` — the second call (after `release()`) erroneously
    /// reported "Model uninitialized but params missing". This test pins the fixed
    /// behaviour: release() can be called any number of times and embed() must
    /// transparently reload.
    ///
    /// CPU-only on purpose — runs in CI without a GPU.
    #[tokio::test]
    async fn test_release_then_reload() {
        let embedder =
            LocalEmbedder::new("all-minilm-l6-v2", None, false).expect("construct LocalEmbedder");

        // Cycle 1: cold load → embed → release.
        let v1 = embedder.embed("first", None).await.expect("first embed");
        assert_eq!(v1.len(), 384, "AllMiniLM-L6-v2 is 384-dim");
        embedder.release();

        // Cycle 2: this is the previously-broken path. Must reload cleanly.
        let v2 = embedder
            .embed("second", None)
            .await
            .expect("reload after release");
        assert_eq!(v2.len(), 384);

        // Cycle 3: prove release() is idempotent and the params survive repeated cycles.
        embedder.release();
        embedder.release(); // double-release must not error or hang
        let v3 = embedder
            .embed("third", None)
            .await
            .expect("reload after double release");
        assert_eq!(v3.len(), 384);

        // Sanity: same input embeds to same vector across reloads (deterministic model).
        let v1_again = embedder.embed("first", None).await.expect("re-embed first");
        assert_eq!(
            v1, v1_again,
            "model must be deterministic across reload cycles"
        );
    }

    /// Confirms the `Embedder` trait's `release()` default no-op compiles for
    /// types that override neither `release()` nor anything else interesting —
    /// concretely, calling `release()` through a trait object on `LocalEmbedder`
    /// must dispatch to the override and not the default.
    #[tokio::test]
    async fn test_release_via_trait_object() {
        let embedder: Arc<dyn Embedder + Send + Sync> =
            Arc::new(LocalEmbedder::new("all-minilm-l6-v2", None, false).unwrap());

        let _ = embedder.embed("warmup", None).await.expect("initial embed");
        embedder.release(); // dispatched via trait object
        let v = embedder
            .embed("post-release", None)
            .await
            .expect("reload via trait");
        assert_eq!(v.len(), 384);
    }
}
