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

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct IngestionState {
    pub files: HashMap<PathBuf, String>, // Relative Path -> Hex Digest
    pub last_ingested_at: Option<chrono::DateTime<chrono::Utc>>,
}

impl IngestionState {
    pub fn load(root: &Path) -> Result<Self> {
        let state_path = root.join(".vecdb/state.toml");
        if state_path.exists() {
            let content = fs::read_to_string(&state_path)
                .context("Failed to read state file")?;
            toml::from_str(&content)
                .context("Failed to parse state file")
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
        
        // Mark timestamp
        let mut to_save = self.clone();
        to_save.last_ingested_at = Some(chrono::Utc::now());

        let content = toml::to_string_pretty(&to_save)?;
        let state_path = vecdb_dir.join("state.toml");
        fs::write(state_path, content)?;
        Ok(())
    }

    /// Update the hash for a file. Returns true if the hash changed (or was new).
    pub fn update_file(&mut self, rel_path: PathBuf, new_hash: String) -> bool {
        if let Some(old_hash) = self.files.get(&rel_path) {
            if old_hash == &new_hash {
                return false;
            }
        }
        self.files.insert(rel_path, new_hash);
        true
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
        write!(s, "{:02x}", b).unwrap();
    }
    s
}
