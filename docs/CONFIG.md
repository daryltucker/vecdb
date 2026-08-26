# Configuration Reference

This document provides a complete reference for configuring `vecdb`.

## Quick Start

Copy this to `~/.config/vecdb/config.toml`:

```toml
default_profile = "default"

[backend.local]
kind = "fastembed"

[embedder.default]
backend = "local"
model = "all-minilm-l6-v2"

[profiles.default]
embedder = "default"
qdrant_url = "http://localhost:6334"
```

That is the whole thing — the local embedder needs no external services (except
Qdrant).

### The three layers

Configuration is split by *what a thing actually is*, so one setting can never
silently apply to something it does not describe:

| Layer | Answers | Contains |
|---|---|---|
| `[backend.<name>]` | **WHERE** a model runs | connection only — `kind`, `url`, credentials |
| `[embedder.<name>]` | **WHAT** model, **HOW** tuned | `backend`, `model`, `num_ctx`, batch size, `dimension` |
| `[profiles.<name>]` | **WHICH** embedder + **WHICH** store | `embedder`, `qdrant_url`, chunking overrides |

The split exists because one Ollama instance serves many models. Two embedders
may share one backend:

```toml
[backend.blade]
kind = "ollama"
url  = "http://blade.lan:11434"

[embedder.small]
backend = "blade"
model   = "qwen3-embedding:0.6b-q8_0"
num_ctx = 16384

[embedder.large]
backend = "blade"          # same instance
model   = "qwen3-embedding:4b-q8_0"
num_ctx = 8192
```

**Backend names are free-form** — `kind` says what it is, so the name need not
repeat it. Dots must be quoted: `[backend."ollama.blade"]` is a backend named
`ollama.blade`, while `[backend.ollama.blade]` is a *nested table* and will not
parse.

Every reference is validated when the config loads, so a typo in `embedder = `
or `backend = ` is reported immediately, naming what exists.

To see what any of it resolves to, and where each value came from:

```
vecdb config show -c <collection>
```

### Overriding a layer for one run

Each layer has a flag, so a single run can change one without redefining it:

| Flag | Overrides | Changes |
|---|---|---|
| `--profile <name>` | WHICH | embedder *and* store |
| `--embedder <name>` | WHAT + HOW | the model and its tuning; store unchanged |
| `--backend <name>` | WHERE | the host only — model, `num_ctx` and batch unchanged |

The motivating case is two machines filling one collection in parallel, because
a single embed host becomes everyone's queue:

```bash
# terminal 1 — this repo, on the embedder's own backend
vecdb --profile code ingest -c code ./

# terminal 2 — another repo, same collection, different GPU
vecdb --profile code --backend blade ingest -c code ./
```

This is safe only because `--backend` cannot change what a vector *is*. Both
runs use the same model and the same tuning, so both write into the same
embedding space — and that is verified rather than assumed: the collection
records the model's **weight digest**, and a run whose digest or dimension
disagrees is refused (see *Model identity*, below).

Two things it deliberately will not do:

- **Cross a `kind`.** Only the knobs matching a backend's `kind` are consulted,
  so pointing an Ollama-tuned embedder at a `fastembed` backend would discard
  `num_ctx` and `batch_inputs` and embed at defaults you never chose. That is an
  error naming `--embedder` as the way to do it deliberately.
- **Blame the wrong thing.** An unknown `--backend` reports the flag, not the
  `[embedder.*]` table, which is correct and would otherwise be audited for
  nothing.

`vecdb list` ignores both flags by design: it resolves every profile to
enumerate *stores*, so an embedder override across all of them is meaningless.

`config show` reports an override as its source, so the effective value never
points at a table that does not explain it:

```
embedder   baby_qwen  (qwen3-embedding:0.6b-q8_0 on backend blade — ollama)
           backend  selected by --backend (model and tuning unchanged)
```

## Full Configuration Example

```toml
# ~/.config/vecdb/config.toml

default_profile = "default"
fastembed_cache_path = "~/.config/vecdb/fastembed_cache"
smart_routing_keys = ["source_type", "language"]

# ═══ BACKENDS — where models run ═══════════════════════════════
[backend.local]
kind = "fastembed"                  # in-process ONNX, no endpoint

[backend.blade]
kind = "ollama"
url  = "http://blade.lan:11434"
# api_key = "..."
# accept_invalid_certs = true       # staging / self-signed

# ═══ EMBEDDERS — what model, how tuned ═════════════════════════
[embedder.micro]
backend    = "local"
model      = "all-minilm-l6-v2"
use_gpu    = false                  # fastembed only
batch_rows = 2                      # ONNX rows per inference

[embedder.code]
backend      = "blade"
model        = "qwen3-embedding:0.6b-q8_0"
num_ctx      = 16384                # ollama only — the EFFECTIVE ceiling
batch_inputs = 8                    # inputs per /api/embed request
# dimension  = 1024                 # Matryoshka truncation (irreversible)

# ═══ PROFILES — which embedder, which store ════════════════════
[profiles.default]
embedder     = "micro"
qdrant_url   = "http://localhost:6334"
quantization = "none"

[profiles.high]
embedder   = "code"
qdrant_url = "http://localhost:6334"
target_chunk_size = 12000

# ═══ COLLECTIONS — overrides ═══════════════════════════════════
[collections.docs]
name    = "docs"
profile = "high"

[collections.docs-lts]
name       = "docs-lts"
profile    = "high"
embedder   = "micro"                        # different model, same profile
qdrant_url = "https://qdrant.example.com"   # different store
target_chunk_size = 2048

[collection_aliases]
b = "brain"

# ═══ INGESTION — chunking policy ═══════════════════════════════
[ingestion]
default_strategy = "recursive"
target_chunk_size       = 512
chunk_overlap    = 50
tokenizer        = "cl100k_base"
on_oversize      = "split"
max_concurrent_requests = 4

[ingestion.overrides."*.rs"]
target_chunk_size = 1024
```

> Source files do not take a `strategy`. Anything vecq can parse is split along
> its AST by the parser — one chunk per function, struct or class — and no
> chunker is consulted at all. `strategy` governs only files with no structural
> parser.

## Configuration Reference

<!-- BEGIN GENERATED CONFIG REFERENCE -->
<!-- Generated by `cargo run -p xtask -- gen-config-docs`. DO NOT EDIT BY HAND. -->
<!-- Source of truth: the doc comments in vecdb-core/src/config.rs -->

### Top-Level Options

| Key | Type | Required | Description |
|-----|------|----------|-------------|
| `backend` | table | no | Where models run. Connection details only, reusable by many embedders. — Names are free-form; `kind` says what it is, so the name need not repeat it. Dots are allowed if quoted — `[backend."ollama.blade"]` — but a bare `[backend.ollama.blade]` is a *nested* TOML table and will not parse. |
| `collection_aliases` | table | no | Simple aliases: short_name -> collection key |
| `collections` | table | no | Collection-level overrides. |
| `default_profile` | string | no | Profile used when `--profile` is not given. |
| `embedder` | table | no | Which model, and how it is tuned. Each references a backend. — This is the unit the storage layer already treats as primary: genesis records model name, digest, architecture, parameter size, quantization and dimension, and the space guard holds every write to that identity. Naming it here means config can finally refer to the thing the database tracks. |
| `fastembed_cache_path` | string | no | Where fastembed caches downloaded models. Genuinely global — it is a disk location, not a property of any one embedder. |
| `ingestion` | `IngestionConfig` | no | Chunking and discovery policy. Applies to every profile — it describes how documents are cut up, which is independent of which model embeds them. |
| `profiles` | table | no | Which embedder to use, and which vector store to write to. |
| `server` | `ServerConfig` | no | Server-side runtime tuning (idle eviction, watchdog cadence). Only consulted by `vecdb-server`; CLI commands ignore it. |
| `smart_routing_keys` | array | no | Keys to use for Smart Routing (Facet Auto-Detection). |

#### Backend Options (`[backend.<name>]`)

| Key | Type | Required | Description |
|-----|------|----------|-------------|
| `kind` | `BackendKind` | **yes** | `"ollama"` or `"fastembed"`. Decides which embedder knobs apply. |
| `accept_invalid_certs` | boolean | no | Accept invalid TLS certificates (staging / self-signed endpoints). |
| `api_key` | string | no | Bearer token, for an authenticated proxy in front of the endpoint. |
| `url` | string | no | Endpoint. Required for `ollama`, meaningless for `fastembed`. |

#### Embedder Options (`[embedder.<name>]`)

| Key | Type | Required | Description |
|-----|------|----------|-------------|
| `backend` | string | **yes** | Name of the `[backend.*]` entry this runs on. |
| `model` | string | **yes** | Model identifier, in the backend's namespace — an Ollama tag, or a fastembed model id. |
| `batch_inputs` | integer | no | Inputs per `/api/embed` request. **Ollama only.** — Not the same knob as `batch_rows`: this is array length over HTTP, and it fails as a request timeout. |
| `batch_rows` | integer | no | Rows per ONNX inference. **fastembed only.** — Not the same knob as `batch_inputs`: this is in-process, and it fails as an OOM. |
| `dimension` | integer | no | Matryoshka truncation target. Omit for the model's native width. — Irreversible once a collection is written at it — the genesis record pins it and the space guard enforces it. |
| `num_ctx` | integer | no | Context window to request, in tokens. **Ollama only.** — This is the effective ceiling, and it is not what the model declares. Measured 2026-236: `qwen3-embedding:0.6b-q8_0` declares `context_length = 32768`, but with no `options` the server refused input at ~4086 tokens — Ollama's default `num_ctx` of 4096. The same input at ~12258 tokens succeeded with `num_ctx = 16384`. So `/api/embed` honours this, and `context_length` is only the maximum it will accept. — Used exactly as written. Never derived over, never clamped. |
| `use_gpu` | boolean | no | Use GPU for local inference. **fastembed only.** |

### Profile Options (`[profiles.<name>]`)

| Key | Type | Required | Description |
|-----|------|----------|-------------|
| `embedder` | string | **yes** | Name of the `[embedder.*]` entry to use. |
| `chunk_overlap` | integer | no | Override `[ingestion].chunk_overlap` for this profile. |
| `default_collection_name` | string | no | Default collection when `-c` is not given. |
| `max_chunk_bytes` | integer | no | Override the byte ceiling above which a chunk is re-split. Unset derives from `target_chunk_size`; it must never sit below it. |
| `qdrant_api_key` | string | no | API key for Qdrant authentication. |
| `qdrant_url` | string | no | Qdrant endpoint for collections under this profile. |
| `quantization` | `QuantizationType` | no | Default quantization for collections created under this profile. |
| `target_chunk_size` | integer | no | Override `[ingestion].target_chunk_size` for this profile. Counted in whatever `tokenizer` counts — tokens under the default `cl100k_base`. |

#### Collection Profile Options (`[collections.<name>]`)

| Key | Type | Required | Description |
|-----|------|----------|-------------|
| `name` | string | **yes** | The actual Qdrant collection name. |
| `chunk_overlap` | integer | no | Override the chunk overlap for this collection. |
| `description` | string | no | Free-text note shown by `vecdb list`. |
| `embedder` | string | no | Override: use a different embedder for this collection. |
| `max_chunk_bytes` | integer | no | Override the byte ceiling above which a chunk is re-split. |
| `profile` | string | no | Profile to inherit from. |
| `qdrant_api_key` | string | no | Override the profile's Qdrant API key. |
| `qdrant_url` | string | no | Override: a different Qdrant instance. |
| `quantization` | `QuantizationType` | no | Vector quantization for this collection: `"scalar"`, `"binary"` or `"none"`. Fixed when the collection is created. |
| `target_chunk_size` | integer | no | Override the chunk target for this collection. Baked into the vectors at ingest — changing it later means a re-ingest. |

### Ingestion Options (`[ingestion]`)

| Key | Type | Required | Description |
|-----|------|----------|-------------|
| `allow_embed_truncation` | boolean | no | Permit the embedder to silently cut chunks that exceed the model context. — Off by default. A truncated embed succeeds in every observable way — right shape, clean upsert — while the tail of the chunk is simply gone, and only a re-ingest restores it. Refusing turns that into an oversized-chunk error that names the file, which is a problem you can act on. |
| `chunk_overlap` | integer | no | How much adjacent chunks overlap, in the same unit as `target_chunk_size`. Overlap preserves context across a boundary at the cost of duplication. |
| `default_strategy` | string | no | Chunking strategy for files with no structural parser: `"recursive"` (token-accurate splitting), `"semantic"` (alias for it), or `"simple"` (fixed-width). Rejected at load time if it is anything else. — This does not govern source code. A file whose type vecq recognises is split along its AST by the parser, per element, and no chunker runs at all — so AST-aware chunking is automatic and is not something a strategy selects. The retired `"code_aware"` value promised exactly that and could not deliver it. |
| `gpu_batch_size` | integer | no | GPU Concurrency: Batch size for GPU embedding (None = auto calculate optimal size) |
| `max_chunk_bytes` | integer | no | Hard limit for acceptable chunk size |
| `max_concurrent_requests` | integer | no | Concurrency Limit: Max number of file processing tasks running in parallel |
| `on_oversize` | `OversizePolicy` | no | What to do with a chunk that exceeds the resolved ceiling: `"split"` or `"skip"`. Defaults to `split`. |
| `overrides` | table | no | Per-glob overrides, e.g. `[ingestion.overrides."*.rs"]`. Lets source files chunk differently from prose without a separate collection. |
| `path_rules` | array | no | Path parsing rules for metadata extraction Path parsing rules for metadata extraction |
| `respect_gitignore` | boolean | no | Consult `.gitignore` when walking. **Off, and stays off.** — `.gitignore` is a build-artifact list, not an indexing policy, and the two disagree constantly. `.vectorignore` is the knob that governs indexing. This is an escape hatch for people driving the system who expect git semantics — it is never the default and never inferred. |
| `target_chunk_size` | integer | no | Target chunk size, counted in whatever `tokenizer` counts — **tokens** under the default `cl100k_base`, not bytes. Compare `max_chunk_bytes`. |
| `tokenizer` | string | no | What `target_chunk_size` and `chunk_overlap` are counted in: — * `"cl100k_base"` (default) — GPT-4 tokens. * `"bytes"` — raw bytes, snapped to the nearest UTF-8 boundary. Fastest. Was spelled `"char"`, which it never was. * anything else — characters, via the text splitter's `Characters` sizer. — Whatever this counts, `max_chunk_bytes` still counts bytes. |

#### Server Options (`[server]`)

| Key | Type | Required | Description |
|-----|------|----------|-------------|
| `deep_idle_secs` | integer | no | After this many seconds without use, drop the cache entry and (in stdio mode) exit the subprocess. Set to 0 to disable deep eviction. Should be greater than `soft_idle_secs`; if not, deep wins. |
| `idle_check_interval_secs` | integer | no | How often the watchdog wakes up to evaluate idle entries. |
| `idle_eviction_enabled` | boolean | no | Master switch — if false, no watchdog is spawned. |
| `soft_idle_secs` | integer | no | After this many seconds without use, release the embedder's loaded model. Set to 0 to disable soft eviction. |

<!-- END GENERATED CONFIG REFERENCE -->

#### `target_chunk_size` vs `max_chunk_bytes` — different units, on purpose

These two are **not denominated in the same unit**, and the difference matters:

| Key | Unit | Where it is enforced |
|-----|------|----------------------|
| `target_chunk_size` | whatever `tokenizer` counts — **tokens** under the default `cl100k_base` | the chunker, when deciding where to split |
| `max_chunk_bytes` | **bytes** (`String::len()`) | the oversize guard, after chunking |

A ceiling below the target it protects is not a safety net — it is a second
chunking pass. Every full-size chunk trips it and gets re-split by
`FixedWidthChunker`, which discards the AST boundaries structural chunking exists to
produce. Nothing about the search results reveals this happened, and only a
re-ingest undoes it.

Concretely: at `target_chunk_size = 6144` tokens, real source code chunks weigh **~32 KB**
(measured, ~5.2 bytes/token). A `max_chunk_bytes` of 6000 or 8192 therefore fires
on essentially everything. Leave `max_chunk_bytes` unset unless you have measured
your corpus — the derived default (`target_chunk_size × 6`) is chosen to clear real
content with headroom.

If a chunk does exceed the ceiling, vecdb now says so, naming the file.

#### Smart Ingestion (Path Parsing)
You can configure `path_rules` to extract metadata from file paths (e.g., years, versions).
See [VECTOR_FACETS.md](VECTOR_FACETS.md) for details and [TRAINING_GOLD.md](internal/TRAINING_GOLD.md) for 10 fun examples!

#### Chunking Strategies

| Strategy | Description | Best For |
|----------|-------------|----------|
| `recursive` | Token-accurate recursive splitting | Prose, documentation, mixed content |
| `semantic` | Alias for `recursive` | — |
| `simple` | Fixed-width splitting | Files with no useful structure |

These apply **only to files vecq cannot parse**. Source code is chunked along
its AST by the parser regardless of this setting.

`code_aware` was removed. It selected a chunker that could never run — the
parser path wins whenever a parser exists, and vecq claims every recognised file
type — so it promised AST-aware chunking while delivering nothing. AST-aware
chunking is automatic and always on. A config still setting it is refused at
load with an explanation rather than silently falling back.

#### Tokenizers

| Tokenizer | Description |
|-----------|-------------|
| `cl100k_base` | GPT-4 tokenizer (recommended) |
| `char` | Character-based splitting |

### Ingestion Overrides (`[ingestion.overrides."<pattern>"]`)

Override settings for files matching glob patterns:

```toml
[ingestion.overrides."*.py"]
target_chunk_size = 800
chunk_overlap = 100
```

### File Ignoring (`.vectorignore`)

You can exclude files or directories from ingestion using a `.vectorignore` file. It follows standard `.gitignore` syntax.

**Priority Order**:
1. `.vectorignore` (Highest priority, always respected)
2. `.ignore` (Standard ripgrep ignore file)
3. `.gitignore` (Only if `--respect-gitignore` is enabled)

Example `.vectorignore`:
```text
target/
*.log
large_data/
secret_keys.json
```

---

## Environment Variables

| Variable | Description |
|----------|-------------|
| `VECDB_PROFILE` | Override default profile (same as `-p` flag) |
| `VECDB_CONFIG` | Override configuration file path (default: `~/.config/vecdb/config.toml`) |

---

## File Locations

| Platform | Config Path |
|----------|-------------|
| Linux | `~/.config/vecdb/config.toml` |
| macOS | `~/.config/vecdb/config.toml` |
| Windows | `%APPDATA%\vecdb\config.toml` |

---

## Troubleshooting

### "Failed to initialize local embedding model"
The local embedder downloads the model (~30MB) on first use. Ensure you have internet access for the initial download. After that, it works offline.

### "Connection refused" to Qdrant
Ensure Qdrant is running:
```bash
docker run -d -p 6333:6333 -p 6334:6334 qdrant/qdrant
```

### "Qdrant error (404): The server at this URL did not recognize the gRPC request"
Qdrant exposes two ports: 6333 (REST/JSON) and 6334 (gRPC/HTTP2). `vecdb` natively uses the high-performance gRPC port (`6334`).
If you are running Qdrant behind a reverse proxy (like Nginx or Traefik), you **must** configure the proxy to route traffic to port `6334` using HTTP/2 (`h2c`), rather than the standard REST port.

**Example Traefik Configuration:**
```yaml
      - "traefik.http.routers.qdrant-grpc.rule=Host(`qdrant-grpc.example.com`)"
      - "traefik.http.routers.qdrant-grpc.entrypoints=websecure"
      - "traefik.http.routers.qdrant-grpc.tls.certresolver=letsencrypt"
      - "traefik.http.services.qdrant-grpc.loadbalancer.server.port=6334"
      - "traefik.http.services.qdrant-grpc.loadbalancer.server.scheme=h2c"
```

### Switching from Ollama to local

Point the profile at a different embedder — one line, and both definitions can
stay in the file:

```toml
[profiles.default]
embedder = "micro"      # was "qwen4b"
```

**This changes the embedding space.** A collection records the model that wrote
it in its genesis point, and the space guard refuses a write from a different
model rather than mixing two spaces in one collection. Switching means a new
collection, or a re-ingest of the existing one.

`vecdb list` shows which model wrote each collection, and
`vecdb config show -c <collection>` shows which embedder you would write with.
