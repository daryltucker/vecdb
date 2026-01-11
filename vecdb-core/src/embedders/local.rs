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
use std::sync::Arc;
use num_cpus;

#[cfg(feature = "local-embed")]
use fastembed::{TextEmbedding, EmbeddingModel, InitOptions};
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
    /// Create a new LocalEmbedder with the default model (AllMiniLML6V2).
    /// The model is downloaded automatically on first use (~30MB).
    pub fn new(cache_path: Option<std::path::PathBuf>, use_gpu: bool) -> Result<Self> {
        // Starvation Protection: Limit ONNX Runtime threads
        // Unless explicitly overridden by user, cap intra-op threads to a safe number (e.g., 4)
        // or 50% of logical cores, to prevent "System Lockup" during ingestion.
        if std::env::var("ORT_INTRA_OP_NUM_THREADS").is_err() {
            let num_cpus = num_cpus::get();
            let safe_threads = (num_cpus / 2).clamp(1, 4).to_string(); // Cap at 4 for stability
            // ORT (ONNX Runtime)
            unsafe { std::env::set_var("ORT_INTRA_OP_NUM_THREADS", &safe_threads); }
             // OpenMP (Torch/Many libs)
            unsafe { std::env::set_var("OMP_NUM_THREADS", &safe_threads); }
            // MKL (Math Kernel Library)
            unsafe { std::env::set_var("MKL_NUM_THREADS", &safe_threads); }
            
            if std::env::var("VECDB_DEBUG").is_ok() {
                eprintln!("[LocalEmbedder] Auto-limited math threads to {}", safe_threads);
            }
        }

        // Create the struct with lazy intent
        let instance = Self::with_model(EmbeddingModel::AllMiniLML6V2, cache_path, use_gpu)?;

        // CRITICAL: If GPU is requested, we MUST initialize EAGERLY.
        // This ensures that if CUDA fails and we fall back to CPU, the 
        // warning messages are printed to stderr *HERE*, before any CLI 
        // progress bars (like in `ingest`) are started.
        // If we wait for lazy init, the progress bar will swallow/overwrite the warnings.
        if use_gpu {
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
    pub fn with_model(model_type: EmbeddingModel, cache_path: Option<std::path::PathBuf>, use_gpu: bool) -> Result<Self> {
        // Get model info for dimension - this is lightweight
        let model_info = TextEmbedding::get_model_info(&model_type)
            .context("Failed to get model info")?;
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

        // Eager init if GPU requested (consistency with ::new)
        if use_gpu {
           tracing::debug!("Eagerly initializing LocalEmbedder (custom model) for CUDA check...");
           match instance.ensure_initialized() {
                Ok(_) => {},
                Err(e) => return Err(e).context("Failed eager initialization")
           }
        }

        Ok(instance)
    }
    
    /// Internal helper to initialize the model on first use
    fn ensure_initialized(&self) -> Result<()> {
        let mut model_guard = self.model.lock().map_err(|e| anyhow::anyhow!("Lock poisoned: {}", e))?;
        
        if model_guard.is_some() {
            return Ok(());
        }
        
        // Need to initialize
        tracing::debug!("Lazy initializing LocalEmbedder...");
        
        let mut params_guard = self.init_params.lock().map_err(|e| anyhow::anyhow!("Lock poisoned: {}", e))?;
        let params = params_guard.take().ok_or_else(|| anyhow::anyhow!("Model uninitialized but params missing"))?;
        

        // Helper to construct base options
        let make_options = || {
            let mut options = InitOptions::new(params.model_type.clone())
                .with_show_download_progress(true);
            if let Some(path) = &params.cache_path {
                options = options.with_cache_dir(path.clone());
            }
            options
        };

        let model = {
            #[cfg(feature = "cuda")]
            if params.use_gpu {
                tracing::debug!("Initializing local embedder with CUDA acceleration");
                // We assume check_cuda_availability() ran earlier in new() so user was warned.
                // We proceed to TRY to init CUDA. 
                
                // SAFETY CHECK REMOVED: Native `ort` handles discovery.
                // We rely on Try Init below.

                let mut gpu_options = make_options();
                gpu_options = gpu_options.with_execution_providers(vec![
                    CUDAExecutionProvider::default().into()
                ]);

                tracing::debug!("Attempting to create TextEmbedding with CUDA provider...");
                match TextEmbedding::try_new(gpu_options) {
                    Ok(m) => {
                        eprintln!("✅ [CUDA] Local Embedder initialized successfully (GPU Accelerated).");
                        m
                    },
                    Err(e) => {
                        let msg = format!("Failed to register CUDA provider: {}. Falling back to CPU.", e);
                        tracing::warn!("{}", msg);
                        eprintln!("\n❌ [CUDA FAILURE] Initialization failed: {}", e);
                        eprintln!("   Falling back to CPU. (Expect High RAM usage)");
                        eprintln!("   Troubleshooting:");
                        eprintln!("     1. drivers: nvidia-smi (should be v550+)");
                        eprintln!("     2. libs: ensure libcudnn and libcublas are in LD_LIBRARY_PATH or system paths");
                        eprintln!("   Continuing with CPU-only mode...\n");
                        std::thread::sleep(std::time::Duration::from_secs(3));
                            TextEmbedding::try_new(make_options())
                            .context("Failed to initialize local embedding model (CPU Fallback)")?
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
                  std::thread::sleep(std::time::Duration::from_secs(3));
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

#[cfg(feature = "local-embed")]
#[async_trait]
impl Embedder for LocalEmbedder {
    async fn embed(&self, text: &str) -> Result<Vec<f32>> {

        
        // We need to re-create the lightweight struct to call ensure_initialized on the CLONE
        // or just implement a standalone initialization helper.
        // Easiest is to move the logic into a spawn_blocking closure but we need to init first.
        
        // Strategy: Init in the current thread (lightweight check), then spawn blocking for embed
        // But initializing might block, so we should spawn blocking for init too if needed.
        
        // Better: Pass the whole struct into spawn_blocking, assuming it's Sync.
        // But we can't move 'self'.
        
        let myself = self.clone();
        
        let text_owned = text.to_string();
        
        // fastembed is sync and requires &mut self, so wrap in spawn_blocking with Mutex
        let result = tokio::task::spawn_blocking(move || {
            // Lazy Init
            myself.ensure_initialized()?;
            
            let mut guard = myself.model.lock().map_err(|e| anyhow::anyhow!("Lock poisoned: {}", e))?;
            let model = guard.as_mut().ok_or_else(|| anyhow::anyhow!("Model not initialized"))?;
            model.embed(vec![text_owned], None)
        })
        .await
        .context("Embedding task panicked")??; // Flatten Result<Result<...>>
        
        result.into_iter().next()
            .ok_or_else(|| anyhow::anyhow!("No embedding returned"))
    }

    async fn embed_batch(&self, texts: &[String]) -> Result<Vec<Vec<f32>>> {
        let myself = self.clone();
        let texts_owned: Vec<String> = texts.to_vec();
        
        // fastembed handles batching efficiently
        let result = tokio::task::spawn_blocking(move || {
            // Lazy Init
            myself.ensure_initialized()?;
            
            let mut guard = myself.model.lock().map_err(|e| anyhow::anyhow!("Lock poisoned: {}", e))?;
            let model = guard.as_mut().ok_or_else(|| anyhow::anyhow!("Model not initialized"))?;
            model.embed(texts_owned, None)
        })
        .await
        .context("Embedding batch task panicked")??;
        
        Ok(result)
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
