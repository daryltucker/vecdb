//! DOCS: docs/CONFIG.md
//! DOCS ARE GENERATED: the reference tables in docs/CONFIG.md are derived from
//! the `///` comments below. Edit the comment, then run:
//!     cargo run -p xtask -- gen-config-docs
/*
 * PURPOSE:
 *   Manages application configuration and profiles.
 *   Allows users to define connection details and behavior in a persistent file.
 *
 * REQUIREMENTS:
 *   User-specified:
 *   - "Config file with profiles" (User Prompt)
 *   - "Default profile" support
 *   - Law #1: Config IS Code
 *   - Ingestion Control (Chunk Size, Strategy)
 *
 *   Implementation-discovered:
 *   - Needs serialization (Serde)
 *   - Needs XDG compliance (~/.config/vecdb/config.toml)
 *
 * IMPLEMENTATION RULES:
 *   1. Use `toml` for storage
 *      Rationale: Human-readable, standard in Rust ecosystem.
 *
 *   2. Use `dirs` crate for path resolution
 *      Rationale: Cross-platform compatibility (Linux/macOS/Windows).
 *
 * USAGE:
 *   let config = Config::load()?;
 *   let profile = config.get_profile("default")?;
 *
 * SELF-HEALING INSTRUCTIONS:
 *   - If `config.toml` is missing: return Default config (don't crash).
 *   - If profile missing: Error gently.
 *
 * RELATED FILES:
 *   - vecdb-cli/src/main.rs - Consumes this config
 *
 * MAINTENANCE:
 *   Update when new backend options are needed (e.g., API keys).
 */

use anyhow::{Context, Result};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fs;
use std::path::PathBuf;

const DEFAULT_PROFILE_NAME: &str = "default";
const DEFAULT_QDRANT_URL: &str = "http://localhost:6334";
const DEFAULT_LOCAL_MODEL: &str = "all-minilm-l6-v2";
pub const DEFAULT_TARGET_CHUNK_SIZE: usize = 512;
const DEFAULT_CHUNK_OVERLAP: usize = 50;

/// Fallback Ollama context window when a profile does not state one.
///
/// Conservative on purpose: this is a guess about someone else's model, and the
/// cost of guessing high is oversized inputs. Set `num_ctx` explicitly and it is
/// used verbatim — a value the operator wrote is never derived over.
pub const DEFAULT_NUM_CTX: usize = 4096;

/// Inputs per `/api/embed` request when an Ollama embedder does not say.
///
/// Not derived from the context window: `/api/embed` processes each input in its
/// own context, so inputs are not concatenated and context math does not govern
/// batch size at all.
pub const DEFAULT_BATCH_INPUTS: usize = 8;

/// Rows per ONNX inference when a fastembed embedder does not say.
pub const DEFAULT_BATCH_ROWS: usize = 2;

/// Default number of search results when the caller does not specify a limit.
/// Single source of truth: the CLI flag default, the MCP tool schema default,
/// and `SearchParams::default()` all resolve here so they cannot drift apart.
pub const DEFAULT_SEARCH_LIMIT: u64 = 10;

const DEFAULT_STRATEGY: &str = "recursive";

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum QuantizationType {
    Scalar,
    Binary,
    None,
}

/// Which layer supplied a resolved configuration value.
///
/// Ordered most-specific first, matching resolution precedence.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum Source {
    /// `[collections.<name>]`
    Collection(String),
    /// `[profiles.<name>]`
    Profile(String),
    /// `[embedder.<name>]`
    Embedder(String),
    /// `[ingestion]` or another global table
    Global,
    /// Compiled-in fallback — nothing in config mentioned this key
    BuiltIn,
    /// Computed from another setting (e.g. a ceiling derived from `target_chunk_size`)
    Derived(String),
    /// A command-line flag overrode the config. Carries the flag name so
    /// `config show` can say `--backend` rather than pointing at a table the
    /// operator would then fail to find.
    Cli(&'static str),
}

impl std::fmt::Display for Source {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Source::Collection(c) => write!(f, "collections.{c}"),
            Source::Profile(p) => write!(f, "profiles.{p}"),
            Source::Embedder(e) => write!(f, "embedder.{e}"),
            Source::Global => write!(f, "global"),
            Source::BuiltIn => write!(f, "built-in default"),
            Source::Derived(from) => write!(f, "derived from {from}"),
            Source::Cli(flag) => write!(f, "{flag}"),
        }
    }
}

/// Per-run overrides for the two layers a profile would otherwise pin.
///
/// One field per config layer, deliberately: `[backend.*]` answers WHERE and
/// `[embedder.*]` answers WHAT+HOW, so `--backend` moves a run to another host
/// without touching the model, and `--embedder` swaps the model without
/// touching the store. WHICH is the profile itself, which is already a
/// parameter.
///
/// The alternative — defining a near-duplicate `[embedder.*]` per host — is how
/// two definitions of the same thing drift apart, which is the failure this
/// codebase has paid for more than once.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Overrides<'a> {
    /// `--embedder`: use this `[embedder.<name>]` instead of the profile's.
    pub embedder: Option<&'a str>,
    /// `--backend`: run the resolved embedder on this `[backend.<name>]`.
    pub backend: Option<&'a str>,
}

impl<'a> Overrides<'a> {
    pub fn is_empty(&self) -> bool {
        self.embedder.is_none() && self.backend.is_none()
    }
}

/// Everything one run needs, every layer already collapsed.
#[derive(Debug, Clone)]
pub struct Resolution {
    pub profile_name: String,
    pub embedder_name: String,
    pub backend_name: String,
    pub backend: Backend,
    pub embedder: EmbedderSpec,

    /// Which layer chose the embedder — a collection, a profile, or `--embedder`.
    pub embedder_source: Source,
    /// Which layer chose the backend — the embedder's own `backend =`, or `--backend`.
    pub backend_source: Source,

    pub qdrant_url: String,
    pub qdrant_api_key: Option<String>,
    pub collection: Option<String>,
    pub quantization: Option<QuantizationType>,

    pub target_chunk_size: Resolved<usize>,
    pub chunk_overlap: Resolved<usize>,
    pub max_chunk_bytes: Resolved<usize>,
    pub on_oversize: Resolved<OversizePolicy>,

    /// Ollama only. Meaningless for fastembed, where the sequence limit is
    /// baked into the model.
    pub num_ctx: Resolved<usize>,
    /// `batch_inputs` for Ollama, `batch_rows` for fastembed — different knobs,
    /// resolved into one slot because only one applies at a time.
    pub batch: Resolved<usize>,
    /// fastembed only.
    pub use_gpu: Resolved<bool>,
}

impl Resolution {
    pub fn is_ollama(&self) -> bool {
        self.backend.kind == BackendKind::Ollama
    }

    /// Endpoint for an Ollama backend. Empty for fastembed, which has none.
    pub fn ollama_url(&self) -> &str {
        self.backend.url.as_deref().unwrap_or("")
    }
}

/// Collection > profile > global, carrying the layer that won.
fn pick(
    coll: Option<usize>,
    prof: Option<usize>,
    global: usize,
    coll_key: Option<&str>,
    profile_name: &str,
) -> Resolved<usize> {
    match (coll, prof) {
        (Some(v), _) => Resolved::new(v, Source::Collection(coll_key.unwrap_or("").to_string())),
        (_, Some(v)) => Resolved::new(v, Source::Profile(profile_name.to_string())),
        _ => Resolved::new(global, Source::Global),
    }
}

/// A configuration value together with the layer that supplied it.
#[derive(Debug, Clone)]
pub struct Resolved<T> {
    pub value: T,
    pub source: Source,
}

impl<T> Resolved<T> {
    fn new(value: T, source: Source) -> Self {
        Self { value, source }
    }
}

/// What to do with a chunk that exceeds the resolved ceiling.
///
/// The invariant both options preserve: **never store a chunk whose metadata
/// claims more than its content contains.** Truncation breaks it — a chunk
/// labelled `main.rs:1-400` holding 60% of that range is a lie no reader can
/// detect. Splitting does not: `split_part: 2` with real line bounds is an
/// honest description of honest content.
///
/// Neither option aborts the run. An oversized chunk is a configuration problem,
/// and taking the whole ingest down for it helps nobody.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum OversizePolicy {
    /// Split until each part fits, labelling the parts. Content is preserved.
    #[default]
    Split,
    /// Refuse the insert and report it. The corpus stays exactly as precise as
    /// the source; the content is genuinely absent and the run summary says so.
    Skip,
}

impl std::fmt::Display for OversizePolicy {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            OversizePolicy::Split => write!(f, "split"),
            OversizePolicy::Skip => write!(f, "skip"),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct Config {
    /// Where models run. Connection details only, reusable by many embedders.
    ///
    /// Names are free-form; `kind` says what it is, so the name need not repeat
    /// it. Dots are allowed if quoted — `[backend."ollama.blade"]` — but a bare
    /// `[backend.ollama.blade]` is a *nested* TOML table and will not parse.
    #[serde(default)]
    pub backend: HashMap<String, Backend>,

    /// Which model, and how it is tuned. Each references a backend.
    ///
    /// This is the unit the storage layer already treats as primary: genesis
    /// records model name, digest, architecture, parameter size, quantization
    /// and dimension, and the space guard holds every write to that identity.
    /// Naming it here means config can finally refer to the thing the database
    /// tracks.
    #[serde(default)]
    pub embedder: HashMap<String, EmbedderSpec>,

    /// Which embedder to use, and which vector store to write to.
    #[serde(default)]
    pub profiles: HashMap<String, Profile>,

    /// Profile used when `--profile` is not given.
    #[serde(default = "default_profile_name")]
    pub default_profile: String,

    /// Collection-level overrides.
    #[serde(default)]
    pub collections: HashMap<String, CollectionConfig>,

    /// Simple aliases: short_name -> collection key
    #[serde(default)]
    pub collection_aliases: HashMap<String, String>,

    /// Chunking and discovery policy. Applies to every profile — it describes
    /// how documents are cut up, which is independent of which model embeds them.
    #[serde(default)]
    pub ingestion: IngestionConfig,

    /// Where fastembed caches downloaded models. Genuinely global — it is a
    /// disk location, not a property of any one embedder.
    #[serde(default = "default_fastembed_cache_path")]
    pub fastembed_cache_path: PathBuf,

    /// Keys to use for Smart Routing (Facet Auto-Detection).
    #[serde(default = "default_smart_routing_keys")]
    pub smart_routing_keys: Vec<String>,

    /// Server-side runtime tuning (idle eviction, watchdog cadence).
    /// Only consulted by `vecdb-server`; CLI commands ignore it.
    #[serde(default)]
    pub server: ServerConfig,
}

/// What kind of thing a backend is.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum BackendKind {
    /// An Ollama server, reachable over HTTP.
    Ollama,
    /// In-process fastembed / ONNX. No endpoint.
    Fastembed,
}

impl std::fmt::Display for BackendKind {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            BackendKind::Ollama => write!(f, "ollama"),
            BackendKind::Fastembed => write!(f, "fastembed"),
        }
    }
}

/// WHERE a model runs.
///
/// Deliberately carries no model and no tuning: one Ollama instance serves many
/// models, and the whole point of Ollama is not being pinned to one. Conflating
/// the two is what made a name like `ollama.blade.high` have to mean both "the
/// blade instance" and "the high-quality setup on it".
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct Backend {
    /// `"ollama"` or `"fastembed"`. Decides which embedder knobs apply.
    pub kind: BackendKind,
    /// Endpoint. Required for `ollama`, meaningless for `fastembed`.
    #[serde(default)]
    pub url: Option<String>,
    /// Bearer token, for an authenticated proxy in front of the endpoint.
    #[serde(default)]
    pub api_key: Option<String>,
    /// Accept invalid TLS certificates (staging / self-signed endpoints).
    #[serde(default)]
    pub accept_invalid_certs: bool,
}

/// WHAT model, and HOW it is tuned.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct EmbedderSpec {
    /// Name of the `[backend.*]` entry this runs on.
    pub backend: String,
    /// Model identifier, in the backend's namespace — an Ollama tag, or a
    /// fastembed model id.
    pub model: String,

    /// Context window to request, in tokens. **Ollama only.**
    ///
    /// This is the effective ceiling, and it is not what the model declares.
    /// Measured 2026-236: `qwen3-embedding:0.6b-q8_0` declares
    /// `context_length = 32768`, but with no `options` the server refused input
    /// at ~4086 tokens — Ollama's default `num_ctx` of 4096. The same input at
    /// ~12258 tokens succeeded with `num_ctx = 16384`. So `/api/embed` honours
    /// this, and `context_length` is only the maximum it will accept.
    ///
    /// Used exactly as written. Never derived over, never clamped.
    #[serde(default)]
    pub num_ctx: Option<usize>,

    /// Inputs per `/api/embed` request. **Ollama only.**
    ///
    /// Not the same knob as `batch_rows`: this is array length over HTTP, and it
    /// fails as a request timeout.
    #[serde(default)]
    pub batch_inputs: Option<usize>,

    /// Rows per ONNX inference. **fastembed only.**
    ///
    /// Not the same knob as `batch_inputs`: this is in-process, and it fails as
    /// an OOM.
    #[serde(default)]
    pub batch_rows: Option<usize>,

    /// Use GPU for local inference. **fastembed only.**
    #[serde(default)]
    pub use_gpu: Option<bool>,

    /// Matryoshka truncation target. Omit for the model's native width.
    ///
    /// Irreversible once a collection is written at it — the genesis record
    /// pins it and the space guard enforces it.
    #[serde(default)]
    pub dimension: Option<usize>,
}

/// Idle-eviction policy for the server's per-collection Core cache.
///
/// Hybrid policy informed by experiment E1 (2026-05-01):
///
/// * **Soft idle** — call `Embedder::release()` to drop the loaded model.
///   On `LocalEmbedder` this frees ~63% of VRAM for a tiny model and a
///   much higher percentage for large models (the residual ~80 MiB CUDA
///   context is roughly fixed). Reload on the next request takes ~200 ms.
///
/// * **Deep idle** — drop the cache entry entirely and (in stdio mode)
///   signal the main loop to exit. The MCP client respawns the subprocess
///   on next use, recovering the residual context. HTTP/daemon mode just
///   drops the cache entry; the daemon is meant to outlive idle.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct ServerConfig {
    /// After this many seconds without use, release the embedder's loaded model.
    /// Set to 0 to disable soft eviction.
    #[serde(default = "default_soft_idle_secs")]
    pub soft_idle_secs: u64,

    /// After this many seconds without use, drop the cache entry and (in stdio
    /// mode) exit the subprocess. Set to 0 to disable deep eviction.
    /// Should be greater than `soft_idle_secs`; if not, deep wins.
    #[serde(default = "default_deep_idle_secs")]
    pub deep_idle_secs: u64,

    /// How often the watchdog wakes up to evaluate idle entries.
    #[serde(default = "default_idle_check_interval_secs")]
    pub idle_check_interval_secs: u64,

    /// Master switch — if false, no watchdog is spawned.
    #[serde(default = "default_idle_eviction_enabled")]
    pub idle_eviction_enabled: bool,
}

fn default_soft_idle_secs() -> u64 {
    600
}
fn default_deep_idle_secs() -> u64 {
    3600
}
fn default_idle_check_interval_secs() -> u64 {
    60
}
fn default_idle_eviction_enabled() -> bool {
    true
}

impl Default for ServerConfig {
    fn default() -> Self {
        Self {
            soft_idle_secs: default_soft_idle_secs(),
            deep_idle_secs: default_deep_idle_secs(),
            idle_check_interval_secs: default_idle_check_interval_secs(),
            idle_eviction_enabled: default_idle_eviction_enabled(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct CollectionConfig {
    /// The actual Qdrant collection name.
    pub name: String,
    /// Free-text note shown by `vecdb list`.
    #[serde(default)]
    pub description: Option<String>,

    /// Profile to inherit from.
    #[serde(default)]
    pub profile: Option<String>,

    /// Override: use a different embedder for this collection.
    #[serde(default)]
    pub embedder: Option<String>,

    /// Override: a different Qdrant instance.
    #[serde(default)]
    pub qdrant_url: Option<String>,
    /// Override the profile's Qdrant API key.
    #[serde(default)]
    pub qdrant_api_key: Option<String>,

    /// Override the chunk target for this collection. Baked into the vectors at
    /// ingest — changing it later means a re-ingest.
    #[serde(default)]
    pub target_chunk_size: Option<usize>,
    /// Override the chunk overlap for this collection.
    #[serde(default)]
    pub chunk_overlap: Option<usize>,
    /// Override the byte ceiling above which a chunk is re-split.
    #[serde(default)]
    pub max_chunk_bytes: Option<usize>,

    /// Vector quantization for this collection: `"scalar"`, `"binary"` or
    /// `"none"`. Fixed when the collection is created.
    #[serde(default)]
    pub quantization: Option<QuantizationType>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct IngestionConfig {
    /// Chunking strategy for files with no structural parser: `"recursive"`
    /// (token-accurate splitting), `"semantic"` (alias for it), or `"simple"`
    /// (fixed-width). Rejected at load time if it is anything else.
    ///
    /// This does not govern source code. A file whose type vecq recognises is
    /// split along its AST by the parser, per element, and no chunker runs at
    /// all — so AST-aware chunking is automatic and is not something a strategy
    /// selects. The retired `"code_aware"` value promised exactly that and could
    /// not deliver it.
    #[serde(default = "default_strategy")]
    pub default_strategy: String,
    /// Target chunk size, counted in whatever `tokenizer` counts — **tokens**
    /// under the default `cl100k_base`, not bytes. Compare `max_chunk_bytes`.
    #[serde(default = "default_chunk_size")]
    pub target_chunk_size: usize,
    /// Hard limit for acceptable chunk size
    #[serde(default)]
    pub max_chunk_bytes: Option<usize>,
    /// How much adjacent chunks overlap, in the same unit as `target_chunk_size`.
    /// Overlap preserves context across a boundary at the cost of duplication.
    #[serde(default = "default_chunk_overlap")]
    pub chunk_overlap: usize,
    /// Consult `.gitignore` when walking. **Off, and stays off.**
    ///
    /// `.gitignore` is a build-artifact list, not an indexing policy, and the
    /// two disagree constantly. `.vectorignore` is the knob that governs
    /// indexing. This is an escape hatch for people driving the system who
    /// expect git semantics — it is never the default and never inferred.
    #[serde(default)]
    pub respect_gitignore: bool,
    /// What to do with a chunk that exceeds the resolved ceiling: `"split"` or
    /// `"skip"`. Defaults to `split`.
    #[serde(default)]
    pub on_oversize: Option<OversizePolicy>,

    /// Permit the embedder to silently cut chunks that exceed the model context.
    ///
    /// Off by default. A truncated embed succeeds in every observable way — right
    /// shape, clean upsert — while the tail of the chunk is simply gone, and only
    /// a re-ingest restores it. Refusing turns that into an oversized-chunk error
    /// that names the file, which is a problem you can act on.
    #[serde(default)]
    pub allow_embed_truncation: bool,
    /// What `target_chunk_size` and `chunk_overlap` are counted in:
    ///
    /// * `"cl100k_base"` (default) — GPT-4 tokens.
    /// * `"bytes"` — raw bytes, snapped to the nearest UTF-8 boundary. Fastest.
    ///   Was spelled `"char"`, which it never was.
    /// * anything else — characters, via the text splitter's `Characters` sizer.
    ///
    /// Whatever this counts, `max_chunk_bytes` still counts bytes.
    #[serde(default)]
    pub tokenizer: String,
    /// Per-glob overrides, e.g. `[ingestion.overrides."*.rs"]`. Lets source
    /// files chunk differently from prose without a separate collection.
    #[serde(default)]
    pub overrides: HashMap<String, IngestionOverride>,

    /// Path parsing rules for metadata extraction
    /// Path parsing rules for metadata extraction
    #[serde(default)]
    pub path_rules: Vec<PathRule>,

    /// Concurrency Limit: Max number of file processing tasks running in parallel
    #[serde(default = "default_concurrency")]
    pub max_concurrent_requests: usize,

    /// GPU Concurrency: Batch size for GPU embedding (None = auto calculate optimal size)
    #[serde(default)]
    pub gpu_batch_size: Option<usize>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct PathRule {
    /// Regex pattern with named capture groups (e.g. "users/(?P<user>\w+)/.*")
    pub pattern: String,
}

impl Default for IngestionConfig {
    fn default() -> Self {
        Self {
            default_strategy: default_strategy(),
            target_chunk_size: default_chunk_size(),
            max_chunk_bytes: None,
            chunk_overlap: default_chunk_overlap(),
            respect_gitignore: false,
            on_oversize: None,
            allow_embed_truncation: false,
            tokenizer: default_tokenizer(),
            overrides: HashMap::new(),
            path_rules: Vec::new(),
            max_concurrent_requests: default_concurrency(),
            gpu_batch_size: None, // Default into auto-sizing mode
        }
    }
}

fn default_concurrency() -> usize {
    4
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct IngestionOverride {
    pub strategy: Option<String>,
    pub target_chunk_size: Option<usize>,
    pub max_chunk_bytes: Option<usize>,
    pub chunk_overlap: Option<usize>,
}

fn default_profile_name() -> String {
    DEFAULT_PROFILE_NAME.to_string()
}

fn default_strategy() -> String {
    DEFAULT_STRATEGY.to_string()
}

fn default_chunk_size() -> usize {
    DEFAULT_TARGET_CHUNK_SIZE
}

fn default_chunk_overlap() -> usize {
    DEFAULT_CHUNK_OVERLAP
}

fn default_tokenizer() -> String {
    "cl100k_base".to_string()
}

/// Bytes assumed per unit of `target_chunk_size` when deriving a byte ceiling.
///
/// The two knobs are denominated differently and always have been:
/// `target_chunk_size` counts whatever the configured `tokenizer` counts — tokens for
/// the default `cl100k_base` — while `max_chunk_bytes` is compared against
/// `String::len()`, which is bytes. Converting between them needs a factor, and
/// that factor decides whether structural chunking survives: set it too low and
/// every full-size chunk trips the ceiling and is re-split by `FixedWidthChunker`,
/// discarding the AST boundaries the structural chunker just established.
///
/// Measured against this repo's own Rust sources, `cl100k_base` averages ~5.2
/// bytes per token — a 6144-token chunk weighs ~31.9 KB. The previous factor of
/// 4 (24.6 KB) sat below that, so it fired on essentially every full-size chunk
/// instead of only on genuine outliers. 6 clears the measurement with headroom.
pub const BYTES_PER_CHUNK_UNIT: usize = 6;

/// Keys from the pre-three-layer schema, and what replaced them.
///
/// Serde ignores unknown fields, so a config written against the old shape
/// loads clean and every one of these is silently dropped. Measured 2026-238:
/// a profile asking for `embedder_type = "ollama"` with `nomic-embed-text`
/// resolved to fastembed/all-minilm-l6-v2 and said nothing — a different model
/// than the operator configured, writing into their collection.
///
/// Silence is the whole problem, so these are refused by name.
const RETIRED_PROFILE_KEYS: &[(&str, &str)] = &[
    ("embedder_type", "`kind` on a [backend.<name>]"),
    ("embedding_model", "`model` on an [embedder.<name>]"),
    ("local_embedding_model", "`model` on an [embedder.<name>]"),
    ("ollama_url", "`url` on a [backend.<name>]"),
    ("ollama_api_key", "`api_key` on a [backend.<name>]"),
    ("local_use_gpu", "`use_gpu` on an [embedder.<name>]"),
    (
        "gpu_batch_size",
        "`batch_inputs` (ollama) or `batch_rows` (fastembed)",
    ),
    ("chunk_size", "`target_chunk_size`"),
    (
        "max_chunk_size",
        "`max_chunk_bytes` (and it counts BYTES, not tokens)",
    ),
];

/// Refuse a config written against the retired schema, naming each key.
///
/// Deliberately not a silent migration: the old `[profiles.*]` conflated WHERE,
/// WHAT and HOW, so several keys have no single mechanical translation —
/// `embedder_type` becomes a backend that other embedders may want to share.
/// Guessing would produce a config the operator never reviewed.
fn check_retired_keys(path: &std::path::Path) -> Result<()> {
    let Ok(content) = fs::read_to_string(path) else {
        return Ok(()); // unreadable is Figment's problem to report, not ours
    };
    // `content.parse::<toml::Value>()` parses a single VALUE, not a document,
    // and fails on any real config file — silently, into the `else` arm below.
    // Deserializing is what reads a document.
    let Ok(raw) = toml::from_str::<toml::Value>(&content) else {
        return Ok(()); // malformed is Figment's problem too
    };

    let mut found: Vec<String> = Vec::new();
    if let Some(profiles) = raw.get("profiles").and_then(|p| p.as_table()) {
        for (profile_name, body) in profiles {
            let Some(table) = body.as_table() else {
                continue;
            };
            for (key, replacement) in RETIRED_PROFILE_KEYS {
                if table.contains_key(*key) {
                    found.push(format!(
                        "  profiles.{profile_name}.{key}
      -> {replacement}"
                    ));
                }
            }
        }
    }

    if found.is_empty() {
        return Ok(());
    }

    anyhow::bail!(
        "{} uses configuration keys that no longer exist.\n\n\
         They are not ignored quietly, because they would change which model runs:\n\n\
         {}\n\n\
         The schema is three layers now:\n  \
         [backend.<name>]   WHERE a model runs\n  \
         [embedder.<name>]  WHAT model, and HOW it is tuned\n  \
         [profiles.<name>]  WHICH embedder and which store\n\n\
         See docs/CONFIG.md for the migration.",
        path.display(),
        found.join("\n")
    )
}

/// Headroom between `target_chunk_size` and the model's context window.
///
/// `target_chunk_size` is counted with `cl100k_base`; the embedding model uses its own
/// tokenizer. The two disagree by a few percent on ordinary text and more on
/// code, identifiers and non-English. This is the margin that disagreement needs
/// — it is not a fudge factor for a formula, because there is no formula here:
/// the ceiling is the model's declared `context_length`, and this only stops us
/// sailing straight at it.
pub const TOKENIZER_MARGIN: f64 = 0.15;

/// Whether a `target_chunk_size` can work against a context window, and how comfortably.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ChunkFit {
    /// Fits with margin to spare.
    Ok,
    /// Fits literally, but inside the tokenizer margin — expect the oversize
    /// policy to fire on some chunks.
    Tight,
    /// Cannot work: the target meets or exceeds the window, so the policy would
    /// fire on essentially every full-size chunk.
    Impossible,
}

/// Compare a chunk target against a context window.
///
/// Deliberately does not "correct" anything. A `num_ctx` the operator wrote is
/// used exactly as written; this reports whether the *target* fits inside it.
pub fn check_chunk_fit(target_chunk_size: usize, num_ctx: usize) -> ChunkFit {
    if target_chunk_size >= num_ctx {
        return ChunkFit::Impossible;
    }
    if (target_chunk_size as f64) * (1.0 + TOKENIZER_MARGIN) > num_ctx as f64 {
        return ChunkFit::Tight;
    }
    ChunkFit::Ok
}

/// Chunking strategies that resolve to a real chunker.
pub const STRATEGIES: [&str; 3] = ["recursive", "semantic", "simple"];

/// Reject a strategy name that resolves to nothing.
///
/// `Factory::get` falls through to `RecursiveChunker` for any unrecognised
/// value, so a typo silently changed how an entire corpus was chunked and said
/// nothing. This runs at load, before any file is read.
///
/// A free function rather than inline in `Config::load` so it can be tested
/// without setting `VECDB_CONFIG` — tests in one binary share a process, and two
/// of them racing on that variable is its own flaky-test story.
pub fn validate_strategy(strategy: &str) -> Result<()> {
    if STRATEGIES.contains(&strategy) {
        return Ok(());
    }

    // `code_aware` needs its own message. It was a documented option, with a
    // worked example in docs/CONFIG.md, so anyone reaching this followed the
    // docs — "unknown strategy" would read as a typo and send them looking for
    // the correct spelling of something that no longer exists.
    if strategy == "code_aware" {
        anyhow::bail!(
            "ingestion.default_strategy = \"code_aware\" is no longer a strategy.\n\n  \
             It selected a chunker that could never run: a file vecdb can parse is split \n  \
             along its AST by the parser itself, per vecq element, and no chunker is \n  \
             consulted. AST-aware chunking is automatic for every supported language — \n  \
             there is nothing to opt into.\n\n  \
             fix: remove the line, or set one of: {}",
            STRATEGIES.join(", ")
        );
    }

    anyhow::bail!(
        "ingestion.default_strategy = \"{strategy}\" is not a known strategy.\n\n  \
         known strategies: {}\n\n  \
         note: this governs files with no structural parser. Source code is chunked \n  \
         along its AST by the parser regardless of this setting.",
        STRATEGIES.join(", ")
    );
}

/// Byte ceiling to use when `max_chunk_bytes` is not configured.
///
/// A ceiling below the target it is meant to protect is not a safety net, it is
/// a second chunker. Deriving it from `target_chunk_size` keeps the two in the same
/// relationship no matter how `target_chunk_size` is tuned.
pub fn default_max_chunk_bytes(target_chunk_size: usize) -> usize {
    target_chunk_size.saturating_mul(BYTES_PER_CHUNK_UNIT)
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct Profile {
    /// Name of the `[embedder.*]` entry to use.
    pub embedder: String,

    /// Qdrant endpoint for collections under this profile.
    #[serde(default = "default_qdrant_url")]
    pub qdrant_url: String,
    /// API key for Qdrant authentication.
    #[serde(default)]
    pub qdrant_api_key: Option<String>,

    /// Default collection when `-c` is not given.
    #[serde(default)]
    pub default_collection_name: Option<String>,

    /// Default quantization for collections created under this profile.
    #[serde(default)]
    pub quantization: Option<QuantizationType>,

    /// Override `[ingestion].target_chunk_size` for this profile. Counted in whatever
    /// `tokenizer` counts — tokens under the default `cl100k_base`.
    #[serde(default)]
    pub target_chunk_size: Option<usize>,
    /// Override the byte ceiling above which a chunk is re-split. Unset derives
    /// from `target_chunk_size`; it must never sit below it.
    #[serde(default)]
    pub max_chunk_bytes: Option<usize>,
    /// Override `[ingestion].chunk_overlap` for this profile.
    #[serde(default)]
    pub chunk_overlap: Option<usize>,

    /// The name this profile was resolved under.
    #[serde(skip)]
    pub resolved_profile_name: String,
}

fn default_qdrant_url() -> String {
    DEFAULT_QDRANT_URL.to_string()
}

fn default_fastembed_cache_path() -> PathBuf {
    let mut path = dirs::config_dir().unwrap_or_else(|| PathBuf::from("."));
    path.push("vecdb");
    path.push("fastembed_cache");
    path
}

fn default_smart_routing_keys() -> Vec<String> {
    vec![
        "source_type".to_string(),
        "language".to_string(),
        // Users can add "version", "cuda", "platform" in config.toml
    ]
}

impl Default for Config {
    /// Zero-config default: one local backend, one small model, one profile.
    ///
    /// Every layer is present even in the trivial case, so the shape a user sees
    /// in `vecdb config show` is the same shape they will edit.
    fn default() -> Self {
        let mut backend = HashMap::new();
        backend.insert(
            "local".to_string(),
            Backend {
                kind: BackendKind::Fastembed,
                url: None,
                api_key: None,
                accept_invalid_certs: false,
            },
        );

        let mut embedder = HashMap::new();
        embedder.insert(
            "default".to_string(),
            EmbedderSpec {
                backend: "local".to_string(),
                model: DEFAULT_LOCAL_MODEL.to_string(),
                num_ctx: None,
                batch_inputs: None,
                batch_rows: None,
                use_gpu: None,
                dimension: None,
            },
        );

        let mut profiles = HashMap::new();
        profiles.insert(
            DEFAULT_PROFILE_NAME.to_string(),
            Profile {
                embedder: "default".to_string(),
                qdrant_url: std::env::var("QDRANT_URL")
                    .unwrap_or_else(|_| DEFAULT_QDRANT_URL.to_string()),
                qdrant_api_key: None,
                default_collection_name: None,
                quantization: Some(QuantizationType::None),
                target_chunk_size: None,
                max_chunk_bytes: None,
                chunk_overlap: None,
                resolved_profile_name: DEFAULT_PROFILE_NAME.to_string(),
            },
        );

        Self {
            backend,
            embedder,
            profiles,
            default_profile: DEFAULT_PROFILE_NAME.to_string(),
            collections: HashMap::new(),
            collection_aliases: HashMap::new(),
            ingestion: IngestionConfig::default(),
            fastembed_cache_path: default_fastembed_cache_path(),
            smart_routing_keys: default_smart_routing_keys(),
            server: ServerConfig::default(),
        }
    }
}

impl Config {
    /// Everything one run needs, with every layer already collapsed.
    ///
    /// `Core::new` takes this instead of seventeen positional arguments, and
    /// `vecdb config show` prints it. One resolution, one truth: the thing that
    /// reports what will happen is the thing that makes it happen.
    /// Resolve with no per-run overrides. The common path.
    pub fn resolve(
        &self,
        requested_profile: Option<&str>,
        requested_collection: Option<&str>,
    ) -> Result<Resolution> {
        self.resolve_with(
            requested_profile,
            requested_collection,
            Overrides::default(),
        )
    }

    /// Resolve, letting `--embedder` / `--backend` override the layer a profile
    /// would otherwise pin.
    pub fn resolve_with(
        &self,
        requested_profile: Option<&str>,
        requested_collection: Option<&str>,
        overrides: Overrides<'_>,
    ) -> Result<Resolution> {
        // ── Collection ───────────────────────────────────────────
        let mut final_c_name = requested_collection;
        let c_config = if let Some(mut c_name) = requested_collection {
            if let Some(real_key) = self.collection_aliases.get(c_name) {
                c_name = real_key.as_str();
                final_c_name = Some(c_name);
            }
            self.collections
                .get(c_name)
                .or_else(|| self.collections.values().find(|c| c.name == c_name))
        } else {
            None
        };

        // ── Profile: CLI flag > collection's profile > default ───
        let profile_name = requested_profile
            .or_else(|| c_config.and_then(|c| c.profile.as_deref()))
            .unwrap_or(&self.default_profile);

        let profile = self.profiles.get(profile_name).ok_or_else(|| {
            anyhow::anyhow!(
                "profile '{profile_name}' not found.\n\n  known profiles: {}",
                self.known(self.profiles.keys())
            )
        })?;

        // ── Embedder: --embedder > collection override > profile ─
        let (embedder_name, embedder_source) = match overrides.embedder {
            Some(e) => (e, Source::Cli("--embedder")),
            None => match c_config.and_then(|c| c.embedder.as_deref()) {
                Some(e) => (
                    e,
                    Source::Collection(final_c_name.unwrap_or_default().to_string()),
                ),
                None => (
                    profile.embedder.as_str(),
                    Source::Profile(profile_name.to_string()),
                ),
            },
        };

        let embedder = self.embedder.get(embedder_name).ok_or_else(|| {
            anyhow::anyhow!(
                "embedder '{embedder_name}' not found (referenced by {embedder_source}).\n\n                   known embedders: {}",
                self.known(self.embedder.keys())
            )
        })?;

        // ── Backend: --backend > the embedder's own `backend =` ──
        let (backend_name, backend_source) = match overrides.backend {
            Some(b) => (b, Source::Cli("--backend")),
            None => (
                embedder.backend.as_str(),
                Source::Embedder(embedder_name.to_string()),
            ),
        };

        let backend = self.backend.get(backend_name).ok_or_else(|| {
            let referenced_by = match backend_source {
                Source::Cli(flag) => format!("requested with {flag}"),
                _ => format!("referenced by embedder '{embedder_name}'"),
            };
            anyhow::anyhow!(
                "backend '{backend_name}' not found ({referenced_by}).\n\n                   known backends: {}\n\n                   note: `[backend.a.b]` is a nested TOML table and will not parse as a \n                   backend name. Quote it instead: `[backend.\"a.b\"]`.",
                self.known(self.backend.keys())
            )
        })?;

        // `--backend` relocates a run; it must not silently retune it.
        //
        // Only the knobs matching a backend's `kind` are consulted, so moving an
        // Ollama-tuned embedder onto a fastembed backend would drop `num_ctx`
        // and `batch_inputs` on the floor and embed at defaults the operator
        // never chose — a wrong answer that still returns 200.
        if let (Some(_), Some(declared)) = (overrides.backend, self.backend.get(&embedder.backend))
        {
            if declared.kind != backend.kind {
                anyhow::bail!(
                    "--backend {backend_name} is kind = \"{}\", but embedder '{embedder_name}' is tuned for \
                     kind = \"{}\" (backend '{}').\n\n  \
                     Only the knobs matching a backend's kind are read, so this would discard that tuning \
                     silently.\n  \
                     Define an embedder for {} and select it with --embedder instead.",
                    backend.kind,
                    declared.kind,
                    embedder.backend,
                    backend.kind,
                );
            }
        }

        // An Ollama backend without a URL is unusable, and the failure would
        // otherwise surface as a connection error to a default endpoint the
        // operator never configured.
        if backend.kind == BackendKind::Ollama && backend.url.is_none() {
            anyhow::bail!("backend '{backend_name}' is kind = \"ollama\" but has no `url`.");
        }

        // ── Qdrant: collection > profile ─────────────────────────
        let qdrant_url = c_config
            .and_then(|c| c.qdrant_url.clone())
            .unwrap_or_else(|| profile.qdrant_url.clone());
        let qdrant_api_key = c_config
            .and_then(|c| c.qdrant_api_key.clone())
            .or_else(|| profile.qdrant_api_key.clone());

        let collection = c_config
            .map(|c| c.name.clone())
            .or_else(|| final_c_name.map(|s| s.to_string()))
            .or_else(|| profile.default_collection_name.clone());

        let quantization = c_config
            .and_then(|c| c.quantization.clone())
            .or_else(|| profile.quantization.clone());

        // ── Chunking: collection > profile > [ingestion] ─────────
        let coll_key = final_c_name;
        let target_chunk_size = pick(
            c_config.and_then(|c| c.target_chunk_size),
            profile.target_chunk_size,
            self.ingestion.target_chunk_size,
            coll_key,
            profile_name,
        );
        let chunk_overlap = pick(
            c_config.and_then(|c| c.chunk_overlap),
            profile.chunk_overlap,
            self.ingestion.chunk_overlap,
            coll_key,
            profile_name,
        );
        let max_chunk_bytes = match (
            c_config.and_then(|c| c.max_chunk_bytes),
            profile.max_chunk_bytes,
            self.ingestion.max_chunk_bytes,
        ) {
            (Some(v), _, _) => {
                Resolved::new(v, Source::Collection(coll_key.unwrap_or("").to_string()))
            }
            (_, Some(v), _) => Resolved::new(v, Source::Profile(profile_name.to_string())),
            (_, _, Some(v)) => Resolved::new(v, Source::Global),
            _ => Resolved::new(
                default_max_chunk_bytes(target_chunk_size.value),
                Source::Derived("target_chunk_size".to_string()),
            ),
        };

        // ── Embedder tuning ──────────────────────────────────────
        let num_ctx = match embedder.num_ctx {
            Some(v) => Resolved::new(v, Source::Embedder(embedder_name.to_string())),
            None => Resolved::new(DEFAULT_NUM_CTX, Source::BuiltIn),
        };
        let batch = match backend.kind {
            BackendKind::Ollama => match embedder.batch_inputs {
                Some(v) => Resolved::new(v, Source::Embedder(embedder_name.to_string())),
                None => Resolved::new(DEFAULT_BATCH_INPUTS, Source::BuiltIn),
            },
            BackendKind::Fastembed => match embedder.batch_rows {
                Some(v) => Resolved::new(v, Source::Embedder(embedder_name.to_string())),
                None => Resolved::new(DEFAULT_BATCH_ROWS, Source::BuiltIn),
            },
        };
        let use_gpu = match embedder.use_gpu {
            Some(v) => Resolved::new(v, Source::Embedder(embedder_name.to_string())),
            None => Resolved::new(false, Source::BuiltIn),
        };

        Ok(Resolution {
            profile_name: profile_name.to_string(),
            embedder_name: embedder_name.to_string(),
            backend_name: backend_name.to_string(),
            backend: backend.clone(),
            embedder: embedder.clone(),
            embedder_source,
            backend_source,
            qdrant_url,
            qdrant_api_key,
            collection,
            quantization,
            target_chunk_size,
            chunk_overlap,
            max_chunk_bytes,
            on_oversize: self.resolve_oversize_policy(),
            num_ctx,
            batch,
            use_gpu,
        })
    }

    fn known<'a>(&self, keys: impl Iterator<Item = &'a String>) -> String {
        let mut v: Vec<&str> = keys.map(|k| k.as_str()).collect();
        v.sort_unstable();
        if v.is_empty() {
            "(none configured)".to_string()
        } else {
            v.join(", ")
        }
    }

    /// Helper to get effective chunk size if a collection overrides it
    /// Where a resolved setting actually came from.
    ///
    /// Carried alongside the value rather than reconstructed for display. A
    /// What to do with a chunk that exceeds the resolved ceiling.
    pub fn resolve_oversize_policy(&self) -> Resolved<OversizePolicy> {
        match self.ingestion.on_oversize {
            Some(p) => Resolved::new(p, Source::Global),
            None => Resolved::new(OversizePolicy::default(), Source::BuiltIn),
        }
    }

    pub fn load() -> Result<Self> {
        let config_path = Self::get_path()?;

        if !config_path.exists() {
            // Write default config
            let default_config = Config::default();
            // Ensure dir exists
            if let Some(parent) = config_path.parent() {
                fs::create_dir_all(parent)?;
            }
            let toml_str = toml::to_string_pretty(&default_config)?;
            fs::write(&config_path, toml_str)?;
            // We continue to load via Figment to ensure consistent behavior
        }

        use figment::{
            providers::{Env, Format, Serialized, Toml},
            Figment,
        };

        let mut figment = Figment::new()
            .merge(Serialized::defaults(Config::default()))
            .merge(Toml::file(&config_path));

        check_retired_keys(&config_path)?;

        // Check for project-local .vecdb/config.toml and merge it on top if it exists
        if let Ok(cwd) = std::env::current_dir() {
            let local_config_path = cwd.join(".vecdb").join("config.toml");
            if local_config_path.exists() {
                check_retired_keys(&local_config_path)?;
                figment = figment.merge(Toml::file(&local_config_path));
            }
        }

        let config: Config = figment
            .merge(Env::prefixed("VECDB_").split("__"))
            .extract()
            .context("Failed to load configuration via Figment")?;

        Self::validate_graph(&config)?;

        Ok(config)
    }

    /// Validate every reference in the config graph.
    ///
    /// Separate from `load()` and taking `&Config` so it is testable without a
    /// config file or `VECDB_CONFIG` — tests in one binary share a process, and
    /// driving validation through the env var makes them race each other.
    pub fn validate_graph(config: &Config) -> Result<()> {
        // Validate the reference graph at load time.
        //
        // A typo in `embedder = "..."` or `backend = "..."` is a static error and
        // there is no reason to discover it at first use, halfway through a
        // command, with an error about something else. Names are listed so the
        // fix is obvious.
        for (name, embedder) in &config.embedder {
            if !config.backend.contains_key(&embedder.backend) {
                anyhow::bail!(
                    "embedder '{name}' references backend '{}', which is not defined.\n\n  \
                 known backends: {}\n\n  \
                 note: `[backend.a.b]` is a nested TOML table, not a backend named \n  \
                 \"a.b\". Quote it: `[backend.\"a.b\"]`.",
                    embedder.backend,
                    config.known(config.backend.keys())
                );
            }
        }
        for (name, profile) in &config.profiles {
            if !config.embedder.contains_key(&profile.embedder) {
                anyhow::bail!(
                    "profile '{name}' references embedder '{}', which is not defined.\n\n  \
                 known embedders: {}",
                    profile.embedder,
                    config.known(config.embedder.keys())
                );
            }
        }
        validate_strategy(&config.ingestion.default_strategy)?;

        // Aliases were the one reference type not checked here, and both ways
        // they can be wrong are silent and route writes to the wrong place.
        for (alias, target) in &config.collection_aliases {
            // `resolve()` consults the alias table *before* [collections.*], so
            // an alias sharing a name with a real collection shadows it — every
            // `-c <name>` silently lands on the alias target instead, with the
            // canonical entry still present in config and apparently in use.
            let shadows = config.collections.contains_key(alias)
                || config.collections.values().any(|c| &c.name == alias);
            if shadows {
                anyhow::bail!(
                    "alias '{alias}' has the same name as a collection, and would shadow it.\n\n  \
                 `-c {alias}` would resolve to '{target}' while `[collections.{alias}]` \n  \
                 stays in config looking authoritative. Rename one of them."
                );
            }

            // An alias pointing at nothing falls through to the requested name
            // used verbatim, which creates a collection named after the typo
            // rather than reporting one.
            let resolves = config.collections.contains_key(target)
                || config.collections.values().any(|c| &c.name == target);
            if !resolves {
                anyhow::bail!(
                    "alias '{alias}' points at collection '{target}', which is not defined.\n\n  \
                 known collections: {}",
                    config.known(config.collections.keys())
                );
            }
        }

        for (name, coll) in &config.collections {
            if let Some(e) = &coll.embedder {
                if !config.embedder.contains_key(e) {
                    anyhow::bail!(
                    "collection '{name}' references embedder '{e}', which is not defined.\n\n  \
                     known embedders: {}",
                    config.known(config.embedder.keys())
                );
                }
            }
        }

        Ok(())
    }

    /// Resolve config path: ~/.config/vecdb/config.toml
    /// Respects VECDB_CONFIG environment variable if set.
    pub fn get_path() -> Result<PathBuf> {
        if let Ok(path) = std::env::var("VECDB_CONFIG") {
            return Ok(PathBuf::from(path));
        }

        let mut path = dirs::config_dir()
            .ok_or_else(|| anyhow::anyhow!("Could not determine config directory"))?;
        path.push("vecdb");
        path.push("config.toml");
        Ok(path)
    }

    pub fn save(&self) -> Result<()> {
        let path = Self::get_path()?;
        let content = toml::to_string_pretty(self).context("Failed to serialize config")?;
        fs::write(&path, content).context("Failed to write config file")?;
        Ok(())
    }

    /// The raw profile, without collection overrides or embedder resolution.
    /// Prefer `resolve()` — this exists for callers that only need the name.
    pub fn get_profile(&self, name: Option<&str>) -> Result<&Profile> {
        let profile_name = name.unwrap_or(&self.default_profile);
        self.profiles
            .get(profile_name)
            .ok_or_else(|| anyhow::anyhow!("Profile '{}' not found in configuration", profile_name))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A config exercising all three layers: two embedders sharing one backend,
    /// which is the case the old two-layer shape could not express.
    fn three_layer() -> Config {
        let mut config = Config::default();

        config.backend.insert(
            "blade".to_string(),
            Backend {
                kind: BackendKind::Ollama,
                url: Some("http://ollama-a.example.com:11434".to_string()),
                api_key: None,
                accept_invalid_certs: false,
            },
        );
        for (name, model, ctx) in [
            ("baby_qwen", "qwen3-embedding:0.6b-q8_0", 16384usize),
            ("big_qwen", "qwen3-embedding:4b-q8_0", 8192),
        ] {
            config.embedder.insert(
                name.to_string(),
                EmbedderSpec {
                    backend: "blade".to_string(),
                    model: model.to_string(),
                    num_ctx: Some(ctx),
                    batch_inputs: Some(48),
                    batch_rows: None,
                    use_gpu: None,
                    dimension: None,
                },
            );
        }
        config.profiles.insert(
            "remote".to_string(),
            Profile {
                embedder: "baby_qwen".to_string(),
                qdrant_url: "http://localhost:6334".to_string(),
                qdrant_api_key: None,
                default_collection_name: None,
                quantization: None,
                target_chunk_size: None,
                max_chunk_bytes: None,
                chunk_overlap: None,
                resolved_profile_name: "remote".to_string(),
            },
        );
        config
    }

    /// Two embedders, one backend. The thing the two-layer shape could not say.
    #[test]
    fn many_embedders_share_one_backend() {
        let mut config = three_layer();
        config.collections.insert(
            "docs".to_string(),
            CollectionConfig {
                name: "docs".to_string(),
                description: None,
                profile: Some("remote".to_string()),
                embedder: Some("big_qwen".to_string()),
                qdrant_url: None,
                qdrant_api_key: None,
                target_chunk_size: None,
                chunk_overlap: None,
                max_chunk_bytes: None,
                quantization: None,
            },
        );

        let a = config.resolve(Some("remote"), None).unwrap();
        let b = config.resolve(None, Some("docs")).unwrap();

        assert_eq!(a.embedder_name, "baby_qwen");
        assert_eq!(b.embedder_name, "big_qwen");
        assert_eq!(a.backend_name, b.backend_name, "same instance");
        assert_eq!(a.embedder.model, "qwen3-embedding:0.6b-q8_0");
        assert_eq!(b.embedder.model, "qwen3-embedding:4b-q8_0");
    }

    // ── Per-run overrides: one flag per layer ────────────────────
    //
    // The motivating case, measured 2026-237: two repos ingesting into one
    // collection, one on jetson and one on blade, because a single embed host
    // becomes everyone's queue. Both hold digest ac6da0dfba84, so the vectors
    // land in the same space — `--backend` is what says "this embedder,
    // elsewhere" without duplicating the `[embedder.*]` table per host.

    /// `--backend` moves WHERE a run executes and nothing else. Same model,
    /// same `num_ctx`, same batch — otherwise the two halves of a parallel
    /// ingest would not be writing comparable vectors.
    #[test]
    fn backend_override_relocates_without_retuning() {
        let mut config = three_layer();
        config.backend.insert(
            "jetson".to_string(),
            Backend {
                kind: BackendKind::Ollama,
                url: Some("http://ollama-b.example.com:11434".to_string()),
                api_key: None,
                accept_invalid_certs: false,
            },
        );

        let home = config.resolve(Some("remote"), None).unwrap();
        let away = config
            .resolve_with(
                Some("remote"),
                None,
                Overrides {
                    backend: Some("jetson"),
                    ..Default::default()
                },
            )
            .unwrap();

        assert_eq!(away.backend_name, "jetson");
        assert_eq!(away.ollama_url(), "http://ollama-b.example.com:11434");
        assert_eq!(away.backend_source, Source::Cli("--backend"));

        // Everything that decides what a vector *is* must be untouched.
        assert_eq!(away.embedder_name, home.embedder_name);
        assert_eq!(away.embedder.model, home.embedder.model);
        assert_eq!(away.num_ctx.value, home.num_ctx.value);
        assert_eq!(away.batch.value, home.batch.value);
        assert_eq!(away.qdrant_url, home.qdrant_url, "same store");
    }

    /// Only the knobs matching a backend's `kind` are read, so crossing kinds
    /// would drop `num_ctx`/`batch_inputs` and embed at defaults nobody chose.
    /// That is a wrong answer that still succeeds, so it must be refused.
    #[test]
    fn backend_override_refuses_to_cross_kind() {
        let mut config = three_layer();
        config.backend.insert(
            "local".to_string(),
            Backend {
                kind: BackendKind::Fastembed,
                url: None,
                api_key: None,
                accept_invalid_certs: false,
            },
        );

        let err = config
            .resolve_with(
                Some("remote"),
                None,
                Overrides {
                    backend: Some("local"),
                    ..Default::default()
                },
            )
            .unwrap_err()
            .to_string();

        assert!(err.contains("fastembed"), "{err}");
        assert!(err.contains("ollama"), "{err}");
        assert!(err.contains("--embedder"), "must name the way out: {err}");
    }

    /// An unknown `--backend` must blame the flag, not an `[embedder.*]` table
    /// the operator would then go looking at and find correct.
    #[test]
    fn unknown_backend_override_blames_the_flag() {
        let config = three_layer();
        let err = config
            .resolve_with(
                Some("remote"),
                None,
                Overrides {
                    backend: Some("typo"),
                    ..Default::default()
                },
            )
            .unwrap_err()
            .to_string();

        assert!(err.contains("--backend"), "{err}");
        assert!(err.contains("blade"), "should list what exists: {err}");
    }

    /// `--embedder` outranks a collection's own override, which outranks the
    /// profile. A flag the operator typed is the most specific thing there is.
    #[test]
    fn embedder_override_outranks_collection_and_profile() {
        let mut config = three_layer();
        config.collections.insert(
            "docs".to_string(),
            CollectionConfig {
                name: "docs".to_string(),
                description: None,
                profile: Some("remote".to_string()),
                embedder: Some("big_qwen".to_string()),
                qdrant_url: None,
                qdrant_api_key: None,
                target_chunk_size: None,
                chunk_overlap: None,
                max_chunk_bytes: None,
                quantization: None,
            },
        );

        let without = config.resolve(None, Some("docs")).unwrap();
        assert_eq!(
            without.embedder_name, "big_qwen",
            "collection wins by default"
        );

        let with = config
            .resolve_with(
                None,
                Some("docs"),
                Overrides {
                    embedder: Some("baby_qwen"),
                    ..Default::default()
                },
            )
            .unwrap();
        assert_eq!(with.embedder_name, "baby_qwen");
        assert_eq!(with.embedder_source, Source::Cli("--embedder"));
        // num_ctx must follow the embedder that actually won, not the one the
        // collection named — this is the coupling that made the old flat config
        // report one model's tuning while running another's.
        assert_eq!(with.num_ctx.value, 16384);
    }

    /// No overrides must be byte-identical to the plain path, or every existing
    /// call site silently changes meaning.
    #[test]
    fn empty_overrides_change_nothing() {
        let config = three_layer();
        let plain = config.resolve(Some("remote"), None).unwrap();
        let empty = config
            .resolve_with(Some("remote"), None, Overrides::default())
            .unwrap();

        assert_eq!(plain.embedder_name, empty.embedder_name);
        assert_eq!(plain.backend_name, empty.backend_name);
        assert_eq!(plain.num_ctx.value, empty.num_ctx.value);
        assert_eq!(
            empty.backend_source,
            Source::Embedder("baby_qwen".to_string()),
            "unflagged runs still blame the embedder, not a flag"
        );
    }

    // ── Upgrade path ─────────────────────────────────────────────

    fn write_tmp(name: &str, body: &str) -> std::path::PathBuf {
        let path = std::env::temp_dir().join(format!("vecdb-retired-{name}.toml"));
        std::fs::write(&path, body).unwrap();
        path
    }

    /// Serde ignores unknown fields, so the old schema loads clean and every
    /// retired key is dropped. Measured: a profile asking for ollama +
    /// nomic-embed-text resolved to fastembed/all-minilm and said nothing.
    #[test]
    fn a_pre_three_layer_config_is_refused_by_name() {
        let path = write_tmp(
            "old",
            r#"
default_profile = "default"
[profiles.default]
embedder_type = "ollama"
ollama_url = "http://localhost:11434"
embedding_model = "nomic-embed-text"
qdrant_url = "http://localhost:6334"
chunk_size = 1000
"#,
        );
        let err = check_retired_keys(&path).unwrap_err().to_string();

        for key in [
            "embedder_type",
            "ollama_url",
            "embedding_model",
            "chunk_size",
        ] {
            assert!(err.contains(key), "must name {key}: {err}");
        }
        // and must say what to use instead, not merely that it is wrong
        assert!(err.contains("[backend."), "{err}");
        assert!(err.contains("target_chunk_size"), "{err}");
        let _ = std::fs::remove_file(path);
    }

    /// `max_chunk_size` -> `max_chunk_bytes` is not a rename: the unit changed
    /// from tokens to bytes. Saying so is the whole point of the message.
    #[test]
    fn the_unit_change_is_called_out_not_just_the_rename() {
        let path = write_tmp(
            "units",
            "[profiles.p]
embedder = \"e\"
qdrant_url = \"u\"
max_chunk_size = 6000
",
        );
        let err = check_retired_keys(&path).unwrap_err().to_string();
        assert!(err.contains("max_chunk_bytes"), "{err}");
        assert!(err.to_uppercase().contains("BYTES"), "{err}");
        let _ = std::fs::remove_file(path);
    }

    /// A current config must pass untouched, or every user is blocked.
    #[test]
    fn a_three_layer_config_passes() {
        let path = write_tmp(
            "new",
            r#"
default_profile = "p"
[backend.b]
kind = "ollama"
url = "http://localhost:11434"
[embedder.e]
backend = "b"
model = "qwen3-embedding:0.6b-q8_0"
num_ctx = 16384
[profiles.p]
embedder = "e"
qdrant_url = "http://localhost:6334"
target_chunk_size = 2000
"#,
        );
        check_retired_keys(&path).expect("current schema must load");
        let _ = std::fs::remove_file(path);
    }

    /// Unreadable or malformed files are Figment's to report, with its own
    /// line/column detail. Reporting them here would replace a good error with
    /// a worse one.
    #[test]
    fn unparseable_input_defers_rather_than_masking() {
        let path = write_tmp("bad", "this is not TOML at all [[[");
        check_retired_keys(&path).expect("malformed must defer");
        let _ = std::fs::remove_file(path);

        check_retired_keys(std::path::Path::new("/nonexistent/vecdb.toml"))
            .expect("missing must defer");
    }

    // ── Collection aliases ───────────────────────────────────────
    //
    // resolve() consults collection_aliases before [collections.*], so both of
    // these route writes somewhere other than where the config appears to say.

    fn with_docs_collection() -> Config {
        let mut config = three_layer();
        config.collections.insert(
            "docs".to_string(),
            CollectionConfig {
                name: "docs".to_string(),
                description: None,
                profile: Some("remote".to_string()),
                embedder: None,
                qdrant_url: None,
                qdrant_api_key: None,
                target_chunk_size: None,
                chunk_overlap: None,
                max_chunk_bytes: None,
                quantization: None,
            },
        );
        config
    }

    /// An alias named after a real collection wins, silently, forever.
    #[test]
    fn alias_shadowing_a_collection_is_refused() {
        let mut config = with_docs_collection();
        config
            .collection_aliases
            .insert("docs".to_string(), "docs".to_string());

        let err = Config::validate_graph(&config).unwrap_err().to_string();
        assert!(err.contains("shadow"), "{err}");
        assert!(err.contains("docs"), "{err}");
    }

    /// A typo'd target does not error at use — the name is taken verbatim and a
    /// collection named after the typo gets created.
    #[test]
    fn alias_pointing_at_nothing_is_refused() {
        let mut config = with_docs_collection();
        config
            .collection_aliases
            .insert("d".to_string(), "dosc".to_string());

        let err = Config::validate_graph(&config).unwrap_err().to_string();
        assert!(err.contains("dosc"), "{err}");
        assert!(err.contains("known collections"), "{err}");
    }

    /// The ordinary case still loads, and still resolves through the alias.
    #[test]
    fn a_well_formed_alias_resolves() {
        let mut config = with_docs_collection();
        config
            .collection_aliases
            .insert("d".to_string(), "docs".to_string());

        Config::validate_graph(&config).expect("valid alias must load");
        let r = config.resolve(None, Some("d")).unwrap();
        assert_eq!(r.collection.as_deref(), Some("docs"));
    }

    /// `num_ctx` belongs to the embedder, and is reported as coming from there.
    #[test]
    fn num_ctx_comes_from_the_embedder_verbatim() {
        let config = three_layer();
        let r = config.resolve(Some("remote"), None).unwrap();
        assert_eq!(r.num_ctx.value, 16384, "used exactly as written");
        assert_eq!(r.num_ctx.source, Source::Embedder("baby_qwen".to_string()));
    }

    /// Local and remote tuning cannot collide: they are different fields on
    /// different embedders, and only the one matching the backend is consulted.
    #[test]
    fn local_and_remote_batch_knobs_are_separate() {
        let mut config = three_layer();
        config.embedder.insert(
            "micro".to_string(),
            EmbedderSpec {
                backend: "local".to_string(),
                model: "bge-micro-v2".to_string(),
                num_ctx: None,
                batch_inputs: Some(999), // wrong knob for this backend
                batch_rows: Some(4),
                use_gpu: Some(true),
                dimension: None,
            },
        );
        config.profiles.insert(
            "localp".to_string(),
            Profile {
                embedder: "micro".to_string(),
                qdrant_url: "http://localhost:6334".to_string(),
                qdrant_api_key: None,
                default_collection_name: None,
                quantization: None,
                target_chunk_size: None,
                max_chunk_bytes: None,
                chunk_overlap: None,
                resolved_profile_name: "localp".to_string(),
            },
        );

        let local = config.resolve(Some("localp"), None).unwrap();
        let remote = config.resolve(Some("remote"), None).unwrap();

        assert_eq!(local.batch.value, 4, "fastembed uses batch_rows");
        assert!(!local.is_ollama());
        assert!(local.use_gpu.value);

        assert_eq!(remote.batch.value, 48, "ollama uses batch_inputs");
        assert!(remote.is_ollama());
        assert!(
            !remote.use_gpu.value,
            "use_gpu is a fastembed knob and must not leak into a remote embedder"
        );
    }

    /// A dangling reference must name what is missing and what exists.
    #[test]
    fn missing_references_are_named() {
        let mut config = three_layer();
        config.embedder.insert(
            "orphan".to_string(),
            EmbedderSpec {
                backend: "nowhere".to_string(),
                model: "x".to_string(),
                num_ctx: None,
                batch_inputs: None,
                batch_rows: None,
                use_gpu: None,
                dimension: None,
            },
        );
        config.profiles.insert(
            "bad".to_string(),
            Profile {
                embedder: "orphan".to_string(),
                qdrant_url: "http://localhost:6334".to_string(),
                qdrant_api_key: None,
                default_collection_name: None,
                quantization: None,
                target_chunk_size: None,
                max_chunk_bytes: None,
                chunk_overlap: None,
                resolved_profile_name: "bad".to_string(),
            },
        );

        let err = config.resolve(Some("bad"), None).unwrap_err().to_string();
        assert!(err.contains("nowhere"), "{err}");
        assert!(err.contains("blade"), "must list what does exist: {err}");
        // The nested-table trap is common enough to name in the error itself.
        assert!(err.contains("nested TOML table"), "{err}");
    }

    /// An Ollama backend with no url is unusable; say so rather than dialling
    /// some default endpoint the operator never configured.
    #[test]
    fn ollama_backend_without_url_is_rejected() {
        let mut config = three_layer();
        config.backend.get_mut("blade").unwrap().url = None;
        let err = config
            .resolve(Some("remote"), None)
            .unwrap_err()
            .to_string();
        assert!(err.contains("no `url`"), "{err}");
    }

    /// Chunking precedence, and the derived ceiling. Asserted as a derivation,
    /// never a literal: the previous version of this test pinned `512 * 4`, so
    /// the fact that ×4 sat below real chunk weight could never fail it.
    #[test]
    fn chunking_precedence_and_derived_ceiling() {
        let mut config = three_layer();
        config.profiles.get_mut("remote").unwrap().target_chunk_size = Some(2048);
        config.collections.insert(
            "docs".to_string(),
            CollectionConfig {
                name: "docs".to_string(),
                description: None,
                profile: Some("remote".to_string()),
                embedder: None,
                qdrant_url: None,
                qdrant_api_key: None,
                target_chunk_size: Some(6900),
                chunk_overlap: None,
                max_chunk_bytes: None,
                quantization: None,
            },
        );

        let prof = config.resolve(Some("remote"), None).unwrap();
        assert_eq!(prof.target_chunk_size.value, 2048);
        assert_eq!(
            prof.target_chunk_size.source,
            Source::Profile("remote".to_string())
        );

        let coll = config.resolve(None, Some("docs")).unwrap();
        assert_eq!(coll.target_chunk_size.value, 6900);
        assert_eq!(
            coll.target_chunk_size.source,
            Source::Collection("docs".to_string())
        );

        assert_eq!(
            coll.max_chunk_bytes.value,
            default_max_chunk_bytes(6900),
            "an unset ceiling derives from the chunk size it protects"
        );
        assert!(
            coll.max_chunk_bytes.value > coll.target_chunk_size.value,
            "the ceiling must sit above the target"
        );
        assert_eq!(
            coll.max_chunk_bytes.source,
            Source::Derived("target_chunk_size".to_string())
        );
    }

    /// Collection overrides win over the profile for the vector store too.
    #[test]
    fn collection_overrides_qdrant_and_collection_name() {
        let mut config = three_layer();
        config.collections.insert(
            "lts".to_string(),
            CollectionConfig {
                name: "docs-lts".to_string(),
                description: None,
                profile: Some("remote".to_string()),
                embedder: None,
                qdrant_url: Some("https://qdrant.example.com".to_string()),
                qdrant_api_key: None,
                target_chunk_size: None,
                chunk_overlap: None,
                max_chunk_bytes: None,
                quantization: None,
            },
        );
        let r = config.resolve(None, Some("lts")).unwrap();
        assert_eq!(r.qdrant_url, "https://qdrant.example.com");
        assert_eq!(r.collection.as_deref(), Some("docs-lts"));
    }
}
