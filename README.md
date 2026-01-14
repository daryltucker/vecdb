# vecdb

> **The Vector Database for Agents & Humans.**
> *Configuration-driven, backend-agnostic, and built for the future.*

`vecdb` is a dual-interface vector database system:
1.  **MCP Server**: Connects to AI agents (Claude, IDEs, etc.) via the Model Context Protocol.
2.  **CLI Tool**: Gives humans and scripts direct power over their vector indices.
3.  **Vecq**: A specialized CLI for structural code querying (jq for code).

Uses **Qdrant** as the robust storage backend.

---

## 🚀 Quick Start

```bash
install.sh
vecdb ingest ./
docsize "How do I install use vecq?"
```

### 1. Installation

**Option A: Install via Cargo (Recommended)**
```bash
cargo install --git https://github.com/daryltucker/vecdb vecdb-cli vecdb-server vecq docsize
```

**Option B: Build from Source**

```bash
(.venv)  [ v0.0.9 ✭ | ● 174 ✚ 16 ]
✔ 23:09 daryl@Sleipnir ~/Projects/NRG/vecdb $ ./install.sh
=== Installing vecdb binaries ===
Target: ~/.cargo/bin

[1/4] Installing vecq (jq for source code)...
[2/4] Installing vecdb (CLI)...
[3/4] Installing vecdb-server (MCP)...
[4/4] Installing docsize (LLM context tool)...

=== Installation Complete ===
Installed:
  - vecq         (jq for source code)
  - vecdb        (CLI tool)
  - vecdb-server (MCP server)
  - docsize      (LLM context tool)

Verify with: vecq --help && vecdb --help && docsize --help
=== Autocomplete Setup ===
Detected bash. To enable autocomplete, add this to your /home/daryl/.bashrc:

  # vecdb completions
  [ -f "/home/daryl/.local/share/vecdb/completions/vecdb" ] && . "/home/daryl/.local/share/vecdb/completions/vecdb"
  [ -f "/home/daryl/.local/share/vecdb/completions/vecq" ] && . "/home/daryl/.local/share/vecdb/completions/vecq"

Would you like me to add this to your /home/daryl/.bashrc now? (y/N) y
Added to /home/daryl/.bashrc. Please restart your shell or run: source /home/daryl/.local/share/vecdb/completions/vecdb && source /home/daryl/.local/share/vecdb/completions/vecq
Tip: Use './install.sh --verbose' to see compilation output
```

See [docs/BUILDING.md](docs/BUILDING.md).

**Python Environment (for tests & tools)**

The project includes several orchestration and test scripts that require Python 3.10+. It is recommended to use a virtual environment:

```bash
python3 -m venv .venv
source .venv/bin/activate
pip install -r requirements.txt
```

### 2. Initialization

Run the initialization command to set up your configuration:
```bash
vecdb init
# Creates ~/.config/vecdb/config.toml
```

### 3. Start Qdrant (Vector Database)

You need a running Qdrant instance.

**Option A: Using Docker (Recommended)**
Use a meaningful Docker Volume for persistence:
```bash
docker run -d -p 6333:6333 \
    -v vecdb_qdrant_data:/qdrant/storage \
    qdrant/qdrant
```

**Option B: Manual / Cloud**
Install/Sign-up at [qdrant.tech](https://qdrant.tech/documentation/quick-start/).
Then update your config:
Edit your config manually:
```bash
nano ~/.config/vecdb/config.toml
```

### 4. Basic Usage

**Ingest your documents:**
```bash
# Ingest a directory with concurrency control
vecdb ingest ./docs --collection my_knowledge -P 4 -G 2

# Note: Ingestion is OOM-protected. 
# -P, --concurrency: Max parallel file processing tasks.
# -G, --gpu-concurrency: Max GPU embedding batch size (Prevents VRAM spikes).
```
## ⚡ CUDA Support

By default, `vecdb` is built with CUDA support enabled (via `ort` static linking).

1.  **Prerequisites**:
    *   NVIDIA Drivers (v550+ recommended)
    *   **NVIDIA CUDA Toolkit** (`sudo apt install nvidia-cuda-toolkit`)
    *   **NVIDIA cuDNN** (`sudo apt install nvidia-cudnn`) - Required for runtime execution.

2.  **Configuration**:
    *   Set `local_use_gpu = true` in `~/.config/vecdb/config.toml` (default).
    *   **No manual library paths needed**: The ONNX Runtime is statically linked into the binary.

### Opting Out (CPU Only)
If you do not need GPU support or want to reduce binary size, you can disable the default CUDA features during build:

```bash
cargo install --path vecdb-cli --no-default-features
```

> **Note**: `vecdb` uses `ort` with static linking. You do **not** need to set `LD_LIBRARY_PATH` or manually manage `libonnxruntime.so` files.

### File Ignoring (`.vectorignore`)

`vecdb` supports two ways to exclude files:

1.  **`.vectorignore`** (Always Respected):
    *   Works exactly like `.gitignore`.
    *   Place it in your project root or subdirectories.
    *   Example: `vecdb-asm/` or `*.secret`.

2.  **`.gitignore`** (Optional):
    *   Use `--respect-gitignore` to also respect your git rules.
    *   Disabled by default to allow ingesting code you might not commit (e.g., local docs).

> **Tip**: See [docs/CONFIG.md](docs/CONFIG.md) for advanced ignore rules.

**Search:**
```bash
# Standard semantic search
vecdb search "How do I configure profiles?" --collection my_knowledge

# Smart routing (multi-hop / filter detection)
vecdb search "latest rust files" --smart

# Pipe-friendly JSON output
vecdb search "auth policy" --json | jq .
```

**Check Status:**
```bash
vecdb list
vecdb status
```

**Quantization Management:**
```bash
# Set Int8 quantization for a collection (persistent config)
vecdb config set-quantization my_coll scalar

# Apply optimization explicitly
vecdb optimize my_coll

# Check warnings for memory usage
vecdb list
```

**More Examples**: See [docs/EXAMPLES.md](docs/EXAMPLES.md) and [docs/CLI.md](docs/CLI.md).

---

## 🤖 MCP Server (Agent) Usage

To use with an MCP client (like Claude Desktop or an IDE):

**Command**: `vecdb-server`
**Arguments**: `--allow-local-fs` (Optional, enables `ingest_path` tool)

**Available Tools**:
*   `search_vectors`: Semantic search.
*   `embed`: Generate embeddings.
*   `ingest_path`: Ingest local files/folders.
*   `ingest_historic_version`: Time-travel ingestion (Git).
*   `code_query`: Analyze code structure with `vecq` (supports `-f`, `-L` flags). Now supports comment-aware queries for Rust!

See [docs/MCP_SERVER.md](docs/MCP_SERVER.md) for API details.

---

## 📚 Documentation

*   **[EXAMPLES.md](docs/EXAMPLES.md)**: Common usage patterns and tricks.
*   **[CONFIG.md](docs/CONFIG.md)**: Full configuration reference.
*   **[BUILDING.md](docs/BUILDING.md)**: Compile from source.
*   **[Vecq Guide](docs/vecq/README.md)**: Manual for the `vecq` code query tool.
*   **Specs**: Detailed feature modules in `docs/specs/` (e.g. [Ingestion Design](docs/specs/INGESTION_DESIGN.md)).

## 🤝 Contributing & support

*   **Bug Reports**: Please file an issue on GitHub.
*   **License**: Business Source License 1.1 (Free for <$1M Revenue). See [LICENSE](LICENSE).

---

> *"Configuration drives. Abstraction enables. Philosophy guides. Code follows."*