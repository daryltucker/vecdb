# Embedding Model Guide

## Supported Local Models (fastembed-rs / ONNX)

These models run locally via ONNX Runtime. GPU acceleration is supported via CUDA.

| Config Name | Model | Params | Dim | Context | Matryoshka | Notes |
|:---|:---|:---|:---|:---|:---|:---|
| `all-minilm-l6-v2` | all-MiniLM-L6-v2 | 22M | 384 | 256 tok | ❌ | Default. Fast, tiny |
| `bge-small-en-v1.5` | BGE Small EN v1.5 | 33M | 384 | 512 tok | ❌ | Small, good English |
| `bge-base-en-v1.5` | BGE Base EN v1.5 | 109M | 768 | 512 tok | ❌ | Mid-tier English |
| `bge-large-en-v1.5` | BGE Large EN v1.5 | 335M | 1024 | 512 tok | ❌ | Highest BGE quality |
| `nomic-embed-text-v1` | Nomic Embed v1 | 137M | 768 | 8192 tok | ❌ | Long context |
| **`nomic-embed-text-v1.5`** | **Nomic Embed v1.5** | **137M** | **768** | **8192 tok** | **✅** | **Recommended** |

### Short Aliases

- `minilm`, `default` → `all-minilm-l6-v2`
- `nomic-v1` → `nomic-embed-text-v1`
- `nomic-v1.5` → `nomic-embed-text-v1.5`
- `bge-small-en` → `bge-small-en-v1.5`
- `bge-base-en` → `bge-base-en-v1.5`
- `bge-large-en` → `bge-large-en-v1.5`

> **⚠️ Unknown model names produce a hard error.**
> This prevents silent fallback to a different model, which would cause
> dimension mismatches and corrupt search results.

## Remote Models (Ollama)

Any Ollama-hosted model can be used. Configure via profile:

```toml
[backend.edge]
kind = "ollama"
url  = "https://ollama.example.com"

[embedder.qwen4b]
backend      = "edge"
model        = "Qwen3-Embedding-4B-Q8_0:latest"
num_ctx      = 8192          # the EFFECTIVE ceiling — see CONFIG.md
batch_inputs = 8

[profiles.edge]
embedder   = "qwen4b"
qdrant_url = "http://localhost:6334"
```

One backend can serve several models: add a second `[embedder.*]` with the same
`backend`, and point another profile at it.

## Matryoshka Embeddings

Models marked ✅ Matryoshka support **dimension truncation** at storage time:

- Embed at **768-dim** (full quality), truncate to **384** or **256** for storage
- **Both query and stored vectors must use the same dimension** at search time
- vecdb's `search()` auto-resolves collection dimension and truncates queries
- The `ingest()` dimension guard prevents mixing different dimensions

### Portability Workflow

1. **Ingest on GPU**: Generate full 768-dim embeddings with `nomic-embed-text-v1.5`
2. **Export at 384-dim**: For lighter devices, truncate stored vectors
3. **Search at 384-dim**: Query vectors are auto-truncated to match

## When to Use What

| Use Case | Model | Why |
|:---|:---|:---|
| Dev/testing | `all-minilm-l6-v2` | Tiny, fast, no GPU needed |
| **Production code search** | **`nomic-embed-text-v1.5`** | Long context (8192 tok), Matryoshka, GPU-friendly |
| Multilingual / highest fidelity | Qwen3-Embedding-4B (Ollama) | State-of-the-art, needs beefy GPU |

## Configuration

```toml
# ~/.config/vecdb/config.toml
[backend.local]
kind = "fastembed"

[embedder.nomic]
backend = "local"
model   = "nomic-embed-text-v1.5"
use_gpu = true

[profiles.default]
embedder   = "nomic"
qdrant_url = "http://localhost:6334"
```

> **⚠️ Changing the model after ingestion requires re-ingesting all collections.**
> The embedding-space guard will block accidental mismatches — see below.

## Hardware: Using a 4GB NVIDIA GPU

`nomic-embed-text-v1.5` at 137M params (~550MB VRAM) runs easily on a 4GB card.
Enable GPU with `use_gpu = true` on that `[embedder.*]`. The ONNX runtime will:

1. Attempt CUDA — if available, uses GPU for 10-50x speedup
2. Fall back to CPU transparently if CUDA fails
3. Cap ONNX threads to prevent system starvation during batch ingestion

For larger models (BGE Large, Qwen3), use the Ollama remote profile.


## Embedding spaces — what makes vectors comparable

Similarity is only meaningful between vectors produced by the same model. The
contract that makes a collection searchable is therefore
`(model, digest, dimension, distance)` — not dimension alone.

**Dimension is the least discriminating field there is.** 384 and 768 are the
two most common embedding dimensions in existence:

| dim | models sharing it |
|---|---|
| 384 | `bge-small-en-v1.5`, `all-MiniLM-L6-v2`, `paraphrase-MiniLM-L6-v2` |
| 768 | `nomic-embed-text`, `bge-base-en-v1.5`, `all-mpnet-base-v2`, `gte-base` |
| 1024 | `bge-large-en-v1.5`, `mxbai-embed-large`, `qwen3-embedding:0.6b` |

A guard that only compares dimension lets two unrelated 768-dim models write
into one collection with no error. Cosine across two embedding spaces is noise,
so the result is silent quality loss with no diagnostic — and re-ingesting does
not repair it, because the poisoned points have valid IDs and dedup keeps them.

### What is recorded

Every collection vecdb creates carries a **genesis point** holding the model's
`name`, `digest`, `architecture`, `family`, `parameter_size`,
`quantization_level`, `dimension` and `distance`. For Ollama these come from
`/api/show` and `/api/tags`; they are read once and stored, so the comparison
costs nothing at query time.

It also carries a `vecdb:<version>` marker.

### Ownership before compatibility

A collection **without the marker is not a vecdb collection.** Not an
incompatible one — someone else's. vecdb will neither read nor write it, and
`vecdb list` shows it labelled rather than hiding it.

This is a permanent condition, not a migration state: a Qdrant instance is
shared infrastructure and other tools keep their own collections on it. It also
cannot be decided by dimension — a MERT audio collection and
`qwen3-embedding:0.6b` text are both 1024-dim and both Cosine.

### Compatibility tiers

Once ownership is settled:

| tier | condition | read | write |
|---|---|---|---|
| **identical** | same model digest | yes | yes |
| **compatible** | same architecture + parameter_size + dimension, different quantization | yes, with a note | needs `--allow-quantization-delta` |
| **incompatible** | anything else, including insufficient recorded identity | no | no |

Strict digest equality alone would be unusable: Q4_K_M and Q8_0 of one model
produce slightly different vectors, but the quantization error is small relative
to retrieval margins, and rejecting that pairing would make the guard something
you route around rather than rely on.

The read/write asymmetry is the point: **a bad write contaminates a collection
permanently and compounds with every subsequent ingest, while a bad read
produces one mediocre ranking and evaporates.** Protect the durable side hard;
let the transient side through with a note.

### Tags are not identity

Observed on one machine:

```
qwen3-embedding:0.6b        digest=ac6da0dfba84a81f   Q8_0
qwen3-embedding:0.6b-q8_0   digest=ac6da0dfba84a81f   Q8_0     ← same blob
qwen3-embedding:4b          digest=df5bd2e3c74cd8d0   Q4_K_M
qwen3-embedding:4b-q4_K_M   digest=df5bd2e3c74cd8d0   Q4_K_M   ← same blob
qwen3-embedding:4b-q8_0     digest=357d756ba8e5a3f2   Q8_0     ← different weights
```

Bare `qwen3-embedding:4b` resolves to **Q4_K_M** here; another machine pulling
the same tag can land on Q8_0. The string matches, the weights differ. vecdb
records the digest and displays the tag.

### Comparing definitions, never profile names

The guard never compares `profile_name`. `~/.config/vecdb/config.toml` is
per-machine, so `profiles.low` on one host and `profiles.low` on another are
different definitions under the same name. A name-based check would report
agreement where none exists — worse than no check, because it would look
verified. The profile name is recorded for diagnostics only.
