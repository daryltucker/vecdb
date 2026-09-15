#!/usr/bin/env python3
"""
Tier 2: `.vecdbrc` routing across embedding spaces, on CPU **and** GPU, and
`delete` targeting the collection's own store.

> [!CRITICAL]
> **TEST ISOLATION MANDATE** — test Qdrant only, ports 6335/6336, `test_`
> prefixed collections. NEVER production (6333/6334).

Uses the shared fixture `tests/fixtures/config.toml` (profiles `route_cpu`,
`route_gpu`, `route_alias`, `route_other`; collections `test_route_near_cpu`,
`test_route_near_gpu`, `test_route_far`, `test_route_same`). It does not write a config of its own —
loading the real fixture is part of what this exercises.

Three defects found 2026-256, all silent:

1. **Routing wrote the wrong embedding space.** `route_chunking` resolved chunk
   size per routed collection, but the embedder and store were resolved once for
   the run, because the pipeline holds a single `Core`. A route naming a
   collection whose profile uses a different model embedded with the run's model
   and wrote it under that name. Where the target did not exist it was CREATED
   at the wrong dimension — the space guard cannot catch that, there being no
   genesis to compare against. `ingest` now runs one pass per destination.

2. **Unrouted files were claimed by every pass.** `collection` was both the
   route fallback and the write target the space guard validates, so a
   per-destination pass re-ingested every unmatched file into its own space.
   Split into `route_default_collection` and `collection`.

3. **`delete` resolved the default profile's store.** It passed `None` for the
   collection, ignoring `[collections.<name>]`, so deleting a collection routed
   elsewhere hit the wrong endpoint — and Qdrant treats deleting an absent
   collection as success, so it printed "Done" while the data survived.

**CPU and GPU are both required.** The operator's route runs on GPU; a CPU-only
pass says nothing about it. Without a `--features cuda-dynamic` binary and a
reachable ONNX Runtime this FAILS rather than skipping — a green suite that
never touched the GPU path is how the 2026-254 GPU bugs survived. Set
`VECDB_ALLOW_NO_GPU=1` on a machine with no GPU to downgrade it to a reported skip.
"""
import json
import os
import shutil
import subprocess
import sys
import time
import urllib.request

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
from paths import bin_path

VECDB_BIN = bin_path("vecdb")
REPO = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
FIXTURE = os.path.join(REPO, "tests", "fixtures", "config.toml")
DATA = "tests/run/tier2_routing_delete/data"
CONTAINER_NAME = "qdrant-test"
HTTP = "http://localhost:6335"
GRPC = "http://localhost:6336"

NEAR_CPU = "test_route_near_cpu"
NEAR_GPU = "test_route_near_gpu"
FAR = "test_route_far"
SAME = "test_route_same"
# No default: a hardcoded path is one machine's disk layout, wrong everywhere
# else and private in a public repo. run_all.sh exports ORT_DYLIB_PATH when the
# runtime exists; a cuda-dynamic binary refuses to embed without it, loudly.
DEFAULT_ORT = os.environ.get("ORT_DYLIB_PATH", "")


def ensure_test_qdrant():
    try:
        res = subprocess.run(
            ["docker", "ps", "--filter", f"name={CONTAINER_NAME}", "--format", "{{.ID}}"],
            capture_output=True, text=True, check=True)
        if res.stdout.strip():
            return
        res = subprocess.run(
            ["docker", "ps", "-a", "--filter", f"name={CONTAINER_NAME}", "--format", "{{.ID}}"],
            capture_output=True, text=True, check=True)
        if res.stdout.strip():
            subprocess.run(["docker", "start", CONTAINER_NAME], check=True)
        else:
            subprocess.run(["docker", "run", "-d", "-p", "6335:6333", "-p", "6336:6334",
                            "--name", CONTAINER_NAME, "qdrant/qdrant"], check=True)
        time.sleep(5)
    except subprocess.CalledProcessError as e:
        print(f"CRITICAL FAIL: could not manage test container: {e}")
        sys.exit(1)


def qdrant(method, path):
    req = urllib.request.Request(HTTP + path, method=method,
                                 headers={"Content-Type": "application/json"})
    with urllib.request.urlopen(req, timeout=30) as r:
        return json.load(r)


def info(name):
    try:
        r = qdrant("GET", f"/collections/{name}")["result"]
        return r["config"]["params"]["vectors"]["size"], r["points_count"]
    except Exception:
        return None


def clear():
    for c in (NEAR_CPU, NEAR_GPU, FAR, SAME):
        try:
            qdrant("DELETE", f"/collections/{c}")
        except Exception:
            pass


def write_tree(near_collection, far_collection):
    """Two unrouted notes plus one routed finding."""
    if os.path.exists(DATA):
        shutil.rmtree(DATA)
    os.makedirs(f"{DATA}/findings")
    for n in ("note_a.md", "note_b.md"):
        with open(f"{DATA}/{n}", "w") as f:
            f.write(f"An ordinary working note ({n}) about chunk granularity and retrieval.\n")
    with open(f"{DATA}/findings/f1.md", "w") as f:
        f.write("A published finding about embeddings, retrieval quality and evaluation.\n")
    with open(f"{DATA}/.vecdbrc", "w") as f:
        f.write(f'[default]\ncollection = "{near_collection}"\n\n'
                f'[[routes]]\nglob = "findings/**"\ncollection = "{far_collection}"\n')


def ingest():
    """No `--profile`: that is the operator's real invocation, and it is the only
    one under which each routed collection resolves through its own entry."""
    return subprocess.run([VECDB_BIN, "ingest", DATA], capture_output=True, text=True)


def gpu_ready():
    ort = os.environ.get("ORT_DYLIB_PATH", DEFAULT_ORT)
    if not os.path.exists(ort):
        return None, f"ONNX Runtime not found at {ort}"
    probe = subprocess.run([VECDB_BIN, "config", "show", "-c", NEAR_GPU],
                           capture_output=True, text=True)
    if probe.returncode != 0:
        return None, f"route_gpu profile does not resolve: {probe.stderr.strip()[:120]}"
    return ort, None


def check_split(label, near, failures):
    """One command, two embedding spaces, every file handled exactly once."""
    write_tree(near, FAR)
    clear()
    r = ingest()
    out = r.stdout + r.stderr
    if r.returncode != 0:
        failures.append(f"[{label}] ingest failed: {out.strip()[-300:]}")
        return
    near_i, far = info(near), info(FAR)
    if not near_i or not far:
        failures.append(f"[{label}] expected both collections; {near}={near_i} FAR={far}")
        return
    if near_i[0] == far[0]:
        failures.append(f"[{label}] both are {near_i[0]}-dim — the second pass "
                        "did not use its own embedder")
    if near_i[1] != 3:
        failures.append(f"[{label}] {near} has {near_i[1]} points, expected 3 "
                        "(genesis + 2 unrouted notes) — passes may be double-claiming files")
    if far[1] != 2:
        failures.append(f"[{label}] {FAR} has {far[1]} points, expected 2 "
                        "(genesis + 1 finding) — a pass took files it does not own")
    print(f"  [{label}] {near} {near_i[0]}d/{near_i[1]}pts · {FAR} {far[0]}d/{far[1]}pts")


def main():
    ensure_test_qdrant()
    os.makedirs(os.path.dirname(DATA), exist_ok=True)
    os.environ["VECDB_CONFIG"] = FIXTURE
    # A cuda-dynamic binary refuses to embed at ALL without this, GPU or not.
    if os.path.exists(os.environ.get("ORT_DYLIB_PATH", DEFAULT_ORT)):
        os.environ.setdefault("ORT_DYLIB_PATH", DEFAULT_ORT)
    failures = []

    # 1. cross-space routing on CPU
    check_split("cpu", NEAR_CPU, failures)
    if not failures:
        print("✓ CPU: split run put each file in its own embedding space")

    # 2. the same thing on GPU — the operator's real path
    ort, why = gpu_ready()
    if ort:
        os.environ["ORT_DYLIB_PATH"] = ort
        before = len(failures)
        check_split("gpu", NEAR_GPU, failures)
        if len(failures) == before:
            print("✓ GPU: split run put each file in its own embedding space")
    elif os.environ.get("VECDB_ALLOW_NO_GPU") == "1":
        print(f"⚠ GPU case SKIPPED (VECDB_ALLOW_NO_GPU=1): {why}")
    else:
        failures.append(
            f"GPU case could not run — {why}. The operator's route is GPU; a CPU-only "
            "pass does not cover it. Build with `--features cuda-dynamic` and set "
            "ORT_DYLIB_PATH, or VECDB_ALLOW_NO_GPU=1 where there is no GPU.")

    # 3. same model under a different embedder name must NOT split
    write_tree(NEAR_CPU, SAME)
    clear()
    r = ingest()
    out = r.stdout + r.stderr
    if "Routing spans" in out:
        failures.append("same-model route triggered a split — the check is comparing "
                        "embedder names instead of the embedding space")
    elif r.returncode != 0:
        failures.append(f"same-space ingest failed: {out.strip()[-200:]}")
    else:
        near, same = info(NEAR_CPU), info(SAME)
        if not near or not same or near[0] != same[0]:
            failures.append(f"same-space run did not populate both at one width: {near} {same}")
        else:
            print(f"✓ same-model route stayed one pass ({near[0]}d)")

    # 4. delete follows the collection's store and reports honestly
    absent = subprocess.run([VECDB_BIN, "delete", "--yes", "test_definitely_absent_xyz"],
                            capture_output=True, text=True)
    if "not found" not in (absent.stdout + absent.stderr):
        failures.append("deleting an absent collection did not say 'not found' — "
                        "Qdrant returns ok for a no-op, so this is a phantom success")
    else:
        print("✓ absent collection reports 'not found', not 'Done'")

    if info(NEAR_CPU):
        d = subprocess.run([VECDB_BIN, "delete", "--yes", NEAR_CPU], capture_output=True, text=True)
        combined = d.stdout + d.stderr
        if GRPC not in combined:
            failures.append(f"delete did not name the store it targeted: {combined.strip()[:160]}")
        elif info(NEAR_CPU):
            failures.append(f"delete said success but {NEAR_CPU} still exists")
        else:
            print("✓ delete named its store and actually removed the collection")

    clear()
    shutil.rmtree(os.path.dirname(DATA), ignore_errors=True)

    if failures:
        print("\nFAILED:")
        for f in failures:
            print(f"  - {f}")
        sys.exit(1)
    print("\nPASS: routing splits per embedding space on CPU and GPU; "
          "delete is store-scoped and honest")


if __name__ == "__main__":
    main()
