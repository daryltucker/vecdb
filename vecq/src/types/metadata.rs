use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
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
    /// How many syntax errors the parser recovered from: the grammar's `ERROR`
    /// and `MISSING` nodes, for the tree-sitter languages and HTML. `0` for a
    /// file that parsed cleanly — and for the library parsers (Markdown, JSON,
    /// TOML, YAML), which have no error recovery: malformed input is an `Err`
    /// from `parse_file`, never a partial document. Python (rustpython) is the
    /// same until it moves to tree-sitter.
    ///
    /// So a consumer learns whether a file parsed cleanly without walking the
    /// tree for `attributes.kind == "ERROR"` elements, which remain in place,
    /// spanned, alongside this count.
    #[serde(default)]
    pub syntax_errors: usize,
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
            syntax_errors: 0,
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
