# Building Vecdb from Source

`vecdb` is written in Rust. You can build it easily using `cargo`.

## Prerequisites

*   **Rust Toolchain**: 1.75.0 or later.
    *   Install via rustup: `curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh`
*   **Git**: To clone the repository.
*   **Build Tools**: Standard build-essential (Linux) or Xcode Command Line Tools (macOS).

## Step-by-Step Build

1.  **Clone the Repository**:
    ```bash
    git clone https://github.com/yourusername/vecdb.git
    cd vecdb
    ```

2.  **Build Release Binaries**:
    ```bash
    # Standard CPU build
    cargo build --release

    # GPU-enabled build (NVIDIA CUDA)
    cargo build --release --features cuda
    ```
    *This compiles `vecdb` (CLI), `vecdb-server` (MCP), and `vecq`.*

3.  **Locate Binaries**:
    The compiled binaries will be in `target/release/`:
    *   `target/release/vecdb`
    *   `target/release/vecdb-server`
    *   `target/release/vecq`

4.  **Install (Optional)**:
    You can install them to your `~/.cargo/bin` path:
    ```bash
    ./install.sh
    # Or manually:
    cargo install --path vecdb-cli
    cargo install --path vecdb-server
    ```

## Cross-Compiling

We support cross-compilation via `cross` or standard cargo targets if you have the linkers installed.

**Common Targets**:
*   `x86_64-unknown-linux-gnu` (Linux)
*   `x86_64-pc-windows-msvc` (Windows)
*   `aarch64-apple-darwin` (macOS Apple Silicon)

## Hardware Acceleration (CUDA)

To leverage an NVIDIA GPU for faster local embeddings:

### Prerequisites (for CUDA)
*   **NVIDIA Drivers**: Version 525+ recommended.
*   **CUDA Toolkit**: Installed on the build machine (for linking) and runtime machine.
*   **Linux**: Currently supported and tested on Linux.

### Enabling at Build Time
Add the `--features cuda` flag to any `cargo` command:
```bash
cargo check --features cuda
cargo build --release --features cuda
```

### Enabling at Runtime
Once built with CUDA support, enable it in your `config.toml`:
```toml
local_use_gpu = true
```

## Development Environment

### 1. Vector Database (Qdrant)
For development, we use a local Qdrant instance with data stored in `.qdrant_storage` (gitignored).

```bash
# Start dev instance
docker-compose -f tools/dev-qdrant.yml up -d
```

### 2. Embeddings (Ollama or Local)

## Development Mode

For faster incremental builds during development:
```bash
cargo build
./target/debug/vecdb --help
```

## Packaging & Assets

The project includes icon assets in the `assets/` directory:
- `assets/vecdb.png`
- `assets/vecq.png`

**Note**: Rust binaries (ELF) do not embed icons. These assets are provided for external packaging systems (e.g., `.desktop` files, AppImage, or distribution packages) to define the application icon.

