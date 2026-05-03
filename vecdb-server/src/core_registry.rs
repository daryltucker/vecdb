// Core registry for vecdb-server.
//
// Fixes the "single boot embedder" bug: the server used to create ONE Core at startup
// and route ALL search/ingest requests through that single embedder, regardless of which
// embedder a collection was ingested with. This registry maintains a lazy cache of Core
// instances keyed by their embedder+backend identity, and resolves the correct Core per
// collection on each request.
//
// Lifecycle (added 2026-05-01):
// Each cached entry tracks `last_used`. A watchdog task spawned via `start_watchdog()`
// implements hybrid idle eviction:
//   * After `soft_idle_secs` of inactivity → call `Embedder::release()` to drop the
//     loaded model (frees the bulk of VRAM; reload on next use is ~200 ms).
//   * After `deep_idle_secs` of inactivity → drop the cache entry entirely. In stdio
//     mode the watchdog also fires the shutdown channel, ending the subprocess so the
//     ~80 MiB process-global CUDA context is reclaimed by the OS. The MCP client
//     respawns it on next use.
//
// Rationale documented in docs/planning/BUG_IDLE_VRAM_AND_RESOURCE_ISOLATION.md (E1).

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

use anyhow::Result;
use tokio::sync::{watch, RwLock};

use vecdb_common::FileTypeDetector;
use vecdb_core::config::{Config, PathRule, Profile, ServerConfig};
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

/// One cached Core plus its idle bookkeeping.
///
/// `last_used` is epoch-millis. Bumped on every `get_for_collection`/`boot_core`
/// resolution. The watchdog reads it under the registry's `RwLock::read()` and
/// makes eviction decisions without blocking new requests.
pub struct CoreEntry {
    core: Arc<Core>,
    last_used: AtomicU64,
}

impl CoreEntry {
    fn new(core: Arc<Core>) -> Self {
        Self { core, last_used: AtomicU64::new(now_ms()) }
    }
    fn touch(&self) {
        self.last_used.store(now_ms(), Ordering::Relaxed);
    }
    pub fn core(&self) -> &Arc<Core> { &self.core }
    pub fn last_used_ms(&self) -> u64 { self.last_used.load(Ordering::Relaxed) }
}

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
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
    cores: RwLock<HashMap<CoreKey, Arc<CoreEntry>>>,
    /// If None, get_for_collection returns an error for uncached keys (test mode).
    factory: Option<CoreFactory>,
    boot_profile_name: String,
    boot_key: CoreKey,
    /// Store config for get_core_for_profile to use
    config: Config,
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
        config: Config,
    ) -> Self {
        let mut map = HashMap::new();
        map.insert(boot_key.clone(), Arc::new(CoreEntry::new(boot_core)));
        Self {
            cores: RwLock::new(map),
            factory: Some(factory),
            boot_profile_name: boot_profile_name.into(),
            boot_key,
            config,
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
        // Synthesise a placeholder boot_key from the first entry, or a default one
        // if the map is empty. Test code that cares about the boot path should
        // populate at least one entry.
        let boot_key = cores.keys().next().cloned().unwrap_or(CoreKey {
            qdrant_url: String::new(),
            qdrant_api_key: None,
            embedder_type: String::new(),
            embedding_model: String::new(),
            ollama_url: String::new(),
            ollama_api_key: None,
            accept_invalid_certs: false,
            local_use_gpu: false,
        });
        let wrapped = cores
            .into_iter()
            .map(|(k, c)| (k, Arc::new(CoreEntry::new(c))))
            .collect();
        Self {
            cores: RwLock::new(wrapped),
            factory: None,
            boot_profile_name: boot_profile_name.into(),
            boot_key,
            config: Config::default(),
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
        let entry = cores.get(&key).cloned().ok_or_else(|| {
            anyhow::anyhow!(
                "Boot Core not found in registry (profile: '{}').  \
                 This is a bug — the boot Core should always be present.",
                self.boot_profile_name
            )
        })?;
        entry.touch();
        Ok(entry.core.clone())
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
            if let Some(entry) = cores.get(&key) {
                entry.touch();
                return Ok(entry.core.clone());
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
        let entry = cores
            .entry(key)
            .or_insert_with(|| Arc::new(CoreEntry::new(core)))
            .clone();
        entry.touch();
        Ok(entry.core.clone())
    }

    /// Snapshot of (key, last_used_ms) for all cached entries. Used by the watchdog.
    pub async fn snapshot_idle(&self) -> Vec<(CoreKey, u64, Arc<CoreEntry>)> {
        self.cores
            .read()
            .await
            .iter()
            .map(|(k, e)| (k.clone(), e.last_used_ms(), e.clone()))
            .collect()
    }

    /// Drop a cache entry by key. Returns true if something was removed.
    /// Boot key is never removed — it would re-create on demand and the dance
    /// is pointless; soft-release on the boot embedder is sufficient.
    pub async fn evict(&self, key: &CoreKey) -> bool {
        if key == &self.boot_key {
            return false;
        }
        self.cores.write().await.remove(key).is_some()
    }

    /// Get a Core for a specific Profile.
    ///
    /// This is used by list_collections to query all backends - it creates/retrieves
    /// a Core for each profile's Qdrant URL without needing collection context.
    pub async fn get_core_for_profile(&self, profile: &Profile) -> Result<Arc<Core>> {
        let key = CoreKey::from_resolved(profile, &self.config);

        // Fast path: read lock
        {
            let cores = self.cores.read().await;
            if let Some(entry) = cores.get(&key) {
                entry.touch();
                return Ok(entry.core.clone());
            }
        }

        // Slow path: create Core
        let factory = self.factory.as_ref().ok_or_else(|| {
            anyhow::anyhow!("No factory available (test registry)")
        })?;

        let core = factory.create_core(profile, &self.config).await?;

        let mut cores = self.cores.write().await;
        let entry = cores
            .entry(key)
            .or_insert_with(|| Arc::new(CoreEntry::new(core)))
            .clone();
        entry.touch();
        Ok(entry.core.clone())
    }
}

/// Spawn the idle-eviction watchdog.
///
/// Returns a `watch::Receiver<bool>` that fires `true` when deep-idle eviction
/// determines the process should shut down. The stdio main loop selects on this
/// to exit cleanly. HTTP/daemon callers can ignore it.
///
/// `eviction_mode` controls deep-idle behaviour:
///   * `EvictionMode::ExitOnDeepIdle` — stdio mode; flips the shutdown channel.
///   * `EvictionMode::CacheOnly` — daemon mode; just drops the cache entry.
pub fn start_watchdog(
    registry: Arc<CoreRegistry>,
    cfg: ServerConfig,
    eviction_mode: EvictionMode,
) -> watch::Receiver<bool> {
    let (shutdown_tx, shutdown_rx) = watch::channel(false);

    if !cfg.idle_eviction_enabled
        || (cfg.soft_idle_secs == 0 && cfg.deep_idle_secs == 0)
    {
        tracing::debug!("Idle-eviction watchdog disabled by config");
        return shutdown_rx;
    }

    let interval = std::time::Duration::from_secs(cfg.idle_check_interval_secs.max(5));
    let soft_ms = cfg.soft_idle_secs.saturating_mul(1000);
    let deep_ms = cfg.deep_idle_secs.saturating_mul(1000);

    tokio::spawn(async move {
        let mut ticker = tokio::time::interval(interval);
        // Skip the immediate first tick — nothing has been idle yet.
        ticker.tick().await;

        loop {
            ticker.tick().await;
            let now = now_ms();
            let entries = registry.snapshot_idle().await;

            for (key, last, entry) in entries {
                let idle_ms = now.saturating_sub(last);

                // Deep idle takes priority over soft.
                if deep_ms > 0 && idle_ms >= deep_ms {
                    // Release first (cheap, idempotent), then drop the cache slot.
                    entry.core().embedder().release();
                    let removed = registry.evict(&key).await;
                    if removed {
                        tracing::info!(
                            "Deep-idle eviction: dropped Core entry (idle {}s, model {})",
                            idle_ms / 1000,
                            entry.core().embedder().model_name(),
                        );
                    }
                    if matches!(eviction_mode, EvictionMode::ExitOnDeepIdle) {
                        // Signal main loop. We don't exit the watchdog itself —
                        // the main loop will tear down the runtime.
                        let _ = shutdown_tx.send(true);
                    }
                } else if soft_ms > 0 && idle_ms >= soft_ms {
                    // Soft eviction: release the model in place. release() is idempotent
                    // so calling it on every tick after threshold is harmless and avoids
                    // tracking yet another flag.
                    entry.core().embedder().release();
                    tracing::debug!(
                        "Soft-idle release: dropped model weights (idle {}s, model {})",
                        idle_ms / 1000,
                        entry.core().embedder().model_name(),
                    );
                }
            }
        }
    });

    shutdown_rx
}

/// Behaviour after deep-idle threshold is crossed.
#[derive(Debug, Clone, Copy)]
pub enum EvictionMode {
    /// Stdio subprocess: signal shutdown so the OS reclaims the CUDA context too.
    ExitOnDeepIdle,
    /// HTTP/daemon: just drop the cache entry; process keeps running.
    CacheOnly,
}
