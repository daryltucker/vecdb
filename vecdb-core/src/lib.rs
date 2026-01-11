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
pub mod config;
pub mod types;
pub mod embedder;
pub mod embedders;
pub mod ingestion;
pub mod router;
pub mod tools;
pub mod git;
pub mod history;
pub mod state;
pub mod parsers;
pub mod chunking;

// Re-export output from vecdb-common for backwards compatibility
pub use vecdb_common::output;

use anyhow::Result;
use backend::Backend;
use backends::qdrant::QdrantBackend;
use embedder::Embedder;
use embedders::OllamaEmbedder;
use types::SearchResult;
use std::sync::Arc;
use ingestion::IngestionOptions;
use router::DynamicRouter;
use vecdb_common::FileTypeDetector;
use parsers::ParserFactory;
// use serde_json::json;

/// The main entry point for the Vector Database logic.
/// Wraps a concrete Backend implementation and Embedder.
pub struct Core {
    backend: Arc<dyn Backend + Send + Sync>,
    embedder: Arc<dyn Embedder + Send + Sync>,
    file_detector: Arc<dyn FileTypeDetector>,
    parser_factory: Arc<dyn ParserFactory>,
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
        // Dependency Injection
        file_detector: Arc<dyn FileTypeDetector>,
        parser_factory: Arc<dyn ParserFactory>,
    ) -> Result<Self> {
        let backend = QdrantBackend::new(qdrant_url, qdrant_api_key)?;
        
        let embedder: Arc<dyn Embedder + Send + Sync> = match embedder_type {
            #[cfg(feature = "local-embed")]
            "local" => {
                if output::OUTPUT.is_interactive {
                    eprintln!("Using local embedder (fastembed) [GPU: {}]", use_gpu);
                }
                Arc::new(embedders::LocalEmbedder::new(fastembed_cache_path, use_gpu)?)
            }
            "ollama" => {
                if output::OUTPUT.is_interactive {
                    eprintln!("Using Ollama embedder at {} with model {}", ollama_url, model);
                }
                Arc::new(OllamaEmbedder::new(ollama_url.to_string(), model.to_string(), accept_invalid_certs, ollama_api_key))
            }
            #[cfg(not(feature = "local-embed"))]
            "local" => {
                anyhow::bail!("Local embedder not available. Compile with 'local-embed' feature or use embedder_type = 'ollama'")
            }
            _ => {
                if output::OUTPUT.is_interactive {
                    eprintln!("Unknown embedder type '{}', falling back to Ollama", embedder_type);
                }
                Arc::new(OllamaEmbedder::new(ollama_url.to_string(), model.to_string(), accept_invalid_certs, ollama_api_key))
            }
        };
        
        Ok(Self {
            backend: Arc::new(backend),
            embedder,
            file_detector,
            parser_factory,
        })
    }

    /// Create a new Core instance from existing backends
    pub fn with_backends(
        backend: Arc<dyn Backend + Send + Sync>,
        embedder: Arc<dyn Embedder + Send + Sync>,
        file_detector: Arc<dyn FileTypeDetector>,
        parser_factory: Arc<dyn ParserFactory>,
    ) -> Self {
        Self { 
            backend, 
            embedder,
            file_detector,
            parser_factory,
        }
    }

    /// Passthrough to Backend::search with automatic embedding
    pub async fn search(&self, collection: &str, query: &str, limit: u64, filter: Option<serde_json::Value>) -> Result<Vec<SearchResult>> {
        // Embed the query
        let vector = self.embedder.embed(query).await?;
        
        // Search
        self.backend.search(collection, &vector, limit, filter).await
    }
    
    /// Ingest a file or directory
    #[allow(clippy::too_many_arguments)]
    pub async fn ingest(
        &self, 
        path: &str, 
        collection: &str, 
        respect_gitignore: bool, 
        chunk_size: Option<usize>,
        max_chunk_size: Option<usize>, 
        chunk_overlap: Option<usize>,
        extensions: Option<Vec<String>>,
        excludes: Option<Vec<String>>,
        dry_run: bool,
        metadata: Option<std::collections::HashMap<String, serde_json::Value>>,
    ) -> Result<()> {
        let options = IngestionOptions {
            path: path.to_string(),
            collection: collection.to_string(),
            chunk_size: chunk_size.unwrap_or(config::DEFAULT_CHUNK_SIZE),
            max_chunk_size,
            chunk_overlap: chunk_overlap.unwrap_or(50),
            respect_gitignore,
            strategy: "recursive".to_string(),
            tokenizer: "cl100k_base".to_string(),
            git_ref: None,
            extensions,
            excludes,
            dry_run,
            metadata,
        };
        
        ingestion::ingest_path(&self.backend, &self.embedder, &self.file_detector, &self.parser_factory, options).await
    }
    
    /// Smart search that routes to specific collections or applies filters based on metadata facets
    pub async fn search_smart(&self, query: &str, limit: u64) -> Result<Vec<SearchResult>> {
        // Default collection - unified strategy uses "docs" or a single project collection
        let collection = "docs"; 
        
        // Use DynamicRouter to detect version/theme facets
        // NOW monitoring multiple keys: version, language, cuda, platform
        let monitored_keys = vec![
            "version".to_string(),
            "language".to_string(),
            "cuda".to_string(),
            "platform".to_string()
        ];
        let router = DynamicRouter::new(self.backend.clone(), monitored_keys);
        
        let (detected_filters, clean_query) = router.route(collection, query).await?;
        
        let filter = if !detected_filters.is_empty() {
            Some(serde_json::Value::Object(detected_filters))
        } else {
            None
        };
        
        if let Some(f) = &filter {
            if output::OUTPUT.is_interactive {
                eprintln!("Smart Route: Applying filter {:?} to query '{}'", f, clean_query);
            }
        }

        self.search(collection, &clean_query, limit, filter).await
    }

    /// Ingest raw content directly (Push Interface)
    pub async fn ingest_content(
        &self, 
        content: &str, 
        metadata: std::collections::HashMap<String, serde_json::Value>,
        collection: &str,
        chunk_size: Option<usize>,
        max_chunk_size: Option<usize>,
        chunk_overlap: Option<usize>,
    ) -> Result<()> {
        ingestion::ingest_memory(&self.backend, &self.embedder, content, metadata, collection, chunk_size, max_chunk_size, chunk_overlap).await
    }

    /// Generate embeddings for a list of texts (Tool Access)
    pub async fn embed(&self, texts: Vec<String>) -> Result<Vec<Vec<f32>>> {
        self.embedder.embed_batch(&texts).await
    }

    /// Ingest a historic version of a repository (Time Travel)
    pub async fn ingest_history(&self, path: &str, git_ref: &str, collection: &str, chunk_size: usize) -> Result<()> {
        crate::history::ingest_history(&self.backend, &self.embedder, &self.file_detector, &self.parser_factory, path, git_ref, collection, chunk_size).await
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
                    });
                }
            }
        }
        
        Ok(infos)
    }

    /// Delete a collection
    pub async fn delete_collection(&self, collection: &str) -> Result<()> {
        self.backend.delete_collection(collection).await
    }

    /// Get the dimension of the configured embedding model
    pub async fn get_embedding_dimension(&self) -> Result<usize> {
        self.embedder.dimension().await
    }

    // Removed misplaces doc comment
    // code_query removed from Core - use vecq directly in CLI/Server
}

// Optional: Facade re-exports if we want a flat namespace
// pub use backend::Backend;
// pub use types::{Document, Chunk, SearchResult};
