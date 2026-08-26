#!/usr/bin/env python3
"""
Tier 2 Integration Test: Smart Routing Facets (Configurable & Regex)

> [!CRITICAL]
> **TEST ISOLATION MANDATE**
> All tests MUST use the dedicated **TEST QDRANT INSTANCE** (`qdrant-test`) running on ports **6335 (HTTP)** and **6336 (gRPC)**.
> NEVER, EVER connect tests to the Production instance (ports 6333/6334).
>
> **PROTOCOL**:
> 1. Tests MUST load `tests/fixtures/config.toml` (or equivalent test-scoped config) via `VECDB_CONFIG`.
> 2. Scripts MUST verify `qdrant-test` is running before execution.
> 3. If a test touches port 6333, IT IS A CRITICAL FAILURE.
"""
import os
import sys
import shutil
import subprocess
import json
import time

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
from lib_envelope import search_results

import sys, os as _os
sys.path.insert(0, _os.path.dirname(_os.path.abspath(__file__)))
from paths import bin_path

# Setup
VECDB_BIN = bin_path("vecdb")
TEST_DIR = "tests/run/tier2_facets"
CONFIG_PATH = os.path.join(TEST_DIR, "config.toml")
CONTAINER_NAME = "qdrant-test"

def ensure_test_qdrant():
    """Ensure the isolated test Qdrant container is running."""
    try:
        # Check if running
        res = subprocess.run(
            ["docker", "ps", "--filter", f"name={CONTAINER_NAME}", "--format", "{{.ID}}"],
            capture_output=True, text=True, check=True
        )
        if res.stdout.strip():
            print("✓ Test Qdrant is running.")
            return

        # Check if exists but stopped
        res = subprocess.run(
            ["docker", "ps", "-a", "--filter", f"name={CONTAINER_NAME}", "--format", "{{.ID}}"],
            capture_output=True, text=True, check=True
        )
        if res.stdout.strip():
            print("↺ Starting existing Test Qdrant container...")
            subprocess.run(["docker", "start", CONTAINER_NAME], check=True)
        else:
            print("✚ Creating Test Qdrant container (Port 6336 gRPC)...")
            subprocess.run([
                "docker", "run", "-d",
                "-p", "6335:6333",  # HTTP
                "-p", "6336:6334",  # gRPC
                "--name", CONTAINER_NAME,
                "qdrant/qdrant"
            ], check=True)
        
        print("Waiting for Qdrant to be healthy...")
        time.sleep(5) # Give it a moment to spin up
        
    except subprocess.CalledProcessError as e:
        print(f"CRITICAL FAIL: Could not manage test container: {e}")
        sys.exit(1)

def setup():
    ensure_test_qdrant()

    if os.path.exists(TEST_DIR):
        shutil.rmtree(TEST_DIR)
    os.makedirs(TEST_DIR)
    os.makedirs(f"{TEST_DIR}/data")
    
    os.environ["VECDB_CONFIG"] = os.path.abspath(CONFIG_PATH)
    
    # Init DB (MUST run before writing custom config due to safety check)
    print("Initializing...")
    subprocess.run([VECDB_BIN, "init"], check=True, capture_output=True)
    
    # Create Config with custom smart_routing_keys manually
    # NOTE: qdrant_url points to 6336 (gRPC) for rust client
    # NOTE: smart_routing_keys MUST be at top level
    config_content = """
smart_routing_keys = ["platform", "version", "language"]

[backend.local]
kind = "fastembed"

[embedder.default]
backend = "local"
model = "all-minilm-l6-v2"

[profiles.default]
embedder = "default"
qdrant_url = "http://localhost:6336"
default_collection_name = "test_facets"
    """
    
    with open(CONFIG_PATH, "w") as f:
        f.write(config_content)

def ingest_data():
    # 1. Windows content
    with open(f"{TEST_DIR}/data/win.txt", "w") as f:
        f.write("PowerShell scripts are great for admin tasks and automation.")
        
    # 2. Linux content
    with open(f"{TEST_DIR}/data/linux.txt", "w") as f:
        f.write("Bash scripts are better for servers and cloud infrastructure.")

    print("Ingesting Windows data...")
    cmd = [VECDB_BIN, "ingest", f"{TEST_DIR}/data/win.txt", "-m", "platform=windows", "-m", "language=powershell"]
    subprocess.run(cmd, check=True)

    print("Ingesting Linux data...")
    cmd = [VECDB_BIN, "ingest", f"{TEST_DIR}/data/linux.txt", "-m", "platform=linux", "-m", "language=bash"]
    subprocess.run(cmd, check=True)

def run_search(query, smart_routing=False):
    cmd = [VECDB_BIN, "search", query, "--json"]
    if smart_routing:
        cmd.append("--smart")
        
    result = subprocess.run(cmd, capture_output=True, text=True)
    if result.returncode != 0:
        print(f"Search failed: {result.stderr}")
        sys.exit(1)

    if result.stderr:
        print(f"DEBUG STDERR: {result.stderr}", file=sys.stderr)
        
    payload = json.loads(result.stdout)
    return search_results(payload, context=f"query={query!r}"), payload["applied_filters"]

def main():
    setup()
    
    # Clean previous run
    print("Cleaning collection...")
    subprocess.run([VECDB_BIN, "delete", "test_facets", "--force"], check=True, capture_output=True)
    
    ingest_data()
    
    # Allow indexing
    time.sleep(1)
    
    print("\n--- Test 1: Generic Search (No Smart) ---")
    results, filters = run_search("script")
    print(f"Generic results: {len(results)}")
    if filters:
        print(f"FAIL: no qualifier was given, so no filter should be applied; got {filters}")
        sys.exit(1)
    if len(results) < 2:
        print("FAIL: Expected 2 results for generic search")
        sys.exit(1)
        
    print("\n--- Test 2: Explicit qualifier (platform:windows) ---")
    # Faceted search is driven by an explicit `key:value` qualifier. The bare
    # word "windows" appearing in a query is prose, not an instruction — see
    # BUG_SMART_ROUTING_NAKED_FACET_MATCH-2026-234.
    results, filters = run_search("platform:windows automation", smart_routing=True)
    print(f"Qualified results: {len(results)} filters={filters}")

    if filters.get("platform") != "windows":
        print(f"FAIL: expected platform=windows to be applied and reported, got {filters}")
        sys.exit(1)

    if len(results) != 1:
        print(f"FAIL: Expected 1 result, got {len(results)}")
        for r in results:
            print(f"- {r['content'][:50]}...")
        sys.exit(1)

    if "PowerShell" not in results[0]['content']:
        print("FAIL: Result content mismatch")
        sys.exit(1)
    print("PASS: Correctly filtered to Windows content")

    print("\n--- Test 3: A bare facet value must not filter ---")
    # The regression guard. Previously any bare word matching a facet value
    # silently narrowed the search, so prose like "windows automation" returned
    # a subset of the corpus with nothing saying why. Now only `key:value` does.
    results, filters = run_search("windows automation", smart_routing=True)
    print(f"Unqualified results: {len(results)} filters={filters}")

    if filters:
        print(f"FAIL: bare prose must not apply a filter; got {filters}")
        sys.exit(1)

    if len(results) < 2:
        print(f"FAIL: unfiltered search should still see the whole corpus, got {len(results)}")
        sys.exit(1)
    print("PASS: Bare facet value left the query unfiltered")

    print("\n--- Test 4: Unknown qualifier value is an error, not silence ---")
    cmd = [VECDB_BIN, "search", "platform:solaris automation", "--json", "--smart"]
    result = subprocess.run(cmd, capture_output=True, text=True)
    if result.returncode == 0:
        print("FAIL: an unknown facet value should fail loudly, not return an empty list")
        sys.exit(1)
    if "solaris" not in (result.stderr + result.stdout):
        print(f"FAIL: the error should name the offending value; got: {result.stderr[:300]}")
        sys.exit(1)
    print("PASS: Unknown facet value rejected and named")

    print("\nALL TESTS PASSED")

if __name__ == "__main__":
    main()
