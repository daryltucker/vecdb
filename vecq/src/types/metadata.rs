use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use chrono::{DateTime, Utc};
use vecdb_common::FileType;

/// Metadata about a parsed document
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DocumentMetadata {
    pub file_type: FileType,
    pub path: PathBuf,
    pub size: u64,
    pub modified: Option<DateTime<Utc>>,
    pub encoding: String,
    pub line_count: usize,
    pub hash: Option<String>, // For caching
}

impl DocumentMetadata {
    /// Create new document metadata
    pub fn new(path: PathBuf, size: u64) -> Self {
        let file_type = FileType::from_path(&path);
        Self {
            file_type,
            path,
            size,
            modified: None,
            encoding: "utf-8".to_string(),
            line_count: 0,
            hash: None,
        }
    }

    /// Update line count from content
    pub fn with_line_count(mut self, content: &str) -> Self {
        self.line_count = content.lines().count();
        self
    }

    /// Update modification time
    pub fn with_modified(mut self, modified: DateTime<Utc>) -> Self {
        self.modified = Some(modified);
        self
    }

    /// Update content hash for caching
    pub fn with_hash(mut self, hash: String) -> Self {
        self.hash = Some(hash);
        self
    }

    /// Override the file type (useful when path doesn't indicate type)
    pub fn with_file_type(mut self, file_type: FileType) -> Self {
        self.file_type = file_type;
        self
    }
}
