# ONNX & ORT Integration

## Overview

**ONNX** (Open Neural Network Exchange) is an open format built to represent machine learning models. It defines a common set of operators - the building blocks of machine learning and deep learning models - and a common file format to enable AI developers to use models with a variety of frameworks, tools, runtimes, and compilers.

*   [Official Website](https://onnx.ai/)
*   [GitHub Repository](https://github.com/onnx/onnx)

**ort** is a Rust wrapper for ONNX Runtime. It provides a safe, idiomatic Rust interface to run ONNX models on various backends (CPU, CUDA, CoreML, etc.).

*   [Crate Documentation (ort)](https://docs.rs/ort/latest/ort/)
*   [Pyke.io ORT Guide](https://ort.pyke.io/)

## Version 2.0 Configuration

As of `ort 2.0.0-rc.11` (2026), the strategy for achieving seamless, statically linked binaries has evolved significantly from the 1.x series.

### Static Linking Guide

To produce a portable binary that includes the ONNX Runtime statically (removing dependencies on system `libonnxruntime.so` or `LD_LIBRARY_PATH`), specific configuration is required.

#### 1. Cargo Dependencies
We use `download-binaries` to fetch the correct static artifacts during the build.

> [!WARNING]
> The `ort` crate version (e.g. 2.0.0-rc.9, 1.16) DOES NOT MATCH the underlying ONNX Runtime library version (e.g. 1.19.2, 1.20.0).
> **ALWAYS check `vecdb --version` to see the actual runtime version required.**

```toml
[dependencies]
ort = {
    version = "2.0.0-rc.11",
    default-features = false,
    features = [
        "std",
        "ndarray",
        "tracing",
        "download-binaries", # Fetches artifacts
        "cuda",              # GPU support
        "tls-native-vendored" # Required for robust static downloading
    ]
}
```

#### 2. Build Configuration (Automated)
We use a `.cargo/config.toml` file in the repository root to automatically set the required environment variable for the build process.

```toml
# .cargo/config.toml
[env]
ORT_STRATEGY = "static"
```

This ensures that `cargo build`, `cargo install`, and other Cargo commands automatically use the static linking strategy without forcing the user to manually export environment variables.

To manually override or test different strategies, you can still set the variable in your shell (which takes precedence over the config file):
```bash
ORT_STRATEGY=dynamic cargo build
```

#### 3. Verification
You can verify the static link by inspecting the final binary with `ldd` (Linux). `libonnxruntime.so` should **not** appear in the output.

```bash
ldd target/release/vecdb | grep onnx
# Should return nothing (exit code 1)
```
