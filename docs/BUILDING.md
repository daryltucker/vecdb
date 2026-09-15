# Building Vecdb from Source

`vecdb` is written in Rust. You can build it easily using `cargo`.

## Prerequisites

*   **Rust Toolchain**: 1.75.0 or later.
    *   Install via rustup: `curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh`
*   **Git**: To clone the repository.
*   **Build Tools**: Standard build-essential (Linux) or Xcode Command Line Tools (macOS).

**Python Environment (for tests & tools)**

The project includes several orchestration and test scripts that require Python 3.10+. It is recommended to use a virtual environment:

```bash
python3 -m venv .venv
source .venv/bin/activate
pip install -r requirements.txt
```

## Step-by-Step Build

1.  **Clone the Repository**:
    ```bash
    git clone https://github.com/yourusername/vecdb.git
    cd vecdb
    ```

2.  **Build Release Binaries**:
    ```bash
    # Default build — CUDA support is ON (`cuda` is a default feature)
    cargo build --release

    # CPU-only build (smaller, no ONNX CUDA provider)
    cargo build --release --no-default-features
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
    make install          # also copies the CUDA provider libs — see GPU.md
    # Or manually (no GPU provider libs):
    cargo install --path vecdb-cli
    cargo install --path vecdb-server
    ```
    `./install.sh` is a contributor convenience wrapper around the same
    `cargo install` calls; it does not set up GPU support.

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
*   **A supported GPU**: compute capability 7.5, 8.0–8.9, or 9.0. Older
    (Maxwell/Pascal/Volta) and newer (Blackwell) cards need
    [`GPU_LEGACY.md`](GPU_LEGACY.md). See [`GPU.md`](GPU.md) for the matrix.

### Enabling at Build Time
Nothing to enable — `cuda` is a **default** feature of `vecdb-core`,
`vecdb-cli` and `vecdb-server`. A plain `cargo build --release` is a CUDA
build. Use `--no-default-features` to opt out.

Because `cuda` is on by default, `--version` cannot tell you whether a binary
is a CUDA build; check for the provider libraries instead (see `GPU.md`).

### Choosing the CUDA major: `ORT_CUDA_VERSION`
The prebuilt ONNX Runtime is fetched in a `cu12` or `cu13` flavour. They carry
**identical GPU kernels** and differ only in which CUDA runtime they link
(`libcudart.so.12` vs `.13`). Left unset, the `ort` crate infers it from the
build machine's `CUDA_HOME` / `NV_CUDA_CUDART_VERSION` / `nvcc --version`,
which makes the resulting binary's runtime requirement a property of your
workstation rather than of the commit. Pin it:

```bash
ORT_CUDA_VERSION=12 cargo build --release
```

vecdb's `make` targets and release workflow pin `12`.
`tests/tier2_ort_distribution.py` asserts what the built artifact actually
links and which SMs it carries, so drift is caught rather than shipped.

### Enabling at Runtime
Once built with CUDA support, enable it in your `config.toml`:
```toml
[embedder.<name>]
use_gpu = true          # fastembed backends only
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

## Make targets

```bash
make check          # cargo check + clippy -D warnings
make test-rust      # cargo test --workspace — fast, NOT the release gate
make tests          # tests/run_all.sh — the release gate
make doc            # cargo doc --no-deps --open

make install                 # cargo install --path vecdb-cli --force
make install-cuda-dynamic    # BYO-ONNX-Runtime machines — see docs/GPU_LEGACY.md

make build          # Docker image
make run            # Docker, interactive
make run-stdio      # Docker, MCP stdio mode
```

Run a single Rust test:

```bash
cargo test -p vecdb-core test_model_selection_nomic_v15
```

> On a machine using the `cuda-dynamic` legacy-GPU build, `make install`
> **overwrites it with a static build** and GPU embedding then fails with
> `CUBLAS_STATUS_ARCH_MISMATCH`. Use `make install-cuda-dynamic` there.

## Debug output

| variable | effect |
|---|---|
| `VECDB_DEBUG=1` | `[LocalEmbedder]` thread diagnostics and other debug prints |
| `RUST_LOG=debug` | tracing spans across the workspace |
| `ORT_DYLIB_PATH` | overrides the `ort_dylib_path` config key (legacy-GPU builds) |

## Packaging & Assets

The project includes icon assets in the `assets/` directory:
- `assets/vecdb.png`
- `assets/vecq.png`

**Note**: Rust binaries (ELF) do not embed icons. These assets are provided for external packaging systems (e.g., `.desktop` files, AppImage, or distribution packages) to define the application icon.

