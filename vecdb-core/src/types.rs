/*
 * PURPOSE:
 *   Defines the core data structures used throughout the vecdb-mcp system.
 *   These types act as the common language between the CLI, MCP Server,
 *   and Storage Backends.
 *
 * REQUIREMENTS:
 *   User-specified:
 *   - Must support "Document Ingestion" (R-001)
 *   - Must support "Embedding Generation" (R-004)
 *   - Must preserve metadata (R-007)
 *
 *   Implementation-discovered:
 *   - Needs Serialization (Serde) for MCP/JSON transport
 *   - Needs Clone/Debug for developer ergonomics
 *   - Split Document vs Chunk to represent Source vs Vectorized units
 *
 * IMPLEMENTATION RULES:
 *   1. Use `HashMap<String, serde_json::Value>` for metadata
 *      Rationale: Maximum flexibility for arbitrary user data (Law #1 Config is Code)
 *
 *   2. Vectors are `Vec<f32>`
 *      Rationale: Standard representation for ONNX/Qdrant
 *
 *   3. IDs are UUIDs
 *      Rationale: Collision-free distributed generation suitable for large datasets
 *
 * USAGE:
 *   use vecdb_core::types::{Document, Chunk};
 *   let doc = Document::new("path/to/file.txt", "content");
 *
 * SELF-HEALING INSTRUCTIONS:
 *   - If new metadata fields required: Update struct and add migration logic/Option types
 *   - If vector dimensions change: `vectors` field is generic `Vec<f32>`, so logic is runtime-dependent
 *
 * RELATED FILES:
 *   - docs/specs/INGESTION_DESIGN.md - Defines the chunking strategy
 *   - src/backend.rs - Consumes these types
 *
 * MAINTENANCE:
 *   Update when:
 *   - Qdrant/Backend API changes require new fields
 *   - MCP Protocol adds new resource primitives
 */

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use uuid::Uuid;

// ARCHITECTURE NOTE:
// This struct uses HashMap<String, Value> for metadata.
// In high-scale environments (>1M vectors), this causes significant heap fragmentation and overhead.
// A future refactor could replace it with bit-packed structs or a zero-copy
// map (`rkyv`), trading flexibility for locality.
/// Represents a source file or logical document before ingestion.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Document {
    pub id: String,
    pub path: String,
    pub content: String,
    pub metadata: HashMap<String, serde_json::Value>,
    pub created_at: chrono::DateTime<chrono::Utc>,
}

impl Document {
    pub fn new(path: impl Into<String>, content: impl Into<String>) -> Self {
        Self {
            id: Uuid::new_v4().to_string(),
            path: path.into(),
            content: content.into(),
            metadata: HashMap::new(),
            created_at: chrono::Utc::now(),
        }
    }

    pub fn with_metadata(mut self, key: &str, value: impl Into<serde_json::Value>) -> Self {
        self.metadata.insert(key.to_string(), value.into());
        self
    }
}

/// A semantic unit of a document (paragraph, sentence) with an associated vector.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Chunk {
    pub id: String,
    pub document_id: String,
    pub content: String,
    pub vector: Option<Vec<f32>>,
    pub metadata: HashMap<String, serde_json::Value>,
    pub page_num: Option<usize>,
    pub byte_start: usize,
    pub byte_end: usize,
    pub start_line: Option<usize>,
    pub end_line: Option<usize>,
}

impl Chunk {
    pub fn new(document_id: &str, content: impl Into<String>) -> Self {
        Self {
            id: Uuid::new_v4().to_string(),
            document_id: document_id.to_string(),
            content: content.into(),
            vector: None,
            metadata: HashMap::new(),
            page_num: None,
            byte_start: 0,
            byte_end: 0,
            start_line: None,
            end_line: None,
        }
    }
}

/// Result from a semantic search operation.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SearchResult {
    pub id: String,
    pub score: f32,
    pub content: String,
    pub document_id: String,
    pub metadata: HashMap<String, serde_json::Value>,
}

/// Information about a vector collection.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CollectionInfo {
    pub name: String,
    pub vector_count: Option<u64>,
    pub vector_size: Option<u64>,
    pub quantization: Option<crate::config::QuantizationType>,
    // NEW
    pub vectors_on_disk: Option<bool>, // VectorParams.on_disk
    pub payload_on_disk: Option<bool>, // CollectionParams.on_disk_payload
}

/// Information about a background task in the backend (e.g., optimization).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TaskInfo {
    pub id: String,
    pub collection: Option<String>,
    pub status: String, // "running", "completed", "failed"
    pub progress: Option<f32>,
    pub description: String,
}

/// Status of a local ingestion job.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum JobStatus {
    Queued,
    Running,
    Completed,
    Failed(String),
}

/// Represents a long-running local ingestion job.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Job {
    pub id: String,
    pub job_type: String, // "ingest", "history"
    pub collection: String,
    pub status: JobStatus,
    pub progress: f32, // 0.0 to 1.0
    pub pid: u32,
    pub started_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

// ── Model identity & the embedding space contract ──────────────────────
//
// A collection is only searchable if every vector in it came from the same
// embedding space. Dimension alone does not establish that: 384 and 768 are
// the two most common embedding dimensions in existence, and MERT audio
// vectors and qwen3-embedding:0.6b text vectors are both 1024-dim/Cosine.
//
// Tag strings are not identity either. Observed on one host, the tags
// `qwen3-embedding:4b` and `qwen3-embedding:4b-q4_K_M` resolve to one blob,
// while `4b-q8_0` is different weights under a name that differs by six
// characters. Record the digest; display the tag.

/// Everything known about the model that produced a set of vectors.
///
/// Sourced from Ollama's `/api/tags` (digest) and `/api/show` (the rest).
/// Every field past `name` is `Option` because an embedder backend may not
/// expose it — a missing field weakens the comparison rather than breaking it.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct ModelIdentity {
    /// Tag as configured, e.g. "qwen3-embedding:0.6b". Display, never identity.
    pub name: String,
    /// Content-addressable digest of the weights. THE identity field.
    pub digest: Option<String>,
    /// `model_info.general.architecture`, e.g. "qwen3".
    pub architecture: Option<String>,
    /// `details.family`, e.g. "qwen3". Coarser than architecture.
    pub family: Option<String>,
    /// `details.parameter_size`, e.g. "596.05M", "4.02B".
    pub parameter_size: Option<String>,
    /// `details.quantization_level`, e.g. "Q8_0", "Q4_K_M".
    pub quantization_level: Option<String>,
    /// `model_info.<arch>.embedding_length` — native dimension, no probe embed.
    pub embedding_length: Option<u64>,
    /// `model_info.<arch>.context_length` — the **maximum `num_ctx` this model
    /// will accept**, not the ceiling you actually get.
    ///
    /// Measured against `qwen3-embedding:0.6b-q8_0` on an Ollama host
    /// (declares 32768): with no `options`, `/api/embed` refused input at
    /// ~4086 tokens — Ollama's default `num_ctx` of 4096. The same input at
    /// ~12258 tokens succeeded with `options.num_ctx = 16384` and failed
    /// without it.
    ///
    /// So: `/api/embed` **does** honour `options.num_ctx`, and the effective
    /// ceiling is `num_ctx` (default 4096), bounded above by this value. Sizing
    /// chunks against `context_length` would overshoot the real limit by 8x.
    pub context_length: Option<u64>,
}

impl ModelIdentity {
    /// Minimal identity for an embedder that exposes nothing but its name.
    pub fn unknown(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            ..Default::default()
        }
    }

    /// Short human-readable form for diagnostics: `name (Q8_0, 4.02B) @digest`.
    pub fn describe(&self) -> String {
        let mut s = self.name.clone();
        let detail: Vec<&str> = [
            self.quantization_level.as_deref(),
            self.parameter_size.as_deref(),
        ]
        .into_iter()
        .flatten()
        .collect();
        if !detail.is_empty() {
            s.push_str(&format!(" ({})", detail.join(", ")));
        }
        if let Some(d) = &self.digest {
            s.push_str(&format!(" @{}", &d[..d.len().min(12)]));
        }
        s
    }
}

/// How two embedding spaces relate.
///
/// Strict digest equality is too strict to be usable: Q4_K_M and Q8_0 of one
/// model produce slightly different vectors, but the quantization error is
/// small relative to retrieval margins. Rejecting that pairing would make the
/// guard something operators route around.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Compatibility {
    /// Same digest — the same weights, bit for bit.
    Identical,
    /// Same architecture + parameter_size + dimension, different quantization.
    Compatible,
    /// Anything else, including "not enough information to tell."
    Incompatible,
}

/// The verdict plus enough context to act on it.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CompatibilityReport {
    pub tier: Compatibility,
    /// Why, in one line. Always populated.
    pub reason: String,
    /// What the operator should do about it, when there is something to do.
    pub suggestion: Option<String>,
    pub collection: ModelIdentity,
    pub active: ModelIdentity,
    pub collection_dimension: Option<u64>,
    pub active_dimension: Option<u64>,
}

impl CompatibilityReport {
    /// Reads are permissive: a mediocre ranking evaporates.
    ///
    /// `Compatible` passes with `warning()` set, because a quantization delta
    /// costs a little precision on one query and nothing thereafter.
    pub fn permits_read(&self) -> bool {
        !matches!(self.tier, Compatibility::Incompatible)
    }

    /// Writes are strict: a bad write contaminates a collection permanently
    /// and compounds with every subsequent ingest. `Compatible` requires an
    /// explicit opt-in from the caller, so mixing quantizations inside one
    /// collection is always a deliberate act.
    pub fn permits_write(&self, explicitly_allowed: bool) -> bool {
        match self.tier {
            Compatibility::Identical => true,
            Compatibility::Compatible => explicitly_allowed,
            Compatibility::Incompatible => false,
        }
    }

    /// Non-fatal note to surface to the caller, if any.
    pub fn warning(&self) -> Option<String> {
        match self.tier {
            Compatibility::Compatible => Some(format!(
                "quantization differs: collection {} vs active {} — results are \
                 comparable but not bit-identical",
                self.collection
                    .quantization_level
                    .as_deref()
                    .unwrap_or("unknown"),
                self.active
                    .quantization_level
                    .as_deref()
                    .unwrap_or("unknown"),
            )),
            _ => None,
        }
    }
}

/// Compare a collection's recorded space against this machine's active space.
///
/// Deliberately compares *definitions*, never profile names. `profiles.low` on
/// one host and `profiles.low` on another are different definitions under the
/// same name, so a name-based check would report agreement where none exists —
/// worse than no check, because it would look verified.
pub fn compare_spaces(
    collection: &ModelIdentity,
    collection_dimension: Option<u64>,
    active: &ModelIdentity,
    active_dimension: Option<u64>,
) -> CompatibilityReport {
    let build = |tier, reason: String, suggestion: Option<String>| CompatibilityReport {
        tier,
        reason,
        suggestion,
        collection: collection.clone(),
        active: active.clone(),
        collection_dimension,
        active_dimension,
    };

    // Tier 1 — digest equality. The only comparison that is actually proof.
    //
    // Deliberately checked *before* dimension. Identical weights at two widths
    // is the supported Matryoshka case, not a conflict: `docs` may be native
    // 2560-dim while a truncated variant is 1024-dim, both written by the same
    // model. Callers narrow to the collection's width using the dimension
    // `ensure_write_target` returns, so permitting it here is what makes MRL
    // work — the width is resolved, not assumed.
    if let (Some(a), Some(b)) = (&collection.digest, &active.digest) {
        if a == b {
            return build(
                Compatibility::Identical,
                format!("same model weights ({})", &a[..a.len().min(12)]),
                None,
            );
        }
    }

    // Dimension must match for every tier *below* Tier 1 — vectors of different
    // lengths cannot be compared at all, and without digest equality there is no
    // evidence that a narrowing is a legitimate MRL truncation rather than two
    // unrelated models that happen to be configured at different widths.
    if let (Some(c), Some(a)) = (collection_dimension, active_dimension) {
        if c != a {
            return build(
                Compatibility::Incompatible,
                format!(
                    "dimension mismatch: collection is {c}-dim (from {}), this machine produces {a}-dim (from {})",
                    collection.name, active.name
                ),
                Some("use a profile whose model matches the collection, or target a different collection".to_string()),
            );
        }
    }

    // Tier 2 — same model at a different quantization.
    let arch_matches = match (&collection.architecture, &active.architecture) {
        (Some(a), Some(b)) => Some(a == b),
        _ => None,
    };
    let params_match = match (&collection.parameter_size, &active.parameter_size) {
        (Some(a), Some(b)) => Some(a == b),
        _ => None,
    };

    match (arch_matches, params_match) {
        (Some(true), Some(true)) => build(
            Compatibility::Compatible,
            format!(
                "same architecture ({}) and size ({}), different build",
                collection.architecture.as_deref().unwrap_or("?"),
                collection.parameter_size.as_deref().unwrap_or("?"),
            ),
            None,
        ),
        (Some(false), _) | (_, Some(false)) => build(
            Compatibility::Incompatible,
            format!(
                "different models at the same dimension: collection {} vs active {}",
                collection.describe(),
                active.describe(),
            ),
            Some(format!(
                "dimensions coincide but the models differ — writing would mix two embedding \
                 spaces. Use a profile resolving to {}, or target a different collection.",
                collection.name
            )),
        ),
        // Not enough recorded to distinguish "same model" from "coincidence".
        // Refuse rather than assume: this is exactly the 768-dim collision case
        // that motivated the whole guard.
        _ => build(
            Compatibility::Incompatible,
            format!(
                "insufficient identity to compare: collection {} vs active {}",
                collection.describe(),
                active.describe(),
            ),
            Some(
                "re-ingest so the collection records a full model identity, or pass the \
                 explicit override if you are certain the models match"
                    .to_string(),
            ),
        ),
    }
}

/// Full metadata written to a collection's genesis point at creation.
///
/// This is what vecdb *writes*; `CollectionGenesis` is what can be read back.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GenesisMetadata {
    pub collection_id: String,
    pub model: ModelIdentity,
    pub dimension: u64,
    pub distance: String,
    /// How text was cut before the model ever saw it. See `ChunkingIdentity`.
    pub chunking: Option<ChunkingIdentity>,
    pub created_at: String,
}

/// Git revision of the running binary, for stamping into a new collection.
///
/// Separate from the version because a version cannot identify a build between
/// releases, which is precisely when semantics change.
pub fn build_revision() -> String {
    vecdb_common::revision()
}

/// How a collection's text was cut, recorded at creation.
///
/// Genesis described the model exhaustively — digest, architecture, parameter
/// size, quantization, context length — and said nothing about chunking, which
/// is baked into the vectors just as permanently and is recoverable from
/// nowhere afterwards. Measured 2026-238: a collection built at 512 tokens and
/// later rebuilt at 12000 was indistinguishable from one always built at 12000.
///
/// Deliberately a record of what was USED, not what was configured:
/// `max_chunk_bytes` is the resolved ceiling, never the `Option`, because a
/// derived ceiling and a written one are the same fact once a chunk has been
/// cut by it.
///
/// A comparison key since 2026-256: `ChunkingIdentity::delta` grades a
/// difference and `ingestion::ensure_write_target` refuses a granularity
/// change. Recording the fact is what made the gate possible; RFC-2026-238
/// specified it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ChunkingIdentity {
    /// Counted in whatever `tokenizer` counts.
    pub target_chunk_size: usize,
    pub chunk_overlap: usize,
    /// Resolved byte ceiling — `String::len()`, not tokens.
    pub max_chunk_bytes: usize,
    /// Which tokenizer `target_chunk_size` is denominated in, e.g. `cl100k_base`.
    pub tokenizer: String,
    /// AST packing granularity, in non-whitespace characters.
    ///
    /// Recorded because it, not `target_chunk_size`, is what cut the chunks for
    /// code, markdown, JSON and YAML. Omitting it left a genesis that described
    /// the generic chunker's settings for chunks the generic chunker never saw.
    pub pack_target_bytes: usize,
}

/// How a collection's recorded chunking differs from the chunking a run is
/// about to write with.
///
/// Graded rather than a plain `!=`, mirroring how the embedding space is
/// compared: not every difference is the same kind of problem.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ChunkingDelta {
    /// Fields that decide WHERE the cuts land: `target_chunk_size`,
    /// `chunk_overlap`, `tokenizer`, `pack_target_bytes`. A difference here
    /// means the collection would hold two corpora cut two different ways, and
    /// no later read can tell them apart. Each entry is `field: was -> now`.
    pub granularity: Vec<String>,
    /// `max_chunk_bytes` only, as `(recorded, current)`.
    ///
    /// Separated because it is a **safety ceiling, not a granularity choice**:
    /// it fires only on a chunk that is already oversized, and it is clamped at
    /// ingest time to whatever the local model will accept. So it moves
    /// legitimately between machines and between models without anybody
    /// reconfiguring anything. Worth saying out loud, never worth refusing over.
    pub ceiling: Option<(usize, usize)>,
}

impl ChunkingDelta {
    /// True when the collection would become a mixture of two chunkings.
    pub fn is_breaking(&self) -> bool {
        !self.granularity.is_empty()
    }
}

impl ChunkingIdentity {
    /// Compare what a collection records against what this run would write.
    ///
    /// `self` is the recorded (genesis) side, `current` the run's.
    pub fn delta(&self, current: &Self) -> ChunkingDelta {
        let mut granularity = Vec::new();
        if self.target_chunk_size != current.target_chunk_size {
            granularity.push(format!(
                "target_chunk_size: {} -> {}",
                self.target_chunk_size, current.target_chunk_size
            ));
        }
        if self.chunk_overlap != current.chunk_overlap {
            granularity.push(format!(
                "chunk_overlap: {} -> {}",
                self.chunk_overlap, current.chunk_overlap
            ));
        }
        if self.tokenizer != current.tokenizer {
            granularity.push(format!(
                "tokenizer: {} -> {}",
                self.tokenizer, current.tokenizer
            ));
        }
        if self.pack_target_bytes != current.pack_target_bytes {
            granularity.push(format!(
                "pack_target_bytes: {} -> {}",
                self.pack_target_bytes, current.pack_target_bytes
            ));
        }
        ChunkingDelta {
            granularity,
            ceiling: (self.max_chunk_bytes != current.max_chunk_bytes)
                .then_some((self.max_chunk_bytes, current.max_chunk_bytes)),
        }
    }
}

/// The magic marker that declares a collection to be vecdb's.
///
/// Written as the payload value of `__meta_vecdb`, self-describing so it is
/// unambiguous when eyeballing a raw Qdrant payload: `vecdb:1.0.4`.
///
/// This is a **positive assertion**, not an inference. Ownership used to be
/// guessed from the absence of `__meta_*` keys, which is the wrong tool for the
/// job — absence is also what a partial write, a schema change, or an unrelated
/// tool that happens to use the nil UUID looks like. A magic number cannot be
/// arrived at by accident.
pub const VECDB_GENESIS_MAGIC: &str = "vecdb";

/// What was read back from a collection's genesis point.
///
/// A Qdrant instance is shared infrastructure. The edge instance holds
/// collections written by tools that are not vecdb at all (MERT audio
/// embeddings, written directly). Those are not "incompatible vecdb
/// collections" — they are **not vecdb collections**, which is a different
/// statement and deserves different words. This is a permanent structural
/// condition, not migration debt.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct CollectionGenesis {
    /// `Some(version)` when the magic marker is present — i.e. vecdb wrote this.
    /// `None` means the collection belongs to something else. Never a guess.
    pub vecdb_version: Option<String>,
    /// Git revision of the binary that created the collection, as stamped by
    /// `vecdb-common/build.rs` (`"-dirty"` suffix included when applicable).
    ///
    /// The version alone cannot identify a build. Measured 2026-238: the `code`
    /// collection reports `1.0.4` yet was written by a development build that
    /// already contained the Python/Go fidelity fix — the version had not been
    /// bumped yet. Anything reasoning from the version alone would classify it
    /// as stale and demand a re-ingest it does not need.
    ///
    /// `None` for collections created before this was recorded.
    pub vecdb_revision: Option<String>,
    pub collection_id: Option<String>,
    pub model: ModelIdentity,
    pub dimension: Option<u64>,
    pub distance: Option<String>,
    /// `None` for collections created before this was recorded (≤ v1.1.0 at the
    /// time of writing). Absence is itself informative — see RFC-2026-238.
    pub chunking: Option<ChunkingIdentity>,
    pub created_at: Option<String>,
}

impl CollectionGenesis {
    /// Whether this collection is vecdb's to reason about at all.
    ///
    /// Every guard checks this *before* comparing embedding spaces, because
    /// "the models do not match" is a claim you can only make about a
    /// collection whose model you know.
    pub fn is_vecdb(&self) -> bool {
        self.vecdb_version.is_some()
    }

    /// Parse the `__meta_vecdb` marker value, e.g. "vecdb:1.0.4" -> "1.0.4".
    /// Anything not carrying the magic prefix is not ours, whatever else it has.
    pub fn parse_marker(raw: Option<String>) -> Option<String> {
        raw?.strip_prefix(&format!("{VECDB_GENESIS_MAGIC}:"))
            .map(str::to_string)
    }

    /// The value to stamp into a new collection.
    ///
    /// Deliberately the bare version, with no build suffix. This string is a
    /// stable contract — `parse_marker` returns everything after the colon as
    /// "the version", and anything appended here would silently become part of
    /// it. The build revision goes in its own field.
    pub fn marker_value() -> String {
        format!("{}:{}", VECDB_GENESIS_MAGIC, env!("CARGO_PKG_VERSION"))
    }
}

#[cfg(test)]
mod space_tests {
    use super::*;

    fn qwen(quant: &str, digest: &str) -> ModelIdentity {
        ModelIdentity {
            name: format!("qwen3-embedding:0.6b-{}", quant.to_lowercase()),
            digest: Some(digest.to_string()),
            architecture: Some("qwen3".into()),
            family: Some("qwen3".into()),
            parameter_size: Some("596.05M".into()),
            quantization_level: Some(quant.into()),
            embedding_length: Some(1024),
            context_length: Some(32768),
        }
    }

    #[test]
    fn same_digest_is_identical_and_writes() {
        let m = qwen("Q8_0", "sha256:aaaa1111");
        let r = compare_spaces(&m, Some(1024), &m, Some(1024));
        assert_eq!(r.tier, Compatibility::Identical);
        assert!(r.permits_write(false));
        assert!(r.permits_read());
        assert!(r.warning().is_none());
    }

    /// The spec's middle tier: quantization is tracked and known, reads pass,
    /// writes need an explicit flag.
    #[test]
    fn quantization_delta_reads_freely_but_gates_writes() {
        let r = compare_spaces(
            &qwen("Q8_0", "sha256:aaaa1111"),
            Some(1024),
            &qwen("Q4_K_M", "sha256:bbbb2222"),
            Some(1024),
        );
        assert_eq!(r.tier, Compatibility::Compatible);
        assert!(r.permits_read(), "a quant delta must never block a read");
        assert!(!r.permits_write(false), "writes are strict by default");
        assert!(r.permits_write(true), "explicit opt-in unlocks the write");
        assert!(
            r.warning().is_some(),
            "the delta must be recorded, not hidden"
        );
    }

    /// The bug this whole thing exists for: two unrelated 768-dim models.
    #[test]
    fn different_models_at_equal_dimension_are_incompatible() {
        let nomic = ModelIdentity {
            name: "nomic-embed-text".into(),
            digest: Some("sha256:1111".into()),
            architecture: Some("nomic-bert".into()),
            parameter_size: Some("137M".into()),
            ..Default::default()
        };
        let bge = ModelIdentity {
            name: "bge-base-en-v1.5".into(),
            digest: Some("sha256:2222".into()),
            architecture: Some("bert".into()),
            parameter_size: Some("109M".into()),
            ..Default::default()
        };
        let r = compare_spaces(&nomic, Some(768), &bge, Some(768));
        assert_eq!(r.tier, Compatibility::Incompatible);
        assert!(!r.permits_read());
        assert!(!r.permits_write(true), "no flag may override incompatible");
        assert!(r.reason.contains("different models"), "{}", r.reason);
    }

    #[test]
    fn dimension_mismatch_is_incompatible_even_for_one_model_family() {
        let r = compare_spaces(
            &qwen("Q8_0", "sha256:aaaa1111"),
            Some(1024),
            &qwen("Q8_0", "sha256:aaaa1111"),
            Some(2560),
        );
        // Same digest wins first — identical weights truncated via MRL.
        assert_eq!(r.tier, Compatibility::Identical);

        // But different weights at different dims must fail.
        let r = compare_spaces(
            &qwen("Q8_0", "sha256:aaaa1111"),
            Some(1024),
            &qwen("Q4_K_M", "sha256:bbbb2222"),
            Some(2560),
        );
        assert_eq!(r.tier, Compatibility::Incompatible);
        assert!(r.reason.contains("dimension mismatch"), "{}", r.reason);
    }

    /// MERT audio vs qwen3 text: both 1024-dim, both Cosine, no shared identity.
    /// Refusing on insufficient information is the point — assuming compatible
    /// here is exactly how a collection gets silently poisoned.
    #[test]
    fn insufficient_identity_refuses_rather_than_assumes() {
        let r = compare_spaces(
            &ModelIdentity::unknown("mert-v1-330m"),
            Some(1024),
            &qwen("Q8_0", "sha256:aaaa1111"),
            Some(1024),
        );
        assert_eq!(r.tier, Compatibility::Incompatible);
        assert!(r.reason.contains("insufficient identity"), "{}", r.reason);
    }

    // ── Chunking, compared ───────────────────────────────────────

    fn identity() -> ChunkingIdentity {
        ChunkingIdentity {
            target_chunk_size: 6000,
            chunk_overlap: 50,
            max_chunk_bytes: 28672,
            tokenizer: "cl100k_base".to_string(),
            pack_target_bytes: 2048,
        }
    }

    #[test]
    fn identical_chunking_is_no_delta_at_all() {
        let d = identity().delta(&identity());
        assert!(!d.is_breaking());
        assert_eq!(d.ceiling, None);
        assert_eq!(d, ChunkingDelta::default());
    }

    /// Each granularity field on its own must break. Written as a loop rather
    /// than four tests so a field added later without a comparison shows up as
    /// an obviously-missing arm instead of passing silently.
    #[test]
    fn every_granularity_field_breaks_on_its_own() {
        let cases: Vec<(&str, ChunkingIdentity)> = vec![
            (
                "target_chunk_size",
                ChunkingIdentity {
                    target_chunk_size: 2000,
                    ..identity()
                },
            ),
            (
                "chunk_overlap",
                ChunkingIdentity {
                    chunk_overlap: 0,
                    ..identity()
                },
            ),
            (
                "tokenizer",
                ChunkingIdentity {
                    tokenizer: "bytes".to_string(),
                    ..identity()
                },
            ),
            (
                "pack_target_bytes",
                ChunkingIdentity {
                    pack_target_bytes: 4096,
                    ..identity()
                },
            ),
        ];

        for (field, current) in cases {
            let d = identity().delta(&current);
            assert!(d.is_breaking(), "{field} alone must be breaking");
            assert_eq!(d.granularity.len(), 1, "{field}: only one field moved");
            assert!(
                d.granularity[0].starts_with(field),
                "the message must name the field that moved: {}",
                d.granularity[0]
            );
            assert_eq!(d.ceiling, None, "{field} is not the ceiling");
        }
    }

    /// The ceiling is a SAFETY limit, not a granularity choice: it fires only on
    /// an already-oversized chunk and is clamped at ingest to whatever the local
    /// model accepts, so it moves between machines with nothing reconfigured.
    /// Reported, never refused — otherwise the same corpus would be
    /// un-ingestable from a second machine.
    #[test]
    fn a_ceiling_change_alone_is_reported_not_refused() {
        let current = ChunkingIdentity {
            max_chunk_bytes: 12000,
            ..identity()
        };
        let d = identity().delta(&current);
        assert!(!d.is_breaking(), "a ceiling move must not refuse the write");
        assert_eq!(d.ceiling, Some((28672, 12000)), "reported as (was, now)");
        assert!(d.granularity.is_empty());
    }

    /// Both kinds at once: the refusal wins and the ceiling is still carried, so
    /// the operator sees everything that moved in one message rather than
    /// discovering the second problem after fixing the first.
    #[test]
    fn a_mixed_delta_reports_both() {
        let current = ChunkingIdentity {
            target_chunk_size: 2000,
            max_chunk_bytes: 12000,
            ..identity()
        };
        let d = identity().delta(&current);
        assert!(d.is_breaking());
        assert_eq!(d.granularity.len(), 1);
        assert_eq!(d.ceiling, Some((28672, 12000)));
    }

    /// Direction is recorded, not just difference — `was -> now`, so an error
    /// message can be read without going to look up which side is which.
    #[test]
    fn the_delta_says_which_way_it_moved() {
        let current = ChunkingIdentity {
            target_chunk_size: 2000,
            ..identity()
        };
        assert_eq!(
            identity().delta(&current).granularity[0],
            "target_chunk_size: 6000 -> 2000"
        );
    }
}
