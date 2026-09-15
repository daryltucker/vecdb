#!/usr/bin/env python3
"""
Tier 2 Integration Test: `vecdb list` shows vecdb's collections and nothing else.

> [!CRITICAL]
> **TEST ISOLATION MANDATE**
> All tests MUST use the dedicated **TEST QDRANT INSTANCE** (`qdrant-test`) running
> on ports **6335 (HTTP)** and **6336 (gRPC)**.
> NEVER, EVER connect tests to the Production instance (ports 6333/6334).

Regression for 2026-255. `list` used to print foreign collections with a
"— not a vecdb collection" label plus a trailing "shown for visibility" note,
so another tool's data appeared in vecdb's own inventory. On a shared Qdrant
holding unrelated corpora, that is most of the output.

The taken-name concern that motivated showing them is already handled at the
only moment it matters: `ensure_write_target` refuses to create or write a
collection whose genesis is not vecdb's. This test pins both halves — the
foreign one is absent from `list`, and writing to it still fails loudly.
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
TEST_DIR = "tests/run/tier2_list_only_vecdb"
CONFIG_PATH = os.path.join(TEST_DIR, "config.toml")
CONTAINER_NAME = "qdrant-test"
HTTP = "http://localhost:6335"

OWNED = "test_list_owned"
FOREIGN = "test_list_foreign_not_vecdb"


def ensure_test_qdrant():
    try:
        res = subprocess.run(
            ["docker", "ps", "--filter", f"name={CONTAINER_NAME}", "--format", "{{.ID}}"],
            capture_output=True, text=True, check=True,
        )
        if res.stdout.strip():
            print("✓ Test Qdrant is running.")
            return
        res = subprocess.run(
            ["docker", "ps", "-a", "--filter", f"name={CONTAINER_NAME}", "--format", "{{.ID}}"],
            capture_output=True, text=True, check=True,
        )
        if res.stdout.strip():
            print("↺ Starting existing Test Qdrant container...")
            subprocess.run(["docker", "start", CONTAINER_NAME], check=True)
        else:
            print("✚ Creating Test Qdrant container...")
            subprocess.run([
                "docker", "run", "-d",
                "-p", "6335:6333", "-p", "6336:6334",
                "--name", CONTAINER_NAME, "qdrant/qdrant",
            ], check=True)
        print("Waiting for Qdrant to be healthy...")
        time.sleep(5)
    except subprocess.CalledProcessError as e:
        print(f"CRITICAL FAIL: Could not manage test container: {e}")
        sys.exit(1)


def qdrant(method, path, body=None):
    req = urllib.request.Request(
        HTTP + path,
        data=json.dumps(body).encode() if body is not None else None,
        headers={"Content-Type": "application/json"},
        method=method,
    )
    with urllib.request.urlopen(req, timeout=30) as r:
        return json.load(r)


def setup():
    ensure_test_qdrant()

    if os.path.exists(TEST_DIR):
        shutil.rmtree(TEST_DIR)
    os.makedirs(f"{TEST_DIR}/data")

    os.environ["VECDB_CONFIG"] = os.path.abspath(CONFIG_PATH)
    subprocess.run([VECDB_BIN, "init"], check=True, capture_output=True)

    with open(CONFIG_PATH, "w") as f:
        f.write("""
[backend.local]
kind = "fastembed"

[embedder.default]
backend = "local"
model = "all-minilm-l6-v2"

[profiles.default]
embedder = "default"
qdrant_url = "http://localhost:6336"
default_collection_name = "%s"
""" % OWNED)

    # A collection vecdb did not create: no genesis point at the nil UUID.
    # This is what another tool's corpus looks like from vecdb's side.
    for name in (OWNED, FOREIGN):
        try:
            qdrant("DELETE", f"/collections/{name}")
        except Exception:
            pass
    qdrant("PUT", f"/collections/{FOREIGN}",
           {"vectors": {"size": 384, "distance": "Cosine"}})
    print(f"✓ Created foreign collection {FOREIGN} (no genesis point)")

    with open(f"{TEST_DIR}/data/note.txt", "w") as f:
        f.write("A short document so the owned collection exists with real vectors.")
    # -c explicitly: this repo ships its own .vecdbrc routing to `code`, and
    # discovery walks UP from the ingest path, so a bare ingest here lands in
    # the wrong collection.
    subprocess.run([VECDB_BIN, "ingest", f"{TEST_DIR}/data/note.txt", "-c", OWNED],
                   check=True, capture_output=True)
    print(f"✓ Ingested into {OWNED}")


def teardown():
    for name in (OWNED, FOREIGN):
        try:
            qdrant("DELETE", f"/collections/{name}")
        except Exception:
            pass
    if os.path.exists(TEST_DIR):
        shutil.rmtree(TEST_DIR)


def main():
    setup()
    failures = []

    # Both collections genuinely exist on the instance; only one is vecdb's.
    live = {c["name"] for c in qdrant("GET", "/collections")["result"]["collections"]}
    if OWNED not in live or FOREIGN not in live:
        print(f"FAIL: fixture wrong — Qdrant holds {sorted(live)}")
        teardown()
        sys.exit(1)

    res = subprocess.run([VECDB_BIN, "list", "--json"], capture_output=True, text=True)
    if res.returncode != 0:
        failures.append(f"`list --json` exited {res.returncode}: {res.stderr.strip()[:200]}")
    payload = json.loads(res.stdout[res.stdout.index("{"):])
    listed = [c["name"] for cols in payload.values() if isinstance(cols, list) for c in cols
              if isinstance(c, dict) and "name" in c]

    if FOREIGN in listed:
        failures.append(f"--json listed the foreign collection {FOREIGN}: {listed}")
    else:
        print(f"✓ --json omits {FOREIGN}")

    if OWNED not in listed:
        failures.append(f"--json dropped vecdb's own collection {OWNED}: {listed}")
    else:
        print(f"✓ --json includes {OWNED}")

    human = subprocess.run([VECDB_BIN, "list"], capture_output=True, text=True).stdout
    if FOREIGN in human:
        failures.append("human output still names the foreign collection")
    elif "not a vecdb collection" in human or "shown for visibility" in human:
        failures.append("human output still carries the foreign-collection labelling")
    else:
        print("✓ human output omits it too")

    if OWNED not in human:
        failures.append(f"human output dropped {OWNED}")

    # Hiding it must not weaken the guard that makes hiding it safe.
    write = subprocess.run([VECDB_BIN, "ingest", f"{TEST_DIR}/data/note.txt", "-c", FOREIGN],
                           capture_output=True, text=True)
    combined = write.stdout + write.stderr
    if write.returncode == 0 or "not a vecdb collection" not in combined:
        failures.append(
            f"writing to the foreign collection was not refused (exit {write.returncode}): "
            f"{combined.strip()[:200]}"
        )
    else:
        print("✓ writing to it is still refused, loudly")

    teardown()

    if failures:
        print("\nFAILED:")
        for f in failures:
            print(f"  - {f}")
        sys.exit(1)
    print("\nPASS: vecdb list shows only vecdb collections")


if __name__ == "__main__":
    main()
