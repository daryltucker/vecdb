use crate::config::default_max_chunk_bytes;
use crate::config::{OversizePolicy, PathRule, QuantizationType};
use crate::vecdbrc::Route;
use std::collections::HashMap;
use std::path::PathBuf;

/// What a dry run reports. Parsing and chunking always run; embedding and
/// writing never do.
///
/// An enum rather than two bools so "show chunks" without a dry run cannot be
/// expressed.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum DryRun {
    /// A real ingest.
    #[default]
    Off,
    /// One line per file: chunk count, bytes, and how many would trip the
    /// oversize ceiling.
    Files,
    /// Every chunk exactly as it would be embedded — after the oversize split,
    /// labels included — with its crumbtrail, size and line span.
    ///
    /// The text is what the model sees; payload keys are not. Without this the
    /// only way to read a chunk was to ingest into a scratch collection and
    /// scroll it back out, so chunking defects shipped that one look at the
    /// output would have caught.
    Chunks {
        /// One JSON object per chunk instead of the human listing. Carried
        /// here because the global output context never sees `--json`.
        json: bool,
    },
}

impl DryRun {
    pub fn is_on(self) -> bool {
        self != DryRun::Off
    }
}

/// `Clone` so a caller can run the same plan against several destinations —
/// see `only_collection`. Every field is owned data or a plain value; there is
/// no shared handle here whose duplication would mean anything.
#[derive(Clone)]
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

    /// Run-level AST packing granularity, in non-whitespace characters.
    /// Per-destination values in `route_chunking` override it.
    pub pack_target_bytes: Option<usize>,
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

    /// Restrict this pass to files that route to exactly this collection.
    ///
    /// A `.vecdbrc` can fan one tree across collections whose profiles resolve
    /// to different models or different stores. A single pass cannot serve
    /// those: it holds one embedder and one backend, resolved before any file
    /// is read, so routing a file elsewhere would embed it with the wrong model
    /// and write it to the wrong endpoint — creating the target at the wrong
    /// dimension when it does not yet exist.
    ///
    /// The caller therefore runs one pass per destination, each with that
    /// collection's own resolution, and sets this so the pass ignores files
    /// belonging to the others. `None` means "take every file", which is the
    /// single-destination case and the default.
    pub only_collection: Option<String>,

    /// Destination for files that match no `.vecdbrc` route.
    ///
    /// `collection` cannot serve this during a split run: it is also the
    /// write-target the embedding-space guard validates, so a per-destination
    /// pass must set it to that destination. If route resolution then fell back
    /// to it, every unrouted file would be claimed by whichever pass is running
    /// and ingested into the wrong space — the exact failure the split exists to
    /// prevent. `None` means "use `collection`", which is correct whenever there
    /// is only one destination.
    pub route_default_collection: Option<String>,
    /// What to do with a chunk that exceeds the resolved ceiling.
    ///
    /// Never an abort. An oversized chunk is a configuration problem, and taking
    /// the whole ingest down for one file helps nobody — but neither policy will
    /// store something that misrepresents its source.
    pub on_oversize: OversizePolicy,
    pub chunk_overlap: usize,
    pub respect_gitignore: bool,
    /// If true, .vectorignore files are NOT respected during file walking
    pub bypass_vectorignore: bool,
    pub strategy: String,
    pub tokenizer: String,
    pub git_ref: Option<String>,
    // Stank Hunt: Globbing Support
    pub extensions: Option<Vec<String>>, // e.g. ["rs", "md"]
    pub excludes: Option<Vec<String>>,   // e.g. ["*.tmp", "target/"]
    // If true, list files but do not chunk/embed — corrected: a dry run DOES
    // parse and chunk (that is how it reports chunk counts); it never embeds
    // or writes. See `DryRun` for what each mode prints.
    pub dry_run: DryRun,
    pub metadata: Option<HashMap<String, serde_json::Value>>, // Global metadata for all files
    /// Per-route metadata from `.vecdbrc` routes, keyed by collection name.
    /// Merged into chunk metadata after global `metadata` but before CLI `-m`
    /// overrides. The pipeline looks up the metadata for each file's resolved
    /// collection.
    pub route_metadata:
        Option<std::collections::HashMap<String, std::collections::BTreeMap<String, String>>>,
    pub path_rules: Vec<PathRule>,      // D031: Smart Path Parsing
    pub max_concurrent_requests: usize, // Concurrency Limit
    pub gpu_batch_size: usize,          // GPU Batch Size
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
    /// Granularity for AST-parsed content, in non-whitespace characters.
    /// `None` uses the run-level default.
    ///
    /// A SEPARATE knob from `target_chunk_size`, and the one that governs code,
    /// markdown, JSON and YAML — `target_chunk_size` drives the generic chunker
    /// and the derived ceiling. Keeping it here, rather than only at run level,
    /// is what lets one routed ingest chunk each destination as configured.
    pub pack_target_bytes: Option<usize>,
}

impl ChunkSpec {
    /// The byte ceiling, derived when unset. Never below `target_chunk_size` — a
    /// ceiling under the target it protects is a second chunker, not a guard.
    pub fn ceiling(&self) -> usize {
        self.max_chunk_bytes
            .unwrap_or_else(|| default_max_chunk_bytes(self.target_chunk_size))
    }

    /// AST packing granularity for this destination, defaulted when unset.
    pub fn pack_target(&self) -> usize {
        self.pack_target_bytes
            .unwrap_or(crate::config::DEFAULT_PACK_TARGET_BYTES)
    }

    /// What a parser needs to size chunks for this destination: how much to
    /// pack, and the ceiling `flush_chunks` will split at.
    pub fn parser_sizing(&self) -> crate::parsers::ParserSizing {
        crate::parsers::ParserSizing {
            pack_target_bytes: self.pack_target(),
            max_chunk_bytes: self.ceiling(),
        }
    }
}

/// Payload keys vecdb writes itself. User-supplied metadata may not use them.
///
/// The ingest path sets these AFTER merging user metadata, so a `.vecdbrc`
/// route `metadata = { source = "x" }` arrived as `source: "./file.md"` —
/// silently, with keys vecdb does not set passing through intact beside it.
/// A value that cannot survive is refused up front rather than dropped. Keys starting `__meta_` belong to the genesis point.
///
/// `tier1`'s `reserved_keys_cover_every_key_the_adapter_writes` checks this
/// list against real chunks, so a new built-in key cannot slip past it.
pub const RESERVED_PAYLOAD_KEYS: &[&str] = &[
    "content",
    "document_id",
    "path",
    "full_path",
    "source",
    "source_type",
    "language",
    "size_bytes",
    "commit_sha",
    "git_ref",
    "element_type",
    "name",
    "line_start",
    "line_end",
    "file_type",
    "docstring",
    "intent",
    "visibility",
    "crumbtrail",
    "context_label",
    "packed_elements",
    "packed_element_types",
    "split_part",
    "original_chunk_id",
    "chunk_index",
    "stream_index",
    "timestamp",
    "_model_name",
];

/// True for a key user metadata may not set — see [`RESERVED_PAYLOAD_KEYS`].
pub fn is_reserved_payload_key(key: &str) -> bool {
    key.starts_with("__meta_") || RESERVED_PAYLOAD_KEYS.contains(&key)
}

/// The check behind [`IngestionOptions::check_user_metadata`], over the three
/// places user keys come from.
pub fn check_user_metadata(
    metadata: Option<&HashMap<String, serde_json::Value>>,
    route_metadata: Option<&HashMap<String, std::collections::BTreeMap<String, String>>>,
    path_rules: &[PathRule],
) -> anyhow::Result<()> {
    let mut clashes: Vec<String> = Vec::new();
    if let Some(meta) = metadata {
        clashes.extend(
            meta.keys()
                .filter(|k| is_reserved_payload_key(k))
                .map(|k| format!("metadata key '{k}'")),
        );
    }
    if let Some(routes) = route_metadata {
        for (collection, meta) in routes {
            clashes.extend(
                meta.keys().filter(|k| is_reserved_payload_key(k)).map(|k| {
                    format!(".vecdbrc route metadata key '{k}' (collection '{collection}')")
                }),
            );
        }
    }
    for rule in path_rules {
        if let Ok(re) = regex::Regex::new(&rule.pattern) {
            clashes.extend(
                re.capture_names()
                    .flatten()
                    .filter(|k| is_reserved_payload_key(k))
                    .map(|k| format!("path rule capture '{k}' (pattern '{}')", rule.pattern)),
            );
        }
    }
    if clashes.is_empty() {
        return Ok(());
    }
    clashes.sort();
    anyhow::bail!(
        "refusing to ingest: user metadata would be silently overwritten by payload \
         keys vecdb writes itself:\n    {}\nRename them. Reserved: {}, and any key \
         starting '__meta_'.",
        clashes.join("\n    "),
        RESERVED_PAYLOAD_KEYS.join(", ")
    )
}

impl IngestionOptions {
    /// Refuse user metadata that vecdb would overwrite.
    ///
    /// Covers every source of user keys: `-m` / tool `metadata`, `.vecdbrc`
    /// route `metadata`, and path-rule capture group names. Checked before
    /// anything is written, and names every offender at once.
    pub fn check_user_metadata(&self) -> anyhow::Result<()> {
        check_user_metadata(
            self.metadata.as_ref(),
            self.route_metadata.as_ref(),
            &self.path_rules,
        )
    }

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
            // Recorded because it is what actually cut the chunks for AST
            // content. Without it the genesis describes the generic chunker's
            // parameters for chunks the generic chunker never touched.
            pack_target_bytes: spec.pack_target(),
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
                pack_target_bytes: self.pack_target_bytes,
            })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// REGRESSION: a `.vecdbrc` route `source = …` arrived
    /// as vecdb's own `source`, silently. A colliding key is now refused, and
    /// a key vecdb does not write still passes.
    #[test]
    fn user_metadata_may_not_shadow_builtin_keys() {
        let route = |k: &str| {
            let mut m = std::collections::BTreeMap::new();
            m.insert(k.to_string(), "embedcheck-fixture".to_string());
            HashMap::from([("ee-smoke".to_string(), m)])
        };

        let err = check_user_metadata(None, Some(&route("source")), &[]).unwrap_err();
        assert!(err.to_string().contains("'source'"), "{err}");
        assert!(check_user_metadata(None, Some(&route("ecosystem")), &[]).is_ok());

        let cli = HashMap::from([("crumbtrail".to_string(), serde_json::json!("x"))]);
        assert!(check_user_metadata(Some(&cli), None, &[]).is_err());
        assert!(check_user_metadata(
            Some(&HashMap::from([(
                "__meta_vecdb".to_string(),
                serde_json::json!(1)
            )])),
            None,
            &[]
        )
        .is_err());

        let rule = PathRule {
            pattern: "(?P<language>[a-z]+)/.*".to_string(),
        };
        assert!(check_user_metadata(None, None, &[rule]).is_err());
        let rule = PathRule {
            pattern: "(?P<year>\\d{4})/.*".to_string(),
        };
        assert!(check_user_metadata(None, None, &[rule]).is_ok());
    }
}
