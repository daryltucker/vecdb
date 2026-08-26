# vecdb

> **The Vector Database for Agents & Humans.**
> *Configuration-driven, backend-agnostic, and built for the future.*

`vecdb` is a dual-interface vector database system:
1.  **MCP Server**: Connects to AI agents (Claude, IDEs, etc.) via the Model Context Protocol.
2.  **CLI Tool**: Gives humans and scripts direct power over their vector indices.
3.  **Vecq**: A specialized CLI for structural code querying (jq for code).

Uses **Qdrant** as the robust storage backend.

`vecq` is now available as a standalone tool! [Read the Guide](docs/vecq/README.md).

---

## 🚀 Quick Start

```bash
install.sh
vecdb ingest ./
docsize "How do I install and use vecq?"
```

### 1. Installation

Two ways in. Both need `--git`: vecdb is **not published on crates.io**, so a
bare `cargo install vecdb-cli` will not find it.

#### Option A — `cargo binstall` (prebuilt, seconds)

Downloads the binaries built by CI for your platform. **No compiler, no build.**
This is the path for Raspberry Pi and anything else where compiling ONNX Runtime
is measured in hours.

```bash
cargo binstall --git https://github.com/daryltucker/vecdb --locked -y vecdb-cli
cargo binstall --git https://github.com/daryltucker/vecdb --locked -y vecdb-server
cargo binstall --git https://github.com/daryltucker/vecdb --locked -y vecq
```

One command per crate: binstall rejects `--git` together with multiple package
names (`You cannot use --git and specify multiple packages at the same time`).
`--manifest-path` has the same restriction.

Don't have it? `cargo install cargo-binstall`, or grab a prebuilt binstall from
its own releases — same idea, one level up.

Add `--force` to reinstall over an existing copy.

Prebuilt binaries exist for:

| platform | target |
|---|---|
| Linux x86-64 | `x86_64-unknown-linux-gnu` |
| Linux ARM64 (Raspberry Pi 4/5, 64-bit OS) | `aarch64-unknown-linux-gnu` |
| macOS Apple Silicon | `aarch64-apple-darwin` |
| Windows x86-64 | `x86_64-pc-windows-msvc` |

Anything else — 32-bit Raspberry Pi OS, musl, FreeBSD — has no artifact; use
Option B.

> **This will never quietly start compiling.** `disabled-strategies =
> ["compile"]` is set, so if no prebuilt artifact matches your target binstall
> errors out instead of silently falling back to a source build — which on an
> ARM board is the difference between 30 seconds and several days, with nothing
> on screen to tell you which one you got.
>
> To build from source deliberately, use Option B, or override:
> `cargo binstall --strategies crate-meta-data,compile --git … vecdb-cli`

#### Option B — `cargo install` (from source)

```bash
cargo install --git https://github.com/daryltucker/vecdb --locked vecdb-cli vecdb-server vecq
```

`cargo install` has no such restriction — all three in one command.

> ⚠️ **Always use `--locked` when installing from git.** It pins dependency
> versions (including the ONNX Runtime binary) to the workspace `Cargo.lock`.
> Without it, cargo may resolve newer dependencies that download incompatible
> prebuilt binaries.

`docsize` is an example client rather than part of the core toolchain, so it is
deliberately not in either block. Add it separately if you want it:
`cargo install --git https://github.com/daryltucker/vecdb --locked docsize`

#### Verify

```bash
vecdb --version
vecdb-server --version
vecq --version
```

Prints `vecdb vX.Y.Z (git:<sha>)`. The revision is stamped at build time, so it
names the commit the binary was actually built from — not whatever happens to be
checked out.

**Auto-completions for Cargo Installs**
If you installed via `cargo install` or `cargo binstall`, you can generate shell completions manually:
Bash:

```bash
mkdir -p ~/.local/share/bash-completion/completions/
vecdb completions bash > ~/.local/share/bash-completion/completions/vecdb
```

Zsh:

```bash
mkdir -p ~/.zfunc
vecdb completions zsh > ~/.zfunc/_vecdb
```

Then add to `~/.zshrc`: `fpath=(~/.zfunc $fpath); autoload -Uz compinit; compinit`

> See `install.sh` for more install options

### 3. Start Qdrant (Vector Database)

You need a running Qdrant instance.

**Option A: Using Docker (Recommended)**
Use a meaningful Docker Volume for persistence:
```bash
docker run -d -p 6333:6333 \
    -v vecdb-data:/qdrant/storage \
    qdrant/qdrant
```

See [Examples README.md](examples/README.md#qdrant) and [docker-compose.qdrant](examples/docker-compose.qdrant)

**Option B: Manual / Cloud**
Install/Sign-up at [qdrant.tech](https://qdrant.tech/documentation/quick-start/).
Then update your config:
Edit your config manually:
```bash
vim ~/.config/vecdb/config.toml
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

By default, `vecdb` is built with CUDA support enabled. The ONNX Runtime is
downloaded as prebuilt shared libraries and dynamically loaded at runtime.

1.  **Prerequisites**:
    *   NVIDIA Drivers (v550+ recommended)
    *   **NVIDIA CUDA Toolkit** (`sudo apt install nvidia-cuda-toolkit`)
    *   **NVIDIA cuDNN** (`sudo apt install nvidia-cudnn`) - Required for runtime execution.

2.  **Install with `--locked`**:
    ```bash
    cargo install --git https://github.com/daryltucker/vecdb --locked vecdb-cli
    ```
    The workspace `Cargo.lock` pins `ort-sys 2.0.0-rc.11` which downloads
    the ONNX Runtime 1.23.2 CUDA binary. Building without `--locked` may resolve
    a newer `ort-sys` that downloads an incompatible ORT binary. See
    [docs/internal/ORT_BINARY_DEPENDENCY.md](docs/internal/ORT_BINARY_DEPENDENCY.md).

3.  **Configuration**:
    *   Set `use_gpu = true` on the `[embedder.<name>]` you use, in
        `~/.config/vecdb/config.toml`. It is a fastembed knob — Ollama's device
        placement is the server's business, not vecdb's.

> **Tip**: GPU is really not required, and you will still benefit from `vecdb` when using the CPU embeddings. However, this feature is here for those who want or need it.

### Opting Out (CPU Only)
If you do not need GPU support or want to reduce binary size, you can disable the default CUDA features during build:

```bash
cargo install --path vecdb-cli --no-default-features
```

### File Ignoring (`.vectorignore`)

`vecdb` supports two ways to exclude files:

1.  **`.vectorignore`** (Respected by default):
    *   Works exactly like `.gitignore`.
    *   Place it in your project root or subdirectories.
    *   Example: `vecdb-asm/` or `*.secret`.
    *   Use `--ignore-vectorignore` to skip `.vectorignore` rules entirely
        (ingests everything regardless of ignore patterns).

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

 **Tip**: `vecdb search` returns richly-scored results with full content and metadata.  Use `docsize` for context-aware relevance ranking that shows what these embeddings can do for your Agent (Even 1B or 4B models).

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
*   `search_vectors`: Semantic search with smart routing.
*   `code_query`: AST-aware structural code search.
*   `project_overview`: Full-project AST analysis with architecture graph + Mermaid diagram.
*   `embed`: Generate embeddings from text.
*   `ingest_path`: Ingest local files/folders.
*   `ingest_historic_version`: Time-travel ingestion (Git).
*   `list_collections`: List collections with stats and compatibility info.
*   `delete_collection`: Delete a collection with safety confirmation.
*   `get_job_status`: Check background job progress.

### Claude Code (User-Global)

```bash
claude mcp add --scope user vecdb \
  -e VECDB_PROFILE=default \
  -e VECDB_ALLOW_LOCAL_FS=true \
  -- vecdb-server --stdio
```

### Centralized HTTP Server (Recommended for Multiple Agents)

If you use multiple MCP agents (e.g., Claude Desktop, Cursor, and Terminal tools), they normally would each spawn their own `vecdb-server` over stdio. This causes multiple processes to waste RAM and compete for VRAM.

Instead, you can run a single `vecdb-server` in HTTP mode and have all your agents talk to it:

1. **Start the Central Server:**
   ```bash
   vecdb-server --port 3000 --allow-local-fs
   ```
2. **Configure your Agents to connect via HTTP / SSE:**
   If your agent supports HTTP transport, point it to `http://localhost:3000`.
   If it only supports `stdio` (like Claude Desktop), use an [MCP Proxy](https://github.com/daryltucker/mcp-proxy) to bridge stdio to the HTTP instance without spawning another resource-heavy `vecdb-server`.

See [docs/MCP_SERVER.md](docs/MCP_SERVER.md) for more details.

---

## 📚 Documentation

*   **[EXAMPLES.md](docs/EXAMPLES.md)**: Common usage patterns and tricks.
*   **[CONFIG.md](docs/CONFIG.md)**: Full configuration reference.
*   **[BUILDING.md](docs/BUILDING.md)**: Compile from source.
*   **[vecq Guide](docs/vecq/README.md)**: Manual for the `vecq` code query tool.
*   **Specs**: Detailed feature modules in `docs/specs/` (e.g. [Ingestion Design](docs/specs/INGESTION_DESIGN.md)).

## 🧪 Testing

The project uses a tiered testing framework. It is **mandatory** to run the complete test suite before any release or major changes.

```bash
# Run the COMPLETE test suite (All tiers, no exceptions - Release Blocker)
make tests

# Run Rust-only tests (Unit & Integration)
make test-rust
```

## 🤝 Contributing & support

*   **Bug Reports**: Please file an issue on GitHub.
*   **License**: Business Source License 1.1 (Free for <$1M Revenue). See [LICENSE](LICENSE).

---

> *"Configuration drives. Abstraction enables. Philosophy guides. Code follows."*