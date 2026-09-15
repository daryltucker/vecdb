# GPU Acceleration

## Microsoft ONNX Runtime

This allows for accelerated CPU and/or GPU embedding.

## Which GPUs the default build supports

<!-- ort-dist-contract: machine-checked by tests/tier2_ort_distribution.py.
     These describe the ACTUAL prebuilt artifact, read out of the shipped
     provider library with objdump/cuobjdump — not an intention. If a bump
     changes them, update this doc's prose AND these values; never just these.
sm_targets = 75,80,90
pinned_cuda_major = 12
onnxruntime_version = 1.24.2
env = ORT_CUDA_VERSION
-->

**Turing through Hopper — compute capability 7.5, 8.0–8.9, and 9.0.**

vecdb's default build uses a prebuilt ONNX Runtime. That prebuilt contains
compiled kernels for `sm_75`, `sm_80` and `sm_90` only, and ships **no PTX**, so
there is no JIT fallback to cover anything else. CUDA kernels are binary
compatible only within a compute-capability major version, which makes the
supported set exactly:

| Architecture | Compute capability | Default build |
|---|---|---|
| Maxwell | 5.0–5.3 | ❌ too old |
| Pascal | 6.0–6.2 | ❌ too old |
| Volta | 7.0 | ❌ too old |
| **Turing** | **7.5** | ✅ `sm_75` |
| **Ampere** | **8.0–8.7** | ✅ `sm_80` |
| **Ada** | **8.9** | ✅ `sm_80` |
| **Hopper** | **9.0** | ✅ `sm_90` |
| Blackwell | 10.0, 12.0 | ❌ too new |

Outside that window the default build fails loudly — `CUBLAS_STATUS_ARCH_MISMATCH`
or `cudaErrorNoKernelImageForDevice` — and never falls back to CPU silently.
**This is not a dead end in either direction:** see
[`GPU_LEGACY.md`](GPU_LEGACY.md) for the supported bring-your-own ONNX Runtime
path, which works on any architecture your CUDA toolkit can target. A brand-new
Blackwell card needs it for the same reason an old Maxwell one does.

If you are unsure what you have, `nvidia-smi --query-gpu=name,compute_cap --format=csv`.

## Which CUDA runtime you need: `ORT_CUDA_VERSION`

The prebuilt comes in two flavours with **identical GPU support** — they differ
only in which CUDA runtime they link against:

| `ORT_CUDA_VERSION` | Requires | Kernels |
|---|---|---|
| `12` | `libcudart.so.12`, `libcublas.so.12`, `libcudnn.so.9` | `sm_75 sm_80 sm_90` |
| `13` | `libcudart.so.13`, `libcublas.so.13`, `libcudnn.so.9` | `sm_75 sm_80 sm_90` |

Because the card support is the same, pick whichever CUDA you actually have
installed. If you set nothing, the `ort` crate **guesses from the build
machine** — it looks at `CUDA_HOME`, `NV_CUDA_CUDART_VERSION`, and `nvcc
--version`, defaulting to `12` when it finds no evidence of CUDA 13.

That guess is a property of the machine you compile on, not of vecdb, so two
people building the same commit can get binaries with different runtime
requirements. **Set it explicitly** if you care which you get:

```bash
ORT_CUDA_VERSION=12 cargo install --git https://github.com/daryltucker/vecdb --locked vecdb-cli
```

vecdb's own release artifacts and `make` targets pin `12`, because CUDA 12 is
more widely installed and needs a lower minimum driver. Verify what a given
binary wants:

```bash
objdump -p ~/.cargo/bin/libonnxruntime_providers_cuda.so | grep NEEDED
```

### CUDA Provider

> You can offload Embedding to the GPU (optional)

#### Grab the ONNX Version
```bash
$ vecdb --version
vecdb v1.1.1 (git:7ed4cca)
ONNX Runtime: 1.24.2
```

#### Enable GPU Offloading
```bash
$ cat ~/.config/vecdb/config.toml
...
[backend.local]
kind = "fastembed"

[embedder.micro]
backend = "local"
model = "all-minilm-l6-v2"
use_gpu = true          # fastembed only
batch_rows = 1          # ONNX rows per inference
...

```

#### Install the Provider Libraries

`make install` does this for you. It copies `libonnxruntime_providers_shared.so`
and `libonnxruntime_providers_cuda.so` from the build into the install
directory, next to the `vecdb` binary. Nothing else is required.

Two facts make every other install flow wrong (learned the hard way,
BUG-2026-254):

1. **ORT resolves these libraries relative to `dirname(argv[0])`.** Not
   `ldconfig`, not `LD_LIBRARY_PATH` for the initial lookup. Copying them to
   `/usr/local/lib` and running `ldconfig` — what this document used to say —
   is never consulted and cannot work with the statically linked runtime vecdb
   ships. Worse, a bare PATH invocation (`vecdb ...`) makes `argv[0]` a bare
   name and the anchor degrades to the *current working directory*. vecdb
   compensates: when invoked bare and the provider libs are installed next to
   the real binary, it re-execs itself once with an absolute `argv[0]`
   (`reexec_for_ort_provider_anchor`), so both invocation styles work.
2. **They must come from the exact ONNX Runtime build the binary was compiled
   against.** vecdb links pyke's static ORT (`download-binaries`); the provider
   `.so`s from that same artifact are symlinked into `target/release/` at build
   time (ort's `copy-dylibs` feature), and `make install` copies those. Pairing
   the binary with provider libs from the official Microsoft release tarball —
   or from a different pyke build — aborts the process
   (`free(): invalid pointer`) inside the provider's initializer.

Verify after install:

```bash
ls ~/.cargo/bin/libonnxruntime_providers_*.so
vecdb status    # warns if GPU is requested but the libs are missing
```

The CUDA runtime itself (`libcublas`, `libcudart`, `libcudnn`, …) is a normal
dependency of `libonnxruntime_providers_cuda.so` and IS found via the regular
loader search path (`ldconfig` / `LD_LIBRARY_PATH`) — which is why the
`ORT_CUDA_VERSION` you built with has to match a CUDA you actually have.

Two distinct failures, often confused:

- **`cudart`/`cublas` cannot be loaded at all** — the CUDA major version the
  binary was built for is not installed. Fix by installing that CUDA, or
  rebuild with the other `ORT_CUDA_VERSION`.
- **`CUBLAS_STATUS_ARCH_MISMATCH` or `cudaErrorNoKernelImageForDevice`** — CUDA
  loaded fine, but your card is outside the `sm_75`/`sm_80`/`sm_90` window
  above. Changing `ORT_CUDA_VERSION` will **not** help; both flavours carry the
  same kernels. This is what `GPU_LEGACY.md` exists for.

vecdb reports either as a hard error rather than falling back to CPU silently.

**Cards outside the window are not abandoned, old or new:** see
`docs/GPU_LEGACY.md` for the supported bring-your-own ONNX Runtime path
(`cuda-dynamic` feature + `ORT_DYLIB_PATH`), which works on any architecture
your CUDA toolkit can target.

## Performance Characteristics

During ingestion, you may observe a period of high CPU and RAM usage before the GPU starts processing. This is expected behavior due to the architecture of the ingestion pipeline:

1.  **Discovery Phase**: The system first scans the entire target directory to count files and build a processing queue. This is a CPU-intensive serial operation.
2.  **Memory Loading**: To ensure high-quality parsing (via `vecq`), files smaller than 50MB are loaded entirely into memory as strings. If you are processing a directory with many medium-sized files, RAM usage will spike during this load window. 
3.  **Tokenization Latency**: Before the GPU can compute embeddings, the texts must be converted into numerical tokens. This "Tokenization" phase runs on the CPU and processes the entire batch (default 20 chunks) before handing the tensors to the GPU.

> [!TIP]
> If your system has many CPU cores but a smaller GPU, you may want to increase `concurrency` while keeping `gpu_concurrency` at 1 to balance the load.