# Embedding Model Guide

`vecdb-mcp` supports two primary methods for generating embeddings: **Local (CPU)** and **Remote (Ollama/API)**.

## 1. Local Embedder (`fastembed`)
This is the default for a zero-config experience. It runs entirely on your CPU using [fastembed-rs](https://github.com/qdrant/fastembed-rs).

### Configuration
You can change the local model globally in your `config.toml`:

```toml
# ~/.config/vecdb/config.toml
local_embedding_model = "bge-micro-v2" # Fast, low RAM
# OR
local_embedding_model = "nomic-embed-text-v2-moe" # 8192 tokens, Matryoshka support
```

### Model Management
Models are **not shipped** with the binary. They are downloaded automatically on first use and stored in:
`~/.config/vecdb/fastembed_cache`

---

## 2. Remote Embedder (`ollama`)
Best for high-fidelity models (like Qwen3) that require more VRAM or should run on a powerful host like the **Blade**.

### Configuration
Create a profile in `config.toml`:

```toml
[profiles.blade]
qdrant_url = "http://vector-server.lan:6334"
ollama_url = "http://vector-server.lan:11434"
embedder_type = "ollama"
embedding_model = "nomic-embed-text" # Or "qwen3-embedding-4b"
```

---

## 3. Recommended Models (2026)

| Model | Type | Best For | Context |
| :--- | :--- | :--- | :--- |
| **`bge-micro-v2`** | Local | Ultra-fast search, low RAM | 512 tokens |
| **`nomic-v2-moe`** | Local | Large files, high fidelity | 8192 tokens |
| **`Qwen3-4B`** | Ollama | Multilingual, state-of-the-art | 32k tokens |

### Note on Matryoshka Learning
Models like **Nomic v2** allow you to truncate vectors (e.g., from 768 down to 128) without significant accuracy loss. 

*   **Portability Hack**: Generate high-dim vectors on your **Blade** or **GPU**, but search using low-dim "short vectors" on your **mobile device** or **CPU**.
*   **Speed**: Short vectors are vastly faster to compare in Qdrant (less floating point math).

---

## 4. Strategic Guidance: When to "Heavy Lift"?

| Scenarios | Recommended Model | Rationale |
| :--- | :--- | :--- |
| **Routine Ingestion** | `bge-micro-v2` | Fast, low cost, perfectly fine for 80% of standard code. |
| **Complex Architecture** | `nomic-v2-moe` | 8k token context allows the model to "see" the whole file structure. (`nomic-embed-text-v2-moe`) |
| **Deep Reasoning/RAG** | `Qwen3-4B` | Use when semantic nuance is critical (e.g., "Find where we violate safety law #3"). |
| **Cross-Language** | `Qwen3-4B` | Superior multilingual "common sense." |

## 5. Hardware Optimization: Using your 4GB GPU

Even with 4GB VRAM, you have an "untouched" asset. 

### Enabling CUDA
The `LocalEmbedder` uses ONNX Runtime. While it defaults to CPU for maximum compatibility, it can be recompiled to use your NVIDIA GPU via `ort-cuda`.

**Why go GPU for embeddings?**
1. **Parallelism**: GPUs can embed thousands of tokens in the time the CPU does hundreds.
2. **Offloading**: Keeps your CPU free for compilation, background agents, and OS responsiveness.
3. **Large Batches**: If you are ingesting a 10,000-file project, the GPU will finish in minutes while the CPU takes an hour.

### The "Matryoshka" Workflow
1. **Ingest (GPU/Blade)**: Generate full 768-dim embeddings using **Nomic v2**.
2. **Store**: Persist full vectors in Qdrant.
3. **Query (CPU)**: When searching, truncate your query vector to 128 or 256. Qdrant can perform "Approximate Nearest Neighbor" on these truncated vectors at light-speed.
