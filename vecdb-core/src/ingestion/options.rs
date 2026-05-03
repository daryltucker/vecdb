use crate::config::{PathRule, QuantizationType};
use crate::vecdbrc::Route;
use std::collections::HashMap;
use std::path::PathBuf;

pub struct IngestionOptions {
    /// Primary path (kept for backward compat). When `file_allowlist` is
    /// set, this should be the common parent of all allowed files.
    pub path: String,
    /// Optional list of specific file paths to ingest within `path`.
    /// When set, only files whose canonical path matches one of these
    /// will be processed. All others in the walk are skipped.
    /// This enables multi-file glob from CLI: find common parent, set
    /// path=parent, allowlist=expanded files → single walk, one pipeline.
    pub file_allowlist: Option<Vec<String>>,
    /// The project root displayed to the user / stored in metadata.
    /// Computed automatically as the common ancestor of all input paths.
    pub project_root: Option<String>,
    pub collection: String,
    /// Optional `.vecdbrc` routes for per-file collection routing.
    /// When set, each file is routed to its matching collection instead of
    /// using the single `collection` field for everything.
    /// The `collection` field serves as the fallback (CLI flag or `[default]`).
    pub vecdbrc_routes: Option<Vec<Route>>,
    /// If routing is active, the project root (parent dir of .vecdbrc)
    /// for resolving relative paths against route globs.
    pub vecdbrc_root: Option<PathBuf>,
    pub chunk_size: usize,
    pub max_chunk_size: Option<usize>,
    pub chunk_overlap: usize,
    pub respect_gitignore: bool,
    /// If true, .vectorignore files are NOT respected during file walking
    pub ignore_vectorignore: bool,
    pub strategy: String,
    pub tokenizer: String,
    pub git_ref: Option<String>,
    // Stank Hunt: Globbing Support
    pub extensions: Option<Vec<String>>, // e.g. ["rs", "md"]
    pub excludes: Option<Vec<String>>,   // e.g. ["*.tmp", "target/"]
    pub dry_run: bool,                   // If true, list files but do not chunk/embed
    pub metadata: Option<HashMap<String, serde_json::Value>>, // Global metadata for all files
    pub path_rules: Vec<PathRule>,       // D031: Smart Path Parsing
    pub max_concurrent_requests: usize,  // Concurrency Limit
    pub gpu_batch_size: usize,           // GPU Batch Size
    pub quantization: Option<QuantizationType>,
}
