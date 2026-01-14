use crate::config::{PathRule, QuantizationType};
use std::collections::HashMap;

pub struct IngestionOptions {
    pub path: String,
    pub collection: String,
    pub chunk_size: usize,
    pub max_chunk_size: Option<usize>,
    pub chunk_overlap: usize,
    pub respect_gitignore: bool,
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
