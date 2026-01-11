use serde::{Deserialize, Serialize};
use serde_json::Value;

/// Represents a single versioned snapshot of an artifact
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Snapshot {
    /// Text content of the snapshot
    pub content: String,
    /// Associated metadata (from vecq or file system)
    pub metadata: Value,
}

impl Snapshot {
    pub fn new(content: String, metadata: Value) -> Self {
        Self { content, metadata }
    }
}
