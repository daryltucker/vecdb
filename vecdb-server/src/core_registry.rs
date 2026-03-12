// Core registry for vecdb-server.
//
// Fixes the "single boot embedder" bug: the server used to create ONE Core at startup
// and route ALL search/ingest requests through that single embedder, regardless of which
// embedder a collection was ingested with. This registry maintains a lazy cache of Core
// instances keyed by their embedder+backend identity, and resolves the correct Core per
// collection on each request.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;

use anyhow::Result;
use tokio::sync::RwLock;

use vecdb_common::FileTypeDetector;
use vecdb_core::config::{Config, PathRule, Profile};
use vecdb_core::parsers::ParserFactory;
use vecdb_core::Core;

/// Identity key for a Core instance.
///
/// Two resolved profiles that share the same key will share the same cached Core.
/// Fields are the subset of Profile that actually affect which embedder and backend
/// are constructed — tuning params like gpu_batch_size and num_ctx are excluded
/// because they don't change the identity of the embedder or Qdrant instance.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct CoreKey {
    pub qdrant_url: String,
    pub qdrant_api_key: Option<String>,
    pub embedder_type: String,
    /// The effective embedding model name (after `config.resolve_embedding_model()` is applied).
    pub embedding_model: String,
    pub ollama_url: String,
    pub ollama_api_key: Option<String>,
    pub accept_invalid_certs: bool,
    pub local_use_gpu: bool,
}

impl CoreKey {
    /// Build a CoreKey from a fully-resolved Profile + global Config.
    /// Uses `config.resolve_embedding_model()` to handle the local/global model name split.
    pub fn from_resolved(profile: &Profile, config: &Config) -> Self {
        let embedding_model = config.resolve_embedding_model(profile);
        let local_use_gpu = config.resolve_local_use_gpu(profile.default_collection_name.as_deref());
        Self {
            qdrant_url: profile.qdrant_url.clone(),
            qdrant_api_key: profile.qdrant_api_key.clone(),
            embedder_type: profile.embedder_type.clone(),
            embedding_model,
            ollama_url: profile.ollama_url.clone(),
            ollama_api_key: profile.ollama_api_key.clone(),
            accept_invalid_certs: profile.accept_invalid_certs,
            local_use_gpu,
        }
    }
}

/// Factory for constructing new Core instances on demand.
/// Holds the global infrastructure that is shared across all Cores.
/// Not present in test registries — those use pre-built mock Cores.
pub struct CoreFactory {
    pub fastembed_cache_path: PathBuf,
    pub smart_routing_keys: Vec<String>,
    pub path_rules: Vec<PathRule>,
    pub max_concurrent_requests: usize,
    pub file_detector: Arc<dyn FileTypeDetector>,
    pub parser_factory: Arc<dyn ParserFactory>,
}

impl CoreFactory {
    async fn create_core(&self, profile: &Profile, config: &Config) -> Result<Arc<Core>> {
        let embedding_model = config.resolve_embedding_model(profile);
        let gpu_batch_size = config.resolve_gpu_batch_size(profile, profile.default_collection_name.as_deref());
        let local_use_gpu = config.resolve_local_use_gpu(profile.default_collection_name.as_deref());
        let core = Core::new(
            &profile.qdrant_url,
            &profile.ollama_url,
            &embedding_model,
            profile.accept_invalid_certs,
            &profile.embedder_type,
            Some(self.fastembed_cache_path.clone()),
            local_use_gpu,
            profile.qdrant_api_key.clone(),
            profile.ollama_api_key.clone(),
            self.smart_routing_keys.clone(),
            self.path_rules.clone(),
            self.max_concurrent_requests,
            gpu_batch_size,
            profile.num_ctx,
            self.file_detector.clone(),
            self.parser_factory.clone(),
        )
        .await?;
        Ok(Arc::new(core))
    }
}

/// Lazy registry of Core instances keyed by embedder+backend identity.
///
/// Thread-safe via RwLock. Used by the MCP server to dispatch each search/ingest
/// request to the Core that matches the target collection's profile.
pub struct CoreRegistry {
    cores: RwLock<HashMap<CoreKey, Arc<Core>>>,
    /// If None, get_for_collection returns an error for uncached keys (test mode).
    factory: Option<CoreFactory>,
    boot_profile_name: String,
}

impl CoreRegistry {
    /// Create a production registry.
    ///
    /// The boot Core (built at server startup) is pre-seeded so the common case
    /// (searching the default collection) hits the fast path immediately.
    pub fn new(
        boot_core: Arc<Core>,
        boot_key: CoreKey,
        boot_profile_name: impl Into<String>,
        factory: CoreFactory,
    ) -> Self {
        let mut map = HashMap::new();
        map.insert(boot_key, boot_core);
        Self {
            cores: RwLock::new(map),
            factory: Some(factory),
            boot_profile_name: boot_profile_name.into(),
        }
    }

    /// Create a test registry from a pre-built map of Cores.
    ///
    /// `get_for_collection` returns an error for any key not present in the map
    /// rather than trying to create new Cores (no factory available).
    pub fn from_map(
        cores: HashMap<CoreKey, Arc<Core>>,
        boot_profile_name: impl Into<String>,
    ) -> Self {
        Self {
            cores: RwLock::new(cores),
            factory: None,
            boot_profile_name: boot_profile_name.into(),
        }
    }

    /// Return the boot Core (the Core initialized at server startup).
    ///
    /// Used for operations that don't have collection context:
    /// - `embed` (no target collection)
    /// - `list_collections` (lists from boot Qdrant instance)
    /// - `get_job_status`
    /// - `delete_collection` (see note below)
    ///
    /// Note: delete_collection and list_collections only reach the boot Qdrant instance.
    /// Collections on remote Qdrant instances require a full BackendRegistry (future work).
    pub async fn boot_core(&self, config: &Config) -> Result<Arc<Core>> {
        let profile = config
            .get_profile(Some(&self.boot_profile_name))
            .or_else(|_| config.get_profile(None))?;
        let key = CoreKey::from_resolved(profile, config);
        let cores = self.cores.read().await;
        cores.get(&key).cloned().ok_or_else(|| {
            anyhow::anyhow!(
                "Boot Core not found in registry (profile: '{}').  \
                 This is a bug — the boot Core should always be present.",
                self.boot_profile_name
            )
        })
    }

    /// Resolve and return the correct Core for a specific collection.
    ///
    /// Algorithm:
    /// 1. Resolve the profile for this collection via `config.resolve_profile()`
    /// 2. Build a `CoreKey` from the resolved profile
    /// 3. Return cached Core if present (fast path — read lock only)
    /// 4. Create a new Core via factory (slow path — Core::new may probe embedder)
    /// 5. Cache under write lock, deferring to an existing entry if we raced
    ///
    /// The slow path is concurrent-safe: two requests racing for the same key will
    /// both create a Core, but only one gets inserted; the other is dropped.
    pub async fn get_for_collection(
        &self,
        config: &Config,
        collection: Option<&str>,
        requested_profile: Option<&str>,
    ) -> Result<Arc<Core>> {
        let profile = config.resolve_profile(requested_profile, collection)?;
        let key = CoreKey::from_resolved(&profile, config);

        // Fast path: read lock
        {
            let cores = self.cores.read().await;
            if let Some(core) = cores.get(&key) {
                return Ok(core.clone());
            }
        }

        // Slow path: create Core *outside* the lock so we don't hold a write lock
        // across the async Core::new() call (which may probe embedder over network).
        let factory = self.factory.as_ref().ok_or_else(|| {
            anyhow::anyhow!(
                "No Core found for profile '{}' (embedder: {}, model: {}, qdrant: {}) \
                 and no factory is available. \
                 This is a test registry — pre-seed it with the required Core.",
                profile.resolved_profile_name,
                profile.embedder_type,
                config.resolve_embedding_model(&profile),
                profile.qdrant_url,
            )
        })?;

        let core = factory.create_core(&profile, config).await?;

        // Insert under write lock, deferring to existing entry if we raced.
        let mut cores = self.cores.write().await;
        Ok(cores.entry(key).or_insert(core).clone())
    }
}
