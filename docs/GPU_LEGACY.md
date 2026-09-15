# GPU Acceleration Outside the Prebuilt's Range

**Who this is for:** anyone whose GPU is not Turing, Ampere, Ada or Hopper —
that is, compute capability outside `7.5`, `8.0`–`8.9`, `9.0`. In practice:

- **Older than the window** — Maxwell (5.x), Pascal (6.x), Volta (7.0).
- **Newer than the window** — Blackwell (10.x, 12.x), including the RTX 50
  series. A brand-new card lands here for exactly the same reason an old one
  does; this is not a "legacy" document despite the filename.

The default vecdb build cannot use these GPUs, but a supported opt-in path
exists: **bring your own ONNX Runtime**, built for your card, loaded at
runtime. This document is that path. See [`GPU.md`](GPU.md) for the full
support matrix and how to tell which case you are in.

<!-- gpu-legacy-contract: machine-checked by tests/tier2_gpu_docs.py.
     If a version bump or rename makes the test fail, update BOTH this doc's
     prose and these values to match reality — never just the values.
feature = cuda-dynamic
env = ORT_DYLIB_PATH
ort_api_version = 24
onnxruntime_version = 1.24.4
ort_crate = 2.0.0-rc.12
-->

## Why the default build can't do it

Two hard constraints, both learned empirically (BUG-2026-254):

1. **The prebuilt ONNX Runtime carries kernels for `sm_75`, `sm_80` and
   `sm_90` only, and no PTX.** vecdb's default build statically links pyke's
   ORT. Read straight out of the shipped
   `libonnxruntime_providers_cuda.so` with `cuobjdump -lelf`, that is the
   entire kernel set; with no PTX embedded there is no JIT fallback either.
   CUDA cubins are binary compatible only within a compute-capability major
   version, so anything outside 7.5/8.x/9.0 fails at session creation —
   `CUBLAS_STATUS_ARCH_MISMATCH` on Maxwell, `cudaErrorNoKernelImageForDevice`
   elsewhere. Loudly, by design.

   > **`ORT_CUDA_VERSION` is not a workaround for an unsupported card.** The
   > `cu12` and `cu13` prebuilts were measured to carry the identical kernel
   > set; the CUDA major version decides only which `libcudart`/`libcublas`
   > you need installed. See [`GPU.md`](GPU.md) for what that knob is for.

2. **You cannot swap in individual provider libraries.** The
   `libonnxruntime_providers_*.so` files must come from the exact ORT build
   the binary links. Pairing the static runtime with provider libs from any
   other build — including Microsoft's official release tarball — aborts the
   process (`free(): invalid pointer`) in the provider's initializer.

So the only correct fix is to replace the *entire* runtime with a
self-consistent one built for your card. That is what the `cuda-dynamic`
feature does.

## The `cuda-dynamic` build

`cuda-dynamic` switches the `ort` crate to `load-dynamic`: instead of
statically linking pyke's runtime, vecdb dlopens the `libonnxruntime.so` you
point it at via the `ORT_DYLIB_PATH` environment variable. The CUDA provider
libraries are then resolved next to *that* `.so` — a real shared object with
a real path — so the argv\[0\]/cwd anchoring problems of the static build do
not apply, and neither does `make install`'s provider-lib copying.

It is deliberately **not** the default: the default build must keep working
with zero external libraries, and a `cuda-dynamic` binary cannot start
embedding without a usable `ORT_DYLIB_PATH`.

## Steps

### 1. Prerequisites

- **A CUDA toolkit that can target your card**, and **cuDNN 9** for it.
  - *Older than the window* — CUDA **12.x**, which still supports SM 5.0+.
    CUDA 13 dropped Maxwell, Pascal and Volta, so it cannot build kernels for
    them at all.
  - *Newer than the window* — CUDA **12.8+** or 13.x. Blackwell (`sm_100`,
    `sm_120`) was introduced in 12.8; older toolkits cannot target it.
- A driver that supports both your card and that toolkit. For Maxwell/Pascal/
  Volta the **r580 branch is the final driver branch** that supports them —
  stay on it.
- Your card's compute capability (SM). Examples: GTX 9xx = `52`,
  GTX 10xx = `61`, Titan V = `70`, RTX 50xx = `120`. Look yours up:
  <https://developer.nvidia.com/cuda-gpus>

### 2. Build ONNX Runtime for your card

**The contract is the ORT *API version*, not the version string.** The `ort`
crate requests the API version its enabled `api-N` features add up to —
currently **24** (fastembed pins `ort/api-24`). Your `libonnxruntime.so` must
answer `GetApi(24)`, which means upstream tag **v1.24.x**. Check `GetApi`,
not the version string: a build can report a string on one release line while
answering a different API number, and it is the API number ort requires. The
probe below is the only reliable test.

> **Warning (ort 2.0.0-rc.12 bug):** if the library answers the wrong API
> version, vecdb does not error — it **hangs forever**. ort's error
> constructor re-enters its own API-init lock (self-deadlock while formatting
> the "unsupported version" message). A hung first embed with a BYO runtime
> means API-version mismatch until proven otherwise.

```bash
git clone --branch v1.24.4 --depth 1 --recursive --shallow-submodules \
  https://github.com/microsoft/onnxruntime
cd onnxruntime
./build.sh --config Release --parallel --skip_tests \
  --compile_no_warning_as_error \
  --build_shared_lib \
  --use_cuda \
  --cuda_home /usr/local/cuda-12.9 \
  --cudnn_home /path/to/cudnn-for-cuda12 \
  --cmake_extra_defines CMAKE_CUDA_ARCHITECTURES=52 \
      onnxruntime_BUILD_UNIT_TESTS=OFF \
      FETCHCONTENT_TRY_FIND_PACKAGE_MODE=NEVER \
      CMAKE_C_COMPILER_LAUNCHER=ccache \
      CMAKE_CXX_COMPILER_LAUNCHER=ccache \
      CMAKE_CUDA_COMPILER_LAUNCHER=ccache
# FETCHCONTENT…=NEVER: don't let system absl/protobuf leak in.
# ccache launchers: harmless if cold; the next API-version bump rebuild
# becomes mostly cache hits. Drop the three lines if ccache isn't installed.
# Libraries land in build/Linux/Release/
```

(`--compile_no_warning_as_error` because a host compiler newer than ORT's CI
turns fresh warnings into errors; `FETCHCONTENT_TRY_FIND_PACKAGE_MODE=NEVER`
because a distro's absl/protobuf cmake configs collide with ORT's vendored
copies. Both bit us on first build.)

Verify the API contract before pointing vecdb at it:

```bash
python3 -c "
import ctypes
lib = ctypes.CDLL('build/Linux/Release/libonnxruntime.so')
class B(ctypes.Structure):
    _fields_=[('GetApi',ctypes.CFUNCTYPE(ctypes.c_void_p,ctypes.c_uint32)),
              ('GetVersionString',ctypes.CFUNCTYPE(ctypes.c_char_p))]
lib.OrtGetApiBase.restype=ctypes.POINTER(B)
print('GetApi(24) ->', hex(lib.OrtGetApiBase().contents.GetApi(24) or 0))"
# non-zero pointer = compatible; 0x0 = wrong tag, vecdb would hang
```

Keep `libonnxruntime.so*` and `libonnxruntime_providers_{shared,cuda}.so`
**together in one directory** — the providers are found relative to the main
library.

> **Do you actually need to be here?** If your card *is* inside the
> `sm_75`/`sm_80`/`sm_90` window and your only problem is that the binary
> wants a CUDA major you do not have installed, you do **not** need this
> document — rebuild the default build with the other `ORT_CUDA_VERSION`
> (see [`GPU.md`](GPU.md)). That is a one-variable fix, not an ORT compile.
>
> Compiling is required only when no prebuilt carries kernels for your SM.
> Microsoft's official `onnxruntime-linux-x64-gpu` tarballs are
> self-consistent and work with `cuda-dynamic` the same way, but they are
> built for the same mainstream SM range — on SM < 6.0 expect
> `cudaErrorNoKernelImageForDevice` from them too. Setting
> `CMAKE_CUDA_ARCHITECTURES` yourself is what fixes that, and it is the whole
> reason for the source build below.

### 3. Build vecdb with `cuda-dynamic`

```bash
make check                                             # sanity
cargo install --path vecdb-cli    --locked --force --features cuda-dynamic
cargo install --path vecdb-server --locked --force --features cuda-dynamic
```

### 4. Tell vecdb where the runtime lives — once, in config

```toml
# ~/.config/vecdb/config.toml (top level, next to fastembed_cache_path)
ort_dylib_path = "/path/to/onnxruntime/build/Linux/Release/libonnxruntime.so"
```

That's the whole integration: vecdb applies it to `ORT_DYLIB_PATH` at startup
itself, so shells, hooks, and MCP server blocks need nothing. An explicitly
exported `ORT_DYLIB_PATH` in the environment overrides the config value —
useful for pointing one run at an experimental build:

```bash
ORT_DYLIB_PATH=/tmp/ort-experiment/libonnxruntime.so \
  vecdb --profile <gpu-profile> ingest -c <collection> <path>
```

If neither is set, a `cuda-dynamic` build refuses to embed with an error
naming both options (it does not fall back and does not hang).

### 5. Verify — trust the exit code, not vibes

vecdb fails **loudly** when the GPU path doesn't work; there is no silent
CPU fallback (that silence was the worst part of BUG-2026-254). A cheap
read-only probe — a `search` embeds the query with the profile's embedder:

```bash
vecdb --profile <gpu-profile> search "probe" -c <existing-collection>
```

**Run the probe; do not stop at the banner.** `search` is the right probe
precisely because it embeds — the banner alone is not proof.

- ✅ `[CUDA] Execution provider registered on the session` — means exactly
  that, and no more. With `error_on_failure` on the dispatch it cannot print
  unless the CUDA EP registered. But registration only needs cuBLAS to come
  up for your device; whether the runtime holds kernels your card can run is
  not settled until the first inference. A card outside the prebuilt's SM
  window can reach this line and still fail the very next step.
- ✅ **The search returns results** — this is the real confirmation.
- ❌ `[CUDA FAILURE]` + exit 1 — read the error: missing CUDA libs, wrong SM,
  version mismatch, or VRAM pressure. Fix or fall back to `use_gpu = false`.
- ❌ `cudaErrorNoKernelImageForDevice` **after** the registration banner — the
  EP loaded but carries no kernels for your compute capability. That is this
  document's whole scenario: build an ORT with your `CMAKE_CUDA_ARCHITECTURES`.

## Support policy

vecdb's GPU story is two tiers, on purpose:

- **Mainline** — the default build (and every published artifact, including
  aarch64) uses whatever the prebuilt ORT chain ships, currently kernels for
  `sm_75`/`sm_80`/`sm_90`. That window is pyke's choice, not ours; it can
  move in either direction when they rebuild, which is why
  `tests/tier2_ort_distribution.py` reads it out of the artifact rather than
  trusting this sentence.
- **Bring-your-own** — this document. Any embedder that works on a CUDA your
  toolkit can target keeps working through `cuda-dynamic` + your own ORT
  build, on any SM you compile for. For Maxwell that means every fastembed
  model vecdb supported through early 2026 remains GPU-capable on a GTX 9xx,
  indefinitely. An embedder that genuinely requires a newer CUDA than your
  card's ceiling is a mainline-only feature, not a regression of this tier.

Honesty is enforced, not promised. This tier is validated on real Maxwell
hardware (SM 5.2, r580 driver, CUDA 12.x and 13.x side-by-side) — including
that failures stay loud — and two contract tests fail the release gate on
drift: `tests/tier2_gpu_docs.py` (T2.5b) for the ort/ONNX versions this
document pins, and `tests/tier2_ort_distribution.py` for the SM range and
CUDA major that `GPU.md` advertises.

## Is it worth it?

Measure before committing. Small embedding models are highly competitive on
CPU — in our fleet benchmarks (BENCH-2026-240) local CPU ONNX beat a remote
GPU host for the small models. A Maxwell-era card gives real speedups mainly
on the larger local models (`nomic-embed-text-v1.5`) and bulk ingests.
