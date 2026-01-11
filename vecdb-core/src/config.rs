//! DOCS: docs/CONFIG.md
//! COMPLIANCE: tests/tier2_config_compliance.py
/*
 * PURPOSE:
 *   Manages application configuration and profiles.
 *   Allows users to define connection details and behavior in a persistent file.
 *
 * REQUIREMENTS:
 *   User-specified:
 *   - "Config file with profiles" (User Prompt)
 *   - "Default profile" support
 *   - Law #1: Config IS Code
 *   - Ingestion Control (Chunk Size, Strategy)
 *
 *   Implementation-discovered:
 *   - Needs serialization (Serde)
 *   - Needs XDG compliance (~/.config/vecdb/config.toml)
 *
 * IMPLEMENTATION RULES:
 *   1. Use `toml` for storage
 *      Rationale: Human-readable, standard in Rust ecosystem.
 *
 *   2. Use `dirs` crate for path resolution
 *      Rationale: Cross-platform compatibility (Linux/macOS/Windows).
 *
 * USAGE:
 *   let config = Config::load()?;
 *   let profile = config.get_profile("default")?;
 *
 * SELF-HEALING INSTRUCTIONS:
 *   - If `config.toml` is missing: return Default config (don't crash).
 *   - If profile missing: Error gently.
 *
 * RELATED FILES:
 *   - vecdb-cli/src/main.rs - Consumes this config
 *
 * MAINTENANCE:
 *   Update when new backend options are needed (e.g., API keys).
 */

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::PathBuf;
use std::fs;

const DEFAULT_PROFILE_NAME: &str = "default";
const DEFAULT_QDRANT_URL: &str = "http://localhost:6334";
const DEFAULT_OLLAMA_URL: &str = "http://localhost:11434";
const DEFAULT_EMBEDDING_MODEL: &str = "nomic-embed-text";
pub const DEFAULT_CHUNK_SIZE: usize = 512;
const DEFAULT_CHUNK_OVERLAP: usize = 50;
const DEFAULT_STRATEGY: &str = "recursive";

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Config {
    pub profiles: HashMap<String, Profile>,
    #[serde(default = "default_profile_name")]
    pub default_profile: String,
    
    /// Global: Local embedding model (shared across all profiles with embedder_type="local")
    /// This enforces the single-local-embedder constraint
    #[serde(default = "default_local_embedding_model")]
    pub local_embedding_model: String,
    
    /// Collection Profiles: Detailed configuration per collection
    #[serde(default)]
    pub collections: HashMap<String, CollectionConfig>,
    
    /// Simple aliases: short_name -> collection_profile_name
    #[serde(default)]
    pub collection_aliases: HashMap<String, String>,
    
    #[serde(default)]
    pub ingestion: IngestionConfig,
    
    /// Global: Location for fastembed model cache
    #[serde(default = "default_fastembed_cache_path")]
    pub fastembed_cache_path: PathBuf,

    /// Global: Use GPU for local embeddings if available
    #[serde(default)]
    pub local_use_gpu: bool,

    /// Keys to use for Smart Routing (Facet Auto-Detection).
    /// Default: ["language", "source_type"]
    /// Add "platform", "version", "cuda" here to enable them.
    #[serde(default = "default_smart_routing_keys")]
    pub smart_routing_keys: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CollectionConfig {
    /// The actual Qdrant collection name
    pub name: String,
    /// Description for listing
    pub description: Option<String>,
    
    /// Override: Embedder type
    pub embedder_type: Option<String>,
    /// Override: Model name
    pub embedding_model: Option<String>,
    /// Override: Ollama URL
    pub ollama_url: Option<String>,
    /// Override: target chunk size
    pub chunk_size: Option<usize>,
    /// Override: chunk overlap
    pub chunk_overlap: Option<usize>,
    /// Override: max chunk size (hard limit)
    pub max_chunk_size: Option<usize>,
    /// Override: Use GPU for local embeddings
    pub use_gpu: Option<bool>,
    /// Override: Qdrant API Key
    pub qdrant_api_key: Option<String>,
    /// Override: Ollama API Key (if using authenticated proxy)
    pub ollama_api_key: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IngestionConfig {
    #[serde(default = "default_strategy")]
    pub default_strategy: String,
    #[serde(default = "default_chunk_size")]
    pub chunk_size: usize,
    /// Hard limit for acceptable chunk size
    #[serde(default)]
    pub max_chunk_size: Option<usize>,
    #[serde(default = "default_chunk_overlap")]
    pub chunk_overlap: usize,
    #[serde(default)]
    pub respect_gitignore: bool,
    #[serde(default)]
    pub tokenizer: String,
    // Wildcard -> Config
    #[serde(default)]
    pub overrides: HashMap<String, IngestionOverride>,
    
    /// Path parsing rules for metadata extraction
    /// Path parsing rules for metadata extraction
    #[serde(default)]
    pub path_rules: Vec<PathRule>,
    
    /// Concurrency Limit: Max number of file processing tasks running in parallel
    #[serde(default = "default_concurrency")]
    pub max_concurrent_requests: usize,

    /// GPU Concurrency: Batch size for GPU embedding (prevents OOM)
    #[serde(default = "default_gpu_batch_size")]
    pub gpu_batch_size: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PathRule {
    /// Regex pattern with named capture groups (e.g. "users/(?P<user>\w+)/.*")
    pub pattern: String,
}

impl Default for IngestionConfig {
    fn default() -> Self {
        Self {
            default_strategy: default_strategy(),
            chunk_size: default_chunk_size(),
            max_chunk_size: None,
            chunk_overlap: default_chunk_overlap(),
            respect_gitignore: false,
            tokenizer: default_tokenizer(),
            overrides: HashMap::new(),
            path_rules: Vec::new(),
            max_concurrent_requests: default_concurrency(),
            gpu_batch_size: default_gpu_batch_size(),
        }
    }
}

fn default_gpu_batch_size() -> usize {
    2
}

fn default_concurrency() -> usize {
    4
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IngestionOverride {
    pub strategy: Option<String>,
    pub chunk_size: Option<usize>,
    pub max_chunk_size: Option<usize>,
    pub chunk_overlap: Option<usize>,
}

fn default_profile_name() -> String {
    DEFAULT_PROFILE_NAME.to_string()
}

fn default_strategy() -> String {
    DEFAULT_STRATEGY.to_string()
}

fn default_chunk_size() -> usize {
    DEFAULT_CHUNK_SIZE
}

fn default_chunk_overlap() -> usize {
    DEFAULT_CHUNK_OVERLAP
}

fn default_tokenizer() -> String {
    "cl100k_base".to_string()
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Profile {
    pub qdrant_url: String,
    /// Default collection to use if none specified
    // No default here - forced requirement to prevent ambiguity
    pub default_collection_name: String,
    #[serde(default = "default_embedding_model")]
    pub embedding_model: String,
    /// Accept invalid TLS certificates (for staging/self-signed HTTPS endpoints)
    #[serde(default)]
    pub accept_invalid_certs: bool,
    #[serde(default = "default_ollama_url")]
    pub ollama_url: String,
    /// Embedder type: "local" (fastembed, no deps) or "ollama" (requires Ollama service)
    /// Default: "local" for zero-config experience
    #[serde(default = "default_embedder_type")]
    pub embedder_type: String,
    
    // Credentials
    pub qdrant_api_key: Option<String>,
    pub ollama_api_key: Option<String>,
}

fn default_embedder_type() -> String {
    "local".to_string()
}

fn default_ollama_url() -> String {
    DEFAULT_OLLAMA_URL.to_string()
}

fn default_embedding_model() -> String {
    DEFAULT_EMBEDDING_MODEL.to_string()
}

fn default_local_embedding_model() -> String {
    "bge-micro-v2".to_string()
}

fn default_fastembed_cache_path() -> PathBuf {
    let mut path = dirs::config_dir().unwrap_or_else(|| PathBuf::from("."));
    path.push("vecdb");
    path.push("fastembed_cache");
    path
}

fn default_smart_routing_keys() -> Vec<String> {
    vec![
        "source_type".to_string(),
        "language".to_string(),
        // Users can add "version", "cuda", "platform" in config.toml
    ]
}

impl Default for Config {
    fn default() -> Self {
        let mut profiles = HashMap::new();
        profiles.insert(
            DEFAULT_PROFILE_NAME.to_string(),
            Profile {
                qdrant_url: std::env::var("QDRANT_URL").unwrap_or_else(|_| DEFAULT_QDRANT_URL.to_string()),
                embedding_model: DEFAULT_EMBEDDING_MODEL.to_string(),
                default_collection_name: "docs".to_string(),
                accept_invalid_certs: false,
                ollama_url: DEFAULT_OLLAMA_URL.to_string(),
                embedder_type: "local".to_string(),
                qdrant_api_key: None,
                ollama_api_key: None,
            },
        );
        Self {
            profiles,
            default_profile: DEFAULT_PROFILE_NAME.to_string(),
            local_embedding_model: "bge-micro-v2".to_string(),
            collections: HashMap::new(),
            collection_aliases: HashMap::new(),
            ingestion: IngestionConfig::default(),
            fastembed_cache_path: default_fastembed_cache_path(),
            local_use_gpu: false,
            smart_routing_keys: default_smart_routing_keys(),
        }
    }
}

impl Config {
    /// Helper to resolve the embedding model name based on the profile's configuration
    /// This unifies the logic: "if local, use global local model, else use profile model"
    pub fn resolve_embedding_model(&self, profile: &Profile) -> String {
        if profile.embedder_type == "local" {
            self.local_embedding_model.clone()
        } else {
            profile.embedding_model.clone()
        }
    }

    /// Resolve the effective profile to use for a given run.
    /// 
    /// Logic:
    /// 1. Start with the base Profile (from -p flag or default)
    /// 2. If a collection is requested (-c):
    ///    a. Check `collection_aliases` to resolve to canonical name
    ///    b. Check `collections` for a matching key
    /// 3. If a CollectionConfig is found, merge it ON TOP of the base Profile.
    ///    - Overrides: embedder, model, url, chunk_size (we need to pass chunk_size out separately or add to Profile)
    ///    - Sets: default_collection_name = config.name
    /// 4. Return the finalized Profile.
    pub fn resolve_profile(&self, requested_profile: Option<&str>, requested_collection: Option<&str>) -> Result<Profile> {
        // 1. Get Base Profile
        let base_profile_name = requested_profile.unwrap_or(&self.default_profile);
        let mut profile = self.profiles.get(base_profile_name)
            .ok_or_else(|| anyhow::anyhow!("Profile '{}' not found", base_profile_name))?
            .clone();

        // 2. Resolve Collection
        if let Some(mut c_name) = requested_collection {
             // 2a. Resolve Alias -> Real Key
             if let Some(real_key) = self.collection_aliases.get(c_name) {
                 c_name = real_key.as_str();
             }
             
             // 2b. Check for Collection Config
             if let Some(c_config) = self.collections.get(c_name) {
                 // 3. Merge Overrides
                 profile.default_collection_name = c_config.name.clone();
                 
                 if let Some(ref et) = c_config.embedder_type {
                     profile.embedder_type = et.clone();
                 }
                 if let Some(ref em) = c_config.embedding_model {
                     profile.embedding_model = em.clone();
                 }
                  if let Some(ref url) = c_config.ollama_url {
                      profile.ollama_url = url.clone();
                  }
                  if let Some(ref key) = c_config.qdrant_api_key {
                      profile.qdrant_api_key = Some(key.clone());
                  }
                  if let Some(ref key) = c_config.ollama_api_key {
                      profile.ollama_api_key = Some(key.clone());
                  }
                 // chunk_size is currently in IngestionConfig, not Profile. 
                 // We might need to carry it? 
                 // For now, let's just update the profile fields we have.
             } else {
                 // No config found, just use the raw name
                 profile.default_collection_name = c_name.to_string();
             }
        }
        
        Ok(profile)
    }

    /// Helper to get effective chunk size if a collection overrides it
    pub fn resolve_chunk_size(&self, requested_collection: Option<&str>) -> usize {
        if let Some(mut c_name) = requested_collection {
             if let Some(real_key) = self.collection_aliases.get(c_name) {
                 c_name = real_key.as_str();
             }
             if let Some(c_config) = self.collections.get(c_name) {
                 if let Some(size) = c_config.chunk_size {
                     return size;
                 }
             }
        }
        self.ingestion.chunk_size
    }

    /// Helper to get effective max_chunk_size if a collection overrides it
    pub fn resolve_max_chunk_size(&self, requested_collection: Option<&str>) -> Option<usize> {
        if let Some(mut c_name) = requested_collection {
             if let Some(real_key) = self.collection_aliases.get(c_name) {
                 c_name = real_key.as_str();
             }
             if let Some(c_config) = self.collections.get(c_name) {
                 if let Some(max) = c_config.max_chunk_size {
                     return Some(max);
                 }
             }
        }
        self.ingestion.max_chunk_size
    }

    /// Helper to get effective chunk_overlap if a collection overrides it
    pub fn resolve_chunk_overlap(&self, requested_collection: Option<&str>) -> usize {
        if let Some(mut c_name) = requested_collection {
             if let Some(real_key) = self.collection_aliases.get(c_name) {
                 c_name = real_key.as_str();
             }
             if let Some(c_config) = self.collections.get(c_name) {
                 if let Some(overlap) = c_config.chunk_overlap {
                     return overlap;
                 }
             }
        }
        self.ingestion.chunk_overlap
    }

    /// Helper to resolve whether to use GPU for local embeddings
    pub fn resolve_local_use_gpu(&self, requested_collection: Option<&str>) -> bool {
        if let Some(mut c_name) = requested_collection {
             if let Some(real_key) = self.collection_aliases.get(c_name) {
                 c_name = real_key.as_str();
             }
             if let Some(c_config) = self.collections.get(c_name) {
                 if let Some(use_gpu) = c_config.use_gpu {
                     return use_gpu;
                 }
             }
        }
        self.local_use_gpu
    }

    /// Helper to get effective gpu_batch_size
    pub fn resolve_gpu_batch_size(&self, _requested_collection: Option<&str>) -> usize {
        // For now, it's just the global ingestion setting
        self.ingestion.gpu_batch_size
    }

    /// Load config from XDG config directory or create default
    pub fn load() -> Result<Self> {
        let config_path = Self::get_path()?;
        
        if !config_path.exists() {
            // Write default config
            let default_config = Config::default();
            // Ensure dir exists
            if let Some(parent) = config_path.parent() {
                fs::create_dir_all(parent)?;
            }
            let toml_str = toml::to_string_pretty(&default_config)?;
            fs::write(&config_path, toml_str)?;
            return Ok(default_config);
        }

        let content = fs::read_to_string(&config_path)?;
        let mut config: Config = toml::from_str(&content)
            .context("Failed to parse config.toml")?;
        
        // ENV VAR OVERRIDE: VECDB_USE_GPU
        // Allow users to force-disable GPU via environment (e.g., mcp_config.json)
        if let Ok(val) = std::env::var("VECDB_USE_GPU") {
            let val = val.trim().to_lowercase();
            if val == "false" || val == "0" {
                if crate::output::OUTPUT.is_interactive && config.local_use_gpu {
                    eprintln!("⚠️  Overriding local_use_gpu=false (via VECDB_USE_GPU env var)");
                }
                config.local_use_gpu = false;
            } else if val == "true" || val == "1" {
                if crate::output::OUTPUT.is_interactive && !config.local_use_gpu {
                    eprintln!("ℹ️  Overriding local_use_gpu=true (via VECDB_USE_GPU env var)");
                }
                config.local_use_gpu = true;
            }
        }
        
        // Validate: Warn if local profiles specify embedding_model (should use global local_embedding_model)
        for (profile_name, profile) in &config.profiles {
            if crate::output::OUTPUT.is_interactive && profile.embedder_type == "local" && profile.embedding_model != default_embedding_model() {
                eprintln!(
                    "⚠️  WARNING: Profile '{}' uses embedder_type=\"local\" but specifies embedding_model=\"{}\".\n\
                     Local profiles should use the global 'local_embedding_model' config field.\n\
                     The profile's embedding_model field will be IGNORED.",
                    profile_name, profile.embedding_model
                );
            }
        }
        
        Ok(config)
    }

    /// Resolve config path: ~/.config/vecdb/config.toml
    /// Respects VECDB_CONFIG environment variable if set.
    pub fn get_path() -> Result<PathBuf> {
        if let Ok(path) = std::env::var("VECDB_CONFIG") {
            return Ok(PathBuf::from(path));
        }

        let mut path = dirs::config_dir()
            .ok_or_else(|| anyhow::anyhow!("Could not determine config directory"))?;
        path.push("vecdb");
        path.push("config.toml");
        Ok(path)
    }

    /// Get a specific profile or the default one
    pub fn get_profile(&self, name: Option<&str>) -> Result<&Profile> {
        let profile_name = name.unwrap_or(&self.default_profile);
        self.profiles.get(profile_name).ok_or_else(|| {
            anyhow::anyhow!("Profile '{}' not found in configuration", profile_name)
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_resolve_profile_defaults() {
        let mut config = Config::default();
        // Create an "edge" profile with specific collection
        config.profiles.insert(
            "edge".to_string(),
            Profile {
                qdrant_url: "http://localhost:6333".to_string(),
                default_collection_name: "docs_qwen".to_string(), // DIFFERENT from default "docs"
                ollama_url: "http://localhost:11434".to_string(),
                embedding_model: "test-model".to_string(),
                accept_invalid_certs: true,
                embedder_type: "ollama".to_string(),
                qdrant_api_key: None,
                ollama_api_key: None,
            }
        );

        // Case 1: No explicit collection provided -> Should use profile's "docs_qwen"
        let resolved = config.resolve_profile(Some("edge"), None).unwrap();
        assert_eq!(resolved.default_collection_name, "docs_qwen");

        // Case 2: Explicit override -> Should use override
        let resolved_override = config.resolve_profile(Some("edge"), Some("my_custom_col")).unwrap();
        assert_eq!(resolved_override.default_collection_name, "my_custom_col");

        // Case 3: Default profile -> Should use default "docs"
        let resolved_default = config.resolve_profile(None, None).unwrap();
        assert_eq!(resolved_default.default_collection_name, "docs");
    }
}
