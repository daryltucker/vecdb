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
[profiles.edge]
qdrant_url = "http://localhost:6334"
ollama_url = "https://ollama.example.com"
embedder_type = "ollama"
embedding_model = "Qwen3-Embedding-4B-Q8_0:latest"
```

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
local_embedding_model = "nomic-embed-text-v1.5"
local_use_gpu = true
```

> **⚠️ Changing the model after ingestion requires re-ingesting all collections.**
> The dimension guard will block accidental mismatches.

## Hardware: Using a 4GB NVIDIA GPU

`nomic-embed-text-v1.5` at 137M params (~550MB VRAM) runs easily on a 4GB card.
Enable GPU in config with `local_use_gpu = true`. The ONNX runtime will:

1. Attempt CUDA — if available, uses GPU for 10-50x speedup
2. Fall back to CPU transparently if CUDA fails
3. Cap ONNX threads to prevent system starvation during batch ingestion

For larger models (BGE Large, Qwen3), use the Ollama remote profile.
