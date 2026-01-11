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
    model: Arc<Mutex<TextEmbedding>>,
    dimension: usize,
    model_name: String,
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
            unsafe { std::env::set_var("ORT_INTRA_OP_NUM_THREADS", &safe_threads); }
            if std::env::var("VECDB_DEBUG").is_ok() {
                eprintln!("[LocalEmbedder] Auto-limiting ONNX threads to {}", safe_threads);
            }
        }
        
        Self::with_model(EmbeddingModel::AllMiniLML6V2, cache_path, use_gpu)
    }

    /// Create a LocalEmbedder with a specific model.
    pub fn with_model(model_type: EmbeddingModel, cache_path: Option<std::path::PathBuf>, use_gpu: bool) -> Result<Self> {
        // Get model info for dimension
        let model_info = TextEmbedding::get_model_info(&model_type)
            .context("Failed to get model info")?;
        let dimension = model_info.dim;
        let model_name = model_info.model_code.clone();

        // Initialize with the specified model
        let mut options = InitOptions::new(model_type)
            .with_show_download_progress(true);
            
        if let Some(path) = cache_path {
            options = options.with_cache_dir(path);
        }
        
        #[cfg(feature = "cuda")]
        if use_gpu {
            tracing::info!("Initializing local embedder with CUDA acceleration");
            options = options.with_execution_providers(vec![
                CUDAExecutionProvider::default().into()
            ]);
        }
        
        #[cfg(not(feature = "cuda"))]
        if use_gpu {
            tracing::warn!("GPU acceleration requested but 'cuda' feature not enabled during compilation. Falling back to CPU.");
        }
        
        let model = TextEmbedding::try_new(options)
            .context("Failed to initialize local embedding model")?;
        
        Ok(Self {
            model: Arc::new(Mutex::new(model)),
            dimension,
            model_name,
            use_gpu,
        })
    }


}

#[cfg(feature = "local-embed")]
#[async_trait]
impl Embedder for LocalEmbedder {
    async fn embed(&self, text: &str) -> Result<Vec<f32>> {
        let model = self.model.clone();
        let text_owned = text.to_string();
        
        // fastembed is sync and requires &mut self, so wrap in spawn_blocking with Mutex
        let result = tokio::task::spawn_blocking(move || {
            let mut guard = model.lock().map_err(|e| anyhow::anyhow!("Lock poisoned: {}", e))?;
            guard.embed(vec![text_owned], None)
        })
        .await
        .context("Embedding task panicked")?
        .context("Embedding generation failed")?;
        
        result.into_iter().next()
            .ok_or_else(|| anyhow::anyhow!("No embedding returned"))
    }

    async fn embed_batch(&self, texts: &[String]) -> Result<Vec<Vec<f32>>> {
        let model = self.model.clone();
        let texts_owned: Vec<String> = texts.to_vec();
        
        // fastembed handles batching efficiently
        let result = tokio::task::spawn_blocking(move || {
            let mut guard = model.lock().map_err(|e| anyhow::anyhow!("Lock poisoned: {}", e))?;
            guard.embed(texts_owned, None)
        })
        .await
        .context("Embedding batch task panicked")?
        .context("Batch embedding generation failed")?;
        
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
