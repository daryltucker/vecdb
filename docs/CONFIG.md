# Configuration Reference

This document provides a complete reference for configuring `vecdb`.

## Quick Start

Copy this minimal configuration to `~/.config/vecdb/config.toml`:

```toml
# Minimal Configuration - Works out of the box!
default_profile = "default"

[profiles.default]
qdrant_url = "http://localhost:6334"

[collections.docs]
name = "docs"
profile = "default"
```

That's it! The local embedder is enabled by default, so no external services are required (except Qdrant).

> **Note:** `default_collection_name` on profiles is optional. The recommended pattern is for **collections to reference profiles** (via `profile = "..."`) rather than profiles referencing collections. This lets you reuse a single profile across many collections.

---

## Full Configuration Example

```toml
# ~/.config/vecdb/config.toml
# Full Configuration with all options

default_profile = "default"

# ═══════════════════════════════════════════════════════════
# GLOBAL SETTINGS
# ═══════════════════════════════════════════════════════════

# Local embedding model (shared across ALL profiles with embedder_type="local")
# Only ONE local model can be loaded per process
local_embedding_model = "bge-micro-v2"

# Use GPU for local embeddings if available (Requires CUDA-enabled build)
local_use_gpu = true

# ═══════════════════════════════════════════════════════════
# PROFILES - Connection + model presets. Collection-agnostic.
# ═══════════════════════════════════════════════════════════
[profiles.default]
qdrant_url = "http://localhost:6334"        # Qdrant gRPC endpoint
embedder_type = "local"                     # "local" (built-in) or "ollama"
accept_invalid_certs = false                # Set true for self-signed certs

# Tier 2: Remote Ollama, high-quality model
[profiles.high]
qdrant_url = "http://localhost:6334"
ollama_url = "https://ollama.example.com"
embedder_type = "ollama"
embedding_model = "Qwen3-Embedding-4B-Q8_0:latest"
accept_invalid_certs = true
num_ctx = 8192

# ═══════════════════════════════════════════════════════════
# COLLECTIONS - Data stores. Each points to a profile.
#   Collections CAN override any profile field.
#   This is the recommended way to bind models to collections.
# ═══════════════════════════════════════════════════════════
[collections.docs]
name = "docs"
description = "General project documentation"
profile = "default"                         # Inherit from "default" profile

[collections.docs-lts]
name = "docs-lts"
description = "High quality, long-term embeddings on remote Qdrant"
profile = "high"                            # Inherit from "high" profile
qdrant_url = "https://qdrant.example.com"  # Override: use remote Qdrant
chunk_size = 2048
max_chunk_size = 3072
chunk_overlap = 256

# Legacy Aliases (Simple redirects)
[collection_aliases]
b = "brain"


# ═══════════════════════════════════════════════════════════
# INGESTION - Document processing settings
# ═══════════════════════════════════════════════════════════
[ingestion]
default_strategy = "recursive"              # "recursive" or "code_aware"
chunk_size = 512                            # Target tokens per chunk
chunk_overlap = 50                          # Overlap between chunks
tokenizer = "cl100k_base"                   # "cl100k_base" (GPT-4) or "char"
max_concurrent_requests = 4                 # Parallel file processing tasks
gpu_batch_size = 2                          # GPU embedding batch size

# Pattern-based overrides (glob patterns)
[ingestion.overrides."*.rs"]
strategy = "code_aware"
chunk_size = 1024

[ingestion.overrides."*.md"]
strategy = "recursive"
chunk_size = 800
```

---

## Configuration Reference

### Top-Level Options

| Key | Type | Default | Description |
|-----|------|---------|-------------|
| `default_profile` | string | `"default"` | Profile to use when `-p` not specified |
| `local_embedding_model` | string | `"bge-micro-v2"` | **Global**: Embedding model for ALL profiles with `embedder_type="local"`. Only **ONE** local model can be loaded per process. |
| `local_use_gpu` | bool | `false` | **Global**: Use GPU for local embeddings if available. Requires `cuda` feature flag. |
| `fastembed_cache_path` | string | `~/.config/vecdb/fastembed_cache` | Path for `local` embedder model cache |
| `smart_routing_keys` | array | `["source_type", "language"]` | Keys to use for Smart Routing / Facet Auto-Detection. |

### Profile Options (`[profiles.<name>]`)

| Key | Type | Default | Description |
|-----|------|---------|-------------|
| `qdrant_url` | string | `"http://localhost:6334"` | Qdrant gRPC endpoint URL |
| `default_collection_name` | string | `null` | **Optional** fallback collection when `-c` is not specified. Prefer using `profile =` on the collection instead. |
| `embedder_type` | string | `"local"` | Embedding backend: `"local"` or `"ollama"` |
| `ollama_url` | string | `"http://localhost:11434"` | Ollama API URL (only for `embedder_type = "ollama"`) |
| `embedding_model` | string | `"nomic-embed-text"` | Ollama model name (**only for** `embedder_type = "ollama"`). If set on a `local` profile, a warning will be displayed and this field will be ignored. |
| `accept_invalid_certs` | bool | `false` | Accept invalid TLS certificates |
| `qdrant_api_key` | string | `null` | Optional API Key for Qdrant authentication |
| `ollama_api_key` | string | `null` | Optional API Key for Ollama proxy authentication |
| `quantization` | string | `null` | "scalar", "binary", or "none" |
| `chunk_size` | integer | `null` | Override ingestion chunk size |
| `chunk_overlap` | integer | `null` | Override chunk overlap |
| `max_chunk_size` | integer | `null` | Override max chunk size |
| `gpu_batch_size` | integer | `null` | Override GPU batch size |
| `num_ctx` | integer | `null` | Override Ollama context window size |

### Embedder Types

| Type | Model | Dimensions | Requirements |
|------|-------|------------|--------------|
| `local` | AllMiniLM-L6-v2 | 384 | None (ONNX built-in, ~30MB download) |
| `ollama` | configurable | varies | Ollama server running + model pulled |

**Recommendation**: Use `local` for development and portability. Use `ollama` when you need larger/custom models.

### Collection Profiles (`[collections.<name>]`)
 
Define named collections with specific configurations that override the active profile.
 
```toml
# Recommended pattern: collection → profile
[collections.brain]
name = "agent_memory_v1"        # Actual Qdrant collection name
profile = "default"             # Inherit connection + model from "default" profile
description = "My agent's memory"
chunk_size = 512                # Collection-specific chunk size

# A collection can target a different Qdrant instance than its profile
[collections.docs-lts]
name = "docs-lts"
profile = "high"                # High-quality remote Ollama model
qdrant_url = "https://qdrant.example.com"  # But store on remote Qdrant
chunk_size = 2048

[collection_aliases]            # Simple redirects
b = "brain"
```

Usage:
- `vecdb search -c brain "query"` — resolves "brain" → uses `agent_memory_v1` with `default` profile
- `vecdb search -c b "query"` — alias redirects to "brain", same result
- `vecdb ingest ./ -c docs-lts` — uses `high` profile's model, stores on remote Qdrant

#### Collection Profile Options (`[collections.<name>]`)

| Key | Type | Default | Description |
|-----|------|---------|-------------|
| `name` | string | **REQUIRED** | Actual Qdrant collection name |
| `description` | string | `null` | Optional description for listing |
| `profile` | string | `null` | Base profile to inherit from (e.g., `"high"`). Recommended way to bind a collection to its model. |
| `qdrant_url` | string | `null` | Override Qdrant URL (e.g., point to a remote instance for this collection only) |
| `embedder_type` | string | `null` | Override active profile's embedder type |
| `embedding_model`| string | `null` | Override active profile's embedding model |
| `ollama_url` | string | `null` | Override Ollama URL |
| `qdrant_api_key` | string | `null` | Override Qdrant API Key |
| `ollama_api_key` | string | `null` | Override Ollama API Key |
| `num_ctx` | integer | `null` | Override Ollama context window size |
| `chunk_size` | integer | `null` | Override ingestion chunk size |
| `max_chunk_size` | integer | `null` | Override max chunk size |
| `chunk_overlap` | integer | `null` | Override chunk overlap |
| `use_gpu` | bool | `null` | Override `local_use_gpu` for this collection |
| `gpu_batch_size` | integer | `null` | Override GPU batch size |
| `quantization` | string | `null` | "scalar", "binary", or "none" (See Quantization below) |

> **Warning:** Changing the `embedder_type` or `embedding_model` for an existing collection will likely break searches due to vector dimension mismatches (e.g., 384 vs 768). If you change the model, you must delete and re-ingest the collection.

### Quantization Options

You can reduce memory usage (RAM) by quantizing vectors. This is configured per-collection or in a profile.

| Type | Description | Memory Usage | Precision Loss |
|------|-------------|--------------|----------------|
| `none` | Default Float32 vectors | 100% (Baseline) | None |
| `scalar` | **Int8 Quantization** | ~25% (4x smaller) | Very Low (<1%) |
| `binary` | **1-bit Quantization** | ~3% (32x smaller) | Moderate |

**Configuration:**
Set `quantization = "scalar"` in your `[profile]` or `[collection]` block.
Or use the CLI: `vecdb config set-quantization <collection> scalar`.

**Applying Changes:**
Changing the config does *not* immediately re-index existing vectors. You must run:
```bash
vecdb optimize <collection_name>
```


### Ingestion Options (`[ingestion]`)

| Key | Type | Default | Description |
|-----|------|---------|-------------|
| `default_strategy` | string | `"recursive"` | Chunking strategy |
| `chunk_size` | integer | `512` | Target tokens/chars per chunk |
| `max_chunk_size` | integer | `null` | Hard limit for chunk size |
| `chunk_overlap` | integer | `50` | Overlap between adjacent chunks |
| `respect_gitignore` | bool | `false` | Always respect .gitignore files |
| `tokenizer` | string | `"cl100k_base"` | Tokenizer for splitting |
| `max_concurrent_requests` | integer | `4` | Max parallel file processing tasks |
| `gpu_batch_size` | integer | `2` | Max GPU embedding batch size (OOM protection) |

#### Smart Ingestion (Path Parsing)
You can configure `path_rules` to extract metadata from file paths (e.g., years, versions).
See [VECTOR_FACETS.md](VECTOR_FACETS.md) for details and [TRAINING_GOLD.md](internal/TRAINING_GOLD.md) for 10 fun examples!

#### Chunking Strategies

| Strategy | Description | Best For |
|----------|-------------|----------|
| `recursive` | Token-accurate recursive splitting | Prose, documentation, mixed content |
| `code_aware` | AST-aware splitting via `vecq` | Source code (functions, structs) |

#### Tokenizers

| Tokenizer | Description |
|-----------|-------------|
| `cl100k_base` | GPT-4 tokenizer (recommended) |
| `char` | Character-based splitting |

### Ingestion Overrides (`[ingestion.overrides."<pattern>"]`)

Override settings for files matching glob patterns:

```toml
[ingestion.overrides."*.py"]
strategy = "code_aware"
chunk_size = 800
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

### Switching from Ollama to Local
Change your profile:
```toml
[profiles.default]
embedder_type = "local"  # Was: "ollama"
```

**Note**: Existing embeddings may not be compatible when switching embedding models. Consider creating a new collection.
