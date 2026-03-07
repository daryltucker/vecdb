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
#[cfg(feature = "local-embed")]
pub struct LocalEmbedder {
    // Lazy: Model is None until first use.
    model: Arc<Mutex<Option<TextEmbedding>>>,
    // Store init params for lazy loading
    init_params: Arc<Mutex<Option<LocalEmbedderInitParams>>>,
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
    pub fn new(model_name: &str, cache_path: Option<std::path::PathBuf>, use_gpu: bool) -> Result<Self> {
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
            "bge-small-en-v1.5" | "bge-small-en" | "baai/bge-small-en-v1.5" => EmbeddingModel::BGESmallENV15,
            // BGE Base EN v1.5: 109M params, 768-dim, 512 tok context
            "bge-base-en-v1.5" | "bge-base-en" | "baai/bge-base-en-v1.5" => EmbeddingModel::BGEBaseENV15,
            // BGE Large EN v1.5: 335M params, 1024-dim, 512 tok context
            "bge-large-en-v1.5" | "bge-large-en" | "baai/bge-large-en-v1.5" => EmbeddingModel::BGELargeENV15,
            // Nomic Embed Text v1: 137M params, 768-dim, 8192 tok context
            "nomic-embed-text-v1" | "nomic-embed-text" | "nomic-v1" => EmbeddingModel::NomicEmbedTextV1,
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

        // Store params for lazy init
        let init_params = LocalEmbedderInitParams {
            model_type: model_type.clone(),
            cache_path: cache_path.clone(),
            use_gpu,
        };

        let instance = Self {
            model: Arc::new(Mutex::new(None)), // Uninitialized
            init_params: Arc::new(Mutex::new(Some(init_params))),
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

        // Need to initialize
        tracing::debug!("Lazy initializing LocalEmbedder...");

        let mut params_guard = self
            .init_params
            .lock()
            .map_err(|e| anyhow::anyhow!("Lock poisoned: {}", e))?;
        let params = params_guard
            .take()
            .ok_or_else(|| anyhow::anyhow!("Model uninitialized but params missing"))?;

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

                let mut gpu_options = make_options();
                gpu_options = gpu_options
                    .with_execution_providers(vec![CUDAExecutionProvider::default().into()]);

                tracing::debug!("Attempting to create TextEmbedding with CUDA provider...");
                match TextEmbedding::try_new(gpu_options) {
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
                        eprintln!("\n❌ [CUDA FAILURE] Initialization failed: {}", e);
                        eprintln!("   Strict GPU mode: Aborting because 'local_use_gpu = true' was explicitly requested.");
                        eprintln!("   Troubleshooting:");
                        eprintln!("     1. drivers: nvidia-smi (should be v550+)");
                        eprintln!("     2. libs: ensure libonnxruntime_providers_cuda.so and libcudnn are in LD_LIBRARY_PATH");
                        eprintln!("        Try: 'export LD_LIBRARY_PATH=/usr/local/lib:$LD_LIBRARY_PATH'");
                        eprintln!("   Check docs/GPU.md for the full setup guide.\n");
                        
                        return Err(e).context("Local embedder failed to initialize with GPU (Strict mode)");
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
            model.embed(vec![text_owned], None)
                .map_err(wrap_cuda_error)
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

    async fn embed_batch(&self, texts: &[String], target_dim: Option<usize>) -> Result<Vec<Vec<f32>>> {
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
            model.embed(texts_owned, None)
                .map_err(wrap_cuda_error)
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
        assert!(result.is_err(), "nomic-v2-moe is not a valid fastembed model and must error");
        let err_msg = match result {
            Err(e) => e.to_string(),
            Ok(_) => panic!("Expected error for nomic-v2-moe"),
        };
        assert!(err_msg.contains("Unknown local embedding model"), "Error must be descriptive");
        assert!(err_msg.contains("nomic-v2-moe"), "Error must include the bad model name");

        // Also test a completely random name
        let result2 = LocalEmbedder::new("totally-fake-model", None, false);
        assert!(result2.is_err());
    }

    #[tokio::test]
    async fn test_removed_aliases_error() {
        // bge-micro-v2 was a misleading alias (mapped to bge-small-en-v1.5)
        let result = LocalEmbedder::new("bge-micro-v2", None, false);
        assert!(result.is_err(), "bge-micro-v2 was a misleading alias and must be removed");
    }
}
