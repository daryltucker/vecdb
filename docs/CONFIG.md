# Configuration Reference

This document provides a complete reference for configuring `vecdb`.

## Quick Start

Copy this minimal configuration to `~/.config/vecdb/config.toml`:

```toml
# Minimal Configuration - Works out of the box!
default_profile = "default"

[profiles.default]
qdrant_url = "http://localhost:6334"
default_collection_name = "docs"
```

That's it! The local embedder is enabled by default, so no external services are required (except Qdrant).

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
# PROFILES - Define multiple backend configurations
# ═══════════════════════════════════════════════════════════
[profiles.default]
qdrant_url = "http://localhost:6334"        # Qdrant gRPC endpoint
default_collection_name = "docs"            # Default collection for searches
embedder_type = "local"                     # "local" (built-in) or "ollama"

# TLS settings
accept_invalid_certs = false                # Set true for self-signed certs

# Example: Production profile with Ollama (remote embedder)
[profiles.production]
qdrant_url = "https://qdrant.example.com:6334"
default_collection_name = "prod_docs"
embedder_type = "ollama"
ollama_url = "https://ollama.example.com"
embedding_model = "nomic-embed-text"        # ← Only for ollama profiles!
accept_invalid_certs = true

# ═══════════════════════════════════════════════════════════
# COLLECTION PROFILES - Collection-specific overrides
# ═══════════════════════════════════════════════════════════
[collections.brain]
name = "agent_memory_v1"                    # Real collection name
description = "My agent's memory"
embedder_type = "local"                     # Override embedder for this collection

[collections.code]
name = "codebase_embeddings_nomic"
embedder_type = "ollama"
embedding_model = "nomic-embed-text"
chunk_size = 1024                           # Override chunk size
use_gpu = false                             # Override GPU usage for this collection

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
| `default_collection_name` | string | **REQUIRED** | Default collection to use if not overridden. Acts as the "Home Base" for this profile. |
| `embedder_type` | string | `"local"` | Embedding backend: `"local"` or `"ollama"` |
| `ollama_url` | string | `"http://localhost:11434"` | Ollama API URL (only for `embedder_type = "ollama"`) |
| `embedding_model` | string | `"nomic-embed-text"` | Ollama model name (**only for** `embedder_type = "ollama"`). If set on a `local` profile, a warning will be displayed and this field will be ignored. |
| `accept_invalid_certs` | bool | `false` | Accept invalid TLS certificates |
| `qdrant_api_key` | string | `null` | Optional API Key for Qdrant authentication |
| `ollama_api_key` | string | `null` | Optional API Key for Ollama proxy authentication |

### Embedder Types

| Type | Model | Dimensions | Requirements |
|------|-------|------------|--------------|
| `local` | AllMiniLM-L6-v2 | 384 | None (ONNX built-in, ~30MB download) |
| `ollama` | configurable | varies | Ollama server running + model pulled |

**Recommendation**: Use `local` for development and portability. Use `ollama` when you need larger/custom models.

### Collection Profiles (`[collections.<name>]`)
 
Define named collections with specific configurations that override the active profile.
 
```toml
[collections.brain]
name = "agent_memory_v1"        # Actual Qdrant collection name
embedder_type = "local"         # Force local embedder
chunk_size = 512                # Force chunk size
 
[collection_aliases]            # Simple redirects
b = "brain"
```
 
Usage: 
- `vecdb search -c brain "query"` (Uses "local" embedder, searchs "agent_memory_v1")
- `vecdb search -c b "query"` (Redirects to "brain", then applies "brain" profile)

#### Collection Profile Options (`[collections.<name>]`)

| Key | Type | Default | Description |
|-----|------|---------|-------------|
| `name` | string | **REQUIRED** | Actual Qdrant collection name |
| `description` | string | `null` | Optional description for listing |
| `embedder_type` | string | `null` | Override active profile's embedder type |
| `embedding_model`| string | `null` | Override active profile's embedding model |
| `ollama_url` | string | `null` | Override Ollama URL |
| `qdrant_api_key` | string | `null` | Override Qdrant API Key |
| `ollama_api_key` | string | `null` | Override Ollama API Key |
| `chunk_size` | integer | `null` | Override ingestion chunk size |
| `max_chunk_size` | integer | `null` | Override max chunk size |
| `chunk_overlap` | integer | `null` | Override chunk overlap |
| `use_gpu` | bool | `null` | Override `local_use_gpu` for this collection |
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
