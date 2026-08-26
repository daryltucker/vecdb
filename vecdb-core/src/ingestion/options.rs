use crate::config::default_max_chunk_bytes;
use crate::config::{OversizePolicy, PathRule, QuantizationType};
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
    pub target_chunk_size: usize,
    pub max_chunk_bytes: Option<usize>,
    /// Chunk parameters per routed destination, keyed by collection name.
    ///
    /// A `.vecdbrc` fans one run across several collections, and chunking is a
    /// property of the destination: profiles in this fleet span 16x in
    /// `target_chunk_size` (384 to 6144). Chunking every route identically means the
    /// files headed somewhere else are cut to the wrong size, and chunk size is
    /// baked into the vectors at ingest — a re-ingest is the only repair.
    ///
    /// Empty means uniform: every destination uses the flat fields above. The
    /// caller populates it because resolving a collection's chunk parameters
    /// needs `Config`, which the ingestion layer deliberately does not depend on.
    pub route_chunking: HashMap<String, ChunkSpec>,
    /// What to do with a chunk that exceeds the resolved ceiling.
    ///
    /// Never an abort. An oversized chunk is a configuration problem, and taking
    /// the whole ingest down for one file helps nobody — but neither policy will
    /// store something that misrepresents its source.
    pub on_oversize: OversizePolicy,
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
    /// Accept writing into a collection whose model matches on architecture and
    /// parameter size but differs in quantization (e.g. Q8_0 into a Q4_K_M
    /// collection). Off by default: writes are strict because a bad one
    /// contaminates permanently, and mixing builds should be a deliberate act.
    pub allow_quantization_delta: bool,
}

/// Chunk parameters for one destination.
#[derive(Debug, Clone, Copy)]
pub struct ChunkSpec {
    pub target_chunk_size: usize,
    pub chunk_overlap: usize,
    /// Byte ceiling. `None` derives from `target_chunk_size`.
    pub max_chunk_bytes: Option<usize>,
}

impl ChunkSpec {
    /// The byte ceiling, derived when unset. Never below `target_chunk_size` — a
    /// ceiling under the target it protects is a second chunker, not a guard.
    pub fn ceiling(&self) -> usize {
        self.max_chunk_bytes
            .unwrap_or_else(|| default_max_chunk_bytes(self.target_chunk_size))
    }
}

impl IngestionOptions {
    /// What to record in a collection's genesis: the RESOLVED chunk parameters
    /// for that destination, plus the tokenizer they are denominated in.
    ///
    /// `target_chunk_size` counts whatever `tokenizer` counts, so recording the
    /// number without the tokenizer would store an ambiguous figure — 512
    /// cl100k_base tokens and 512 bytes are wildly different corpora.
    pub fn chunking_identity(&self, collection: &str) -> crate::types::ChunkingIdentity {
        let spec = self.chunking_for(collection);
        crate::types::ChunkingIdentity {
            target_chunk_size: spec.target_chunk_size,
            chunk_overlap: spec.chunk_overlap,
            // The resolved ceiling, never the Option: once a chunk has been cut
            // by it, a derived ceiling and a written one are the same fact.
            max_chunk_bytes: spec.ceiling(),
            tokenizer: self.tokenizer.clone(),
        }
    }

    /// Chunk parameters for a destination, falling back to the flat fields.
    pub fn chunking_for(&self, collection: &str) -> ChunkSpec {
        self.route_chunking
            .get(collection)
            .copied()
            .unwrap_or(ChunkSpec {
                target_chunk_size: self.target_chunk_size,
                chunk_overlap: self.chunk_overlap,
                max_chunk_bytes: self.max_chunk_bytes,
            })
    }
}
