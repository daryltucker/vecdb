/*
 * PURPOSE:
 *   Manages the state of ingested files to enable incremental ingestion.
 *   Tracks file hashes to determine if a file has changed since last ingestion.
 *
 * STORAGE:
 *   State is stored in `.vecdb/state.toml` relative to the ingestion root.
 */

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use sha2::{Sha256, Digest};
use uuid::Uuid;

use chrono::{DateTime, Utc};

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct CollectionState {
    pub id: String,
    pub last_ingested_at: Option<DateTime<Utc>>,
    pub files: HashMap<PathBuf, String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct IngestionState {
    #[serde(default)]
    pub collections: HashMap<String, CollectionState>,
    
    // Legacy fields for backward compatibility during migration
    #[serde(skip_serializing_if = "HashMap::is_empty", default)]
    pub files: HashMap<PathBuf, String>,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub last_ingested_at: Option<chrono::DateTime<chrono::Utc>>,
}

impl IngestionState {
    pub fn load(root: &Path) -> Result<Self> {
        let state_path = root.join(".vecdb/state.toml");
        if state_path.exists() {
            let content = fs::read_to_string(&state_path)
                .context("Failed to read state file")?;
            let state: IngestionState = toml::from_str(&content)
                .context("Failed to parse state file")?;
            
            // Auto-migrate legacy root-level files to "default" if collections is empty
            // Actually, we can't migrate to a specific collection without knowing WHICH one it was.
            // So we'll just leave them there, but new ingests will use the collections map.
            Ok(state)
        } else {
            Ok(Self::default())
        }
    }

    pub fn save(&self, root: &Path) -> Result<()> {
        // Cannot save state if the root is a single file (cannot create .vecdb dir)
        if root.is_file() {
            return Ok(());
        }

        let vecdb_dir = root.join(".vecdb");
        if !vecdb_dir.exists() {
            fs::create_dir_all(&vecdb_dir)?;
        }
        
        // Remove legacy fields before saving to clean up
        let mut to_save = self.clone();
        to_save.files.clear();
        to_save.last_ingested_at = None;

        let content = toml::to_string_pretty(&to_save)?;
        let state_path = vecdb_dir.join("state.toml");
        fs::write(state_path, content)?;
        Ok(())
    }

    /// Update the hash for a file. Returns true if the hash changed (or was new).
    pub fn update_file(&mut self, collection: &str, rel_path: PathBuf, new_hash: String) -> bool {
         let col_state = self.collections.entry(collection.to_string()).or_insert_with(|| CollectionState {
             id: Uuid::new_v4().to_string(), // Should be set externally, but safe default
             last_ingested_at: None,
             files: HashMap::new(),
         });
         
        if let Some(old_hash) = col_state.files.get(&rel_path) {
            if old_hash == &new_hash {
                return false;
            }
        }
        col_state.files.insert(rel_path, new_hash);
        true
    }
    
    pub fn get_collection_id(&self, collection: &str) -> Option<String> {
        self.collections.get(collection).map(|c| c.id.clone())
    }
    
    pub fn set_collection_id(&mut self, collection: &str, id: String) {
        let col_state = self.collections.entry(collection.to_string()).or_default();
        col_state.id = id;
    }
    
    pub fn clear_collection(&mut self, collection: &str, new_id: String) {
        self.collections.insert(collection.to_string(), CollectionState {
            id: new_id,
            last_ingested_at: Some(Utc::now()),
            files: HashMap::new(),
        });
    }
    
    pub fn touch_collection(&mut self, collection: &str) {
         if let Some(c) = self.collections.get_mut(collection) {
             c.last_ingested_at = Some(Utc::now());
         }
    }
}

pub fn compute_hash(content: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(content);
    let result = hasher.finalize();
    format_hex(&result)
}

/// Fast file change detection using metadata only (no content read required).
/// Hashes: path + size + modification time
pub fn compute_file_metadata_hash(path: &std::path::Path) -> Result<String> {
    use std::time::UNIX_EPOCH;
    
    let meta = std::fs::metadata(path)?;
    let mut hasher = Sha256::new();
    
    hasher.update(path.to_string_lossy().as_bytes());
    hasher.update(meta.len().to_le_bytes());
    
    if let Ok(modified) = meta.modified() {
        if let Ok(duration) = modified.duration_since(UNIX_EPOCH) {
            hasher.update(duration.as_secs().to_le_bytes());
        }
    }
    
    Ok(format_hex(&hasher.finalize()))
}

fn format_hex(bytes: &[u8]) -> String {
    use std::fmt::Write;
    let mut s = String::with_capacity(bytes.len() * 2);
    for b in bytes {
        write!(s, "{:02x}", b).expect("Writing to String should never fail");
    }
    s
}
