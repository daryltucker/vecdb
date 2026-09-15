#!/usr/bin/env python3
"""
Tier 2: GPU documentation contract (docs/GPU_LEGACY.md)

Purpose: keep the published legacy-GPU path honest. GPU_LEGACY.md tells users
to build a specific ONNX Runtime version and a specific cargo feature; if the
ort crate is bumped, the feature renamed, or the hardcoded ORT version in
vecdb_core::get_ort_version() changes without the doc following, users compile
onnxruntime for hours against the wrong contract.

The doc carries a machine-readable block:

    <!-- gpu-legacy-contract: ...
    feature = cuda-dynamic
    env = ORT_DYLIB_PATH
    ort_api_version = 24
    onnxruntime_version = 1.24.4
    ort_crate = 2.0.0-rc.12
    -->

Note this is the tag users are told to BUILD for a bring-your-own runtime, not
the version of the prebuilt vecdb links by default — those are different numbers
on purpose. The prebuilt's version is asserted by tier2_ort_distribution.py
(T2.5c), which reads it out of the artifact.

This test verifies each value against the repository:
1. `feature` exists in [features] of vecdb-core, vecdb-cli AND vecdb-server
2. `ort_crate` matches the ort version pinned in Cargo.lock
3. `ort_api_version` matches the highest `api-N` feature in the resolved
   dependency graph (cargo tree) — THE real compatibility contract. Version
   strings are not it: pyke's prebuilt says "1.23.2" but answers API 24,
   stock v1.23.2 does not, and ort rc.12 HANGS (error-path self-deadlock)
   rather than erroring on a mismatch. Deliberately NOT compared against
   get_ort_version()'s literal, which describes the static runtime's
   self-reported string.
4. `onnxruntime_version` (the tag the doc tells users to build) belongs to
   the 1.<api> release line
5. `env` and `feature` are actually used in the doc prose (not only in the
   contract comment), and docs/GPU.md cross-links GPU_LEGACY.md
"""

import re
import subprocess
import sys
from pathlib import Path

REPO = Path(__file__).resolve().parent.parent
DOC = REPO / "docs" / "GPU_LEGACY.md"
GPU_DOC = REPO / "docs" / "GPU.md"

FAILED = []


def log(msg, status="INFO"):
    use_colors = sys.stderr.isatty()
    if use_colors:
        colors = {"PASS": "\033[32m", "FAIL": "\033[31m", "INFO": "\033[34m"}
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
    m = re.search(r"<!--\s*gpu-legacy-contract:.*?-->", text, re.DOTALL)
    if not m:
        return None
    contract = {}
    for line in m.group(0).splitlines():
        kv = re.match(r"\s*([a-z_]+)\s*=\s*(\S+)\s*$", line)
        if kv:
            contract[kv.group(1)] = kv.group(2)
    return contract


def main():
    log("━━━ GPU documentation contract (docs/GPU_LEGACY.md) ━━━")

    check("docs/GPU_LEGACY.md exists", DOC.is_file())
    if FAILED:
        return 1
    text = DOC.read_text()

    contract = parse_contract(text)
    check("contract block present and parseable", bool(contract))
    if not contract:
        return 1
    for key in ("feature", "env", "ort_api_version", "onnxruntime_version", "ort_crate"):
        check(f"contract has `{key}`", key in contract)
    if FAILED:
        return 1

    feature = contract["feature"]

    # 1. Feature exists in all three crates' [features] tables.
    for crate in ("vecdb-core", "vecdb-cli", "vecdb-server"):
        manifest = (REPO / crate / "Cargo.toml").read_text()
        feat_section = manifest.split("[features]", 1)
        ok = len(feat_section) == 2 and re.search(
            rf"^{re.escape(feature)}\s*=", feat_section[1], re.MULTILINE
        )
        check(
            f"feature `{feature}` in {crate}/Cargo.toml [features]",
            bool(ok),
            f"doc names a feature {crate} does not define",
        )

    # 2. ort crate version in Cargo.lock matches the doc.
    lock = (REPO / "Cargo.lock").read_text()
    m = re.search(r'name = "ort"\nversion = "([^"]+)"', lock)
    check("ort in Cargo.lock", bool(m))
    if m:
        check(
            f"doc ort_crate ({contract['ort_crate']}) == Cargo.lock ({m.group(1)})",
            m.group(1) == contract["ort_crate"],
            "ort was bumped — update GPU_LEGACY.md (contract AND prose)",
        )

    # 3. The real contract: the API version the ort crate will request is the
    # highest api-N feature enabled anywhere in the resolved graph (fastembed
    # pins one). A BYO libonnxruntime.so must answer GetApi(<that N>); a
    # mismatch does not error under ort rc.12 — it deadlocks. So this number
    # in the doc MUST track the graph.
    tree = subprocess.run(
        ["cargo", "tree", "-p", "vecdb-core", "--features", "cuda-dynamic",
         "-i", "ort", "-e", "features"],
        capture_output=True, text=True, cwd=REPO, timeout=120,
    )
    check("cargo tree resolves ort feature graph", tree.returncode == 0,
          tree.stderr.strip()[:200])
    api_versions = [int(n) for n in re.findall(r'ort feature "api-(\d+)"', tree.stdout)]
    check("api-N features present in graph", bool(api_versions))
    if api_versions:
        graph_api = max(api_versions)
        check(
            f"doc ort_api_version ({contract['ort_api_version']}) == "
            f"highest api-N in graph ({graph_api})",
            str(graph_api) == contract["ort_api_version"],
            "the ort/fastembed api-N pin moved — users' BYO runtimes will "
            "HANG, not error; update GPU_LEGACY.md contract AND prose",
        )

    # 4. The tag the doc tells users to build must belong to the 1.<api>
    # release line (upstream introduces API N in release 1.N).
    check(
        f"doc onnxruntime_version ({contract['onnxruntime_version']}) is on "
        f"the 1.{contract['ort_api_version']} line",
        contract["onnxruntime_version"].startswith(f"1.{contract['ort_api_version']}."),
        "build-tag and API version disagree — one of them is stale",
    )

    # 4. The doc actually teaches what the contract claims, and GPU.md links here.
    prose = re.sub(r"<!--.*?-->", "", text, flags=re.DOTALL)
    check(
        f"prose mentions `{feature}`",
        feature in prose,
        "contract names a feature the instructions never use",
    )
    check(
        f"prose mentions `{contract['env']}`",
        contract["env"] in prose,
        "contract names an env var the instructions never use",
    )
    check(
        "prose pins the onnxruntime version",
        contract["onnxruntime_version"] in prose,
        "build instructions must state the exact version to check out",
    )
    check(
        "docs/GPU.md cross-links GPU_LEGACY.md",
        "GPU_LEGACY.md" in GPU_DOC.read_text(),
        "users on old cards land in GPU.md first; it must route them onward",
    )

    if FAILED:
        log(f"FAILED: {len(FAILED)} check(s)", "FAIL")
        return 1
    log("All GPU documentation contract checks passed", "PASS")
    return 0


if __name__ == "__main__":
    sys.exit(main())
