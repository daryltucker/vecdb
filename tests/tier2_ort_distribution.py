#!/usr/bin/env python3
"""
Tier 2: ONNX Runtime distribution contract (docs/GPU.md)

WHY THIS EXISTS
---------------
Which prebuilt ONNX Runtime gets baked into a vecdb binary was, until now,
UNOBSERVABLE and NON-DETERMINISTIC.

`ort-sys` picks between a `cu12` and a `cu13` artifact by sniffing the BUILD
MACHINE — `ORT_CUDA_VERSION`, else `CUDA_HOME`, else `NV_CUDA_CUDART_VERSION`,
else `nvcc --version`, else defaults to 12 (ort-sys build/download/resolve.rs).
So two people building the same commit could ship binaries requiring different
`libcudart` sonames, with nothing recording which. We shipped cu13 for months
purely because CUDA 13's nvcc was first on one PATH.

The two flavours carry the IDENTICAL kernel set, so this is not a GPU-support
knob — it only decides which CUDA the user must have installed. That makes it
exactly the kind of thing that drifts silently and is discovered by a user.

This test reads the truth out of the shipped artifact with objdump/cuobjdump
and holds docs/GPU.md to it. It asserts the ARTIFACT, never the configuration:
checking that the Makefile says `ORT_CUDA_VERSION ?= 12` would prove a variable
was set, not that the linked library honoured it.

Contract block in docs/GPU.md:

    <!-- ort-dist-contract: ...
    sm_targets = 75,80,90
    pinned_cuda_major = 12
    onnxruntime_version = 1.24.2
    env = ORT_CUDA_VERSION
    -->

NOT APPLICABLE to `cuda-dynamic` builds: those dlopen a runtime the user
supplies, so there is no bundled provider library to inspect — by design.
"""

import os
import re
import shutil
import subprocess
import sys
from pathlib import Path

REPO = Path(__file__).resolve().parent.parent
DOC = REPO / "docs" / "GPU.md"
PROVIDER_LIB = "libonnxruntime_providers_cuda.so"

FAILED = []


def log(msg, status="INFO"):
    use_colors = sys.stderr.isatty()
    if use_colors:
        colors = {"PASS": "\033[32m", "FAIL": "\033[31m", "WARN": "\033[33m", "INFO": "\033[34m"}
        print(f"{colors.get(status, '')}{msg}\033[0m", file=sys.stderr)
    else:
        prefix = f"{status}: " if status != "INFO" else ""
        print(f"{prefix}{msg}", file=sys.stderr)


def check(name, ok, detail=""):
    if ok:
        log(f"  ✓ {name}", "PASS")
    else:
        log(f"  ✗ {name} — {detail}", "FAIL")
        FAILED.append(name)


def parse_contract(text):
    m = re.search(r"<!--\s*ort-dist-contract:.*?-->", text, re.DOTALL)
    if not m:
        return None
    contract = {}
    for line in m.group(0).splitlines():
        kv = re.match(r"\s*([a-z_]+)\s*=\s*(\S+)\s*$", line)
        if kv:
            contract[kv.group(1)] = kv.group(2)
    return contract


def find_provider_lib():
    """Locate the CUDA provider library ort copied next to the build output.

    Searched in build-output order, then the install root. Returns None when
    there is none — a cuda-dynamic or --no-default-features build.
    """
    # Use the shared resolver, never a hand-rolled REPO/"target": this
    # workspace sets `build.target-dir` in .cargo/config.toml, so the literal
    # repo-local target/ is a STALE leftover that a fresh build never touches.
    # Inspecting it would have this test report on an artifact nobody ships.
    sys.path.insert(0, str(REPO / "tests"))
    from paths import target_dir  # noqa: E402

    candidates = [target_dir() / profile / PROVIDER_LIB for profile in ("release", "debug")]
    candidates.append(Path.home() / ".cargo" / "bin" / PROVIDER_LIB)
    for c in candidates:
        if c.exists():
            return c
    return None


def needed_cuda_major(lib):
    """The CUDA major this library will demand from the loader at runtime.

    Read from DT_NEEDED sonames (libcudart.so.N), which is what actually fails
    on a user's machine when it is absent.
    """
    out = subprocess.run(
        ["objdump", "-p", str(lib)], capture_output=True, text=True, timeout=120
    )
    if out.returncode != 0:
        return None, f"objdump failed: {out.stderr.strip()[:200]}"
    majors = set(re.findall(r"NEEDED\s+libcudart\.so\.(\d+)", out.stdout))
    if not majors:
        return None, "no libcudart NEEDED entry found"
    if len(majors) > 1:
        return None, f"multiple libcudart majors linked: {sorted(majors)}"
    return majors.pop(), None


def sm_targets(lib):
    """The compute capabilities with compiled kernels present in the fatbinary.

    This is the real support window — cubins are binary-compatible only within
    a compute-capability major version, and these artifacts embed no PTX, so
    there is no JIT fallback to widen it.
    """
    out = subprocess.run(
        ["cuobjdump", "-lelf", str(lib)], capture_output=True, text=True, timeout=600
    )
    if out.returncode != 0:
        return None, f"cuobjdump failed: {out.stderr.strip()[:200]}"
    found = sorted({int(n) for n in re.findall(r"sm_(\d+)", out.stdout)})
    if not found:
        return None, "no sm_NN cubins found in the library"
    return found, None


def main():
    log("━━━ ONNX Runtime distribution contract (docs/GPU.md) ━━━")

    check("docs/GPU.md exists", DOC.is_file())
    if FAILED:
        return 1
    contract = parse_contract(DOC.read_text())
    check("ort-dist-contract block present and parseable", bool(contract))
    if not contract:
        return 1
    for key in ("sm_targets", "pinned_cuda_major", "env"):
        check(f"contract has `{key}`", key in contract)
    if FAILED:
        return 1

    lib = find_provider_lib()
    if lib is None:
        # Genuinely not applicable — say so plainly and do not imply the
        # contract was verified. A cuda-dynamic build has no bundled provider
        # library to check; that is the whole point of that feature.
        log(
            f"  (no {PROVIDER_LIB} in target/ or ~/.cargo/bin — cuda-dynamic or "
            f"CPU-only build; distribution contract does not apply)",
            "WARN",
        )
        log("SKIPPED: nothing to verify for this build configuration", "WARN")
        return 0
    log(f"  inspecting {lib}")

    # ── 1. Which CUDA runtime will this binary demand? ──────────────────
    if shutil.which("objdump") is None:
        check("objdump available", False, "binutils is required to verify the artifact")
        return 1
    major, err = needed_cuda_major(lib)
    check("provider library links exactly one libcudart major", major is not None, err or "")
    if major is not None:
        check(
            f"linked CUDA major ({major}) == contract pinned_cuda_major "
            f"({contract['pinned_cuda_major']})",
            major == contract["pinned_cuda_major"],
            f"built artifact needs libcudart.so.{major}; docs/GPU.md promises "
            f"{contract['pinned_cuda_major']}. Either {contract['env']} was not "
            f"honoured by this build, or the pin moved and the doc did not.",
        )

    # ── 2. Which GPUs does it actually carry kernels for? ───────────────
    # Requires the CUDA toolkit. The release gate runs on a machine that has
    # it, so a missing cuobjdump there is a real failure, not a skip — the
    # sole permitted opt-out is explicit and lives in .github/workflows/ci.yml,
    # where hosted runners have no CUDA toolkit.
    expected_sms = [int(n) for n in contract["sm_targets"].split(",")]
    if shutil.which("cuobjdump") is None:
        if os.environ.get("VECDB_ALLOW_UNVERIFIED_SM") == "1":
            log(
                "  ! SM coverage UNVERIFIED this run — cuobjdump not found and "
                "VECDB_ALLOW_UNVERIFIED_SM=1. docs/GPU.md's support matrix was "
                "NOT checked against the artifact.",
                "WARN",
            )
        else:
            check(
                "cuobjdump available",
                False,
                "needed to verify docs/GPU.md's GPU support matrix against the "
                "artifact. Install the CUDA toolkit, or set "
                "VECDB_ALLOW_UNVERIFIED_SM=1 to acknowledge the gap explicitly.",
            )
    else:
        found, err = sm_targets(lib)
        check("cuobjdump lists compiled kernels", found is not None, err or "")
        if found is not None:
            check(
                f"SM coverage {found} == contract sm_targets {expected_sms}",
                found == expected_sms,
                "the prebuilt's GPU support window moved. docs/GPU.md's matrix, "
                "docs/GPU_LEGACY.md's 'who this is for', and the README blockquote "
                "all quote this set and are now wrong.",
            )

    # ── 3. `vecdb --version` must report a MEASURED ONNX version. ───────
    # get_ort_version() once returned the literal "1.23.2" while the prebuilt
    # had moved to 1.24.2, so the CLI published a number nobody had checked and
    # docs/GPU.md copied it. It now asks the linked runtime via OrtGetApiBase.
    # Compare that answer to the artifact the contract describes.
    check_version_against = contract.get("onnxruntime_version")
    if check_version_against:
        sys.path.insert(0, str(REPO / "tests"))
        try:
            from paths import find_bin  # noqa: E402

            out = subprocess.run(
                [find_bin("vecdb"), "--version"], capture_output=True, text=True, timeout=120
            )
            reported = re.search(r"ONNX Runtime: (\S+)", out.stdout)
            if reported and reported.group(1).startswith("dynamic"):
                # cuda-dynamic build: the runtime is whatever ORT_DYLIB_PATH
                # names, so the prebuilt's version is not what this binary uses.
                # Correct to report the contract instead of a concrete number.
                log("  (cuda-dynamic build — ONNX version is the user's, not the prebuilt's)")
            elif reported:
                check(
                    f"`vecdb --version` ONNX Runtime: {reported.group(1)} == contract "
                    f"onnxruntime_version ({check_version_against})",
                    reported.group(1) == check_version_against,
                    "the reported ONNX version disagrees with the artifact this "
                    "doc describes — a hardcoded literal has drifted again",
                )
            else:
                check(
                    "`vecdb --version` reports an ONNX version",
                    False,
                    f"no 'ONNX Runtime: …' line in: {out.stdout.strip()[:200]}",
                )
        except SystemExit as e:
            log(f"  (vecdb binary unavailable — {e}; ONNX version not checked)", "WARN")

    # ── 4. The doc must teach the knob it pins. ─────────────────────────
    prose = re.sub(r"<!--.*?-->", "", DOC.read_text(), flags=re.DOTALL)
    check(
        f"prose documents `{contract['env']}`",
        contract["env"] in prose,
        "contract pins a variable the document never explains",
    )
    for sm in expected_sms:
        check(
            f"prose states sm_{sm}",
            f"sm_{sm}" in prose,
            "the support matrix must name every SM the artifact carries",
        )

    if FAILED:
        log(f"FAILED: {len(FAILED)} check(s)", "FAIL")
        return 1
    log("All ONNX Runtime distribution contract checks passed", "PASS")
    return 0


if __name__ == "__main__":
    sys.exit(main())
