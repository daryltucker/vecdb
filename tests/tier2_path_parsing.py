#!/usr/bin/env python3
"""
Tier 2 Integration Test: Path Parsing Rules
Verifies that metadata is extracted from file paths using regex rules defined in config.toml.

> [!CRITICAL]
> **TEST ISOLATION MANDATE**
> All tests MUST use the dedicated **TEST QDRANT INSTANCE** (`qdrant-test`) running on ports **6335 (HTTP)** and **6336 (gRPC)**.
"""
import os
import sys
import shutil
import subprocess
import json
import time

# Setup
VECDB_BIN = "./target/debug/vecdb"
TEST_DIR = "tests/run/tier2_path_parsing"
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
        time.sleep(5) 
        
    except subprocess.CalledProcessError as e:
        print(f"CRITICAL FAIL: Could not manage test container: {e}")
        sys.exit(1)

def setup():
    ensure_test_qdrant()

    if os.path.exists(TEST_DIR):
        shutil.rmtree(TEST_DIR)
    os.makedirs(TEST_DIR)
    
    # Create nested data structure with TWO years to verify filtering
    
    # 1. 2025 Data
    dir_2025 = f"{TEST_DIR}/data/2025/Q1"
    os.makedirs(dir_2025)
    with open(f"{dir_2025}/strategy.txt", "w") as f:
        f.write("Our strategy for 2025 is to leverage path parsing for smarter retrieval.")

    # 2. 2024 Data
    dir_2024 = f"{TEST_DIR}/data/2024/Q4"
    os.makedirs(dir_2024)
    with open(f"{dir_2024}/strategy.txt", "w") as f:
        f.write("Our strategy for 2024 was focused on stability.")

    # Create Config
    config_content = """
smart_routing_keys = ["year", "quarter"]

[profiles.default]
qdrant_url = "http://localhost:6336"
embedding_model = "nomic-embed-text"
default_collection_name = "test_path_parsing"
embedder_type = "local"

[[ingestion.path_rules]]
# Matches: .../data/2025/Q1/strategy.txt
# Capture year and quarter
# Note: Ingestion root is .../data, so relative path is 2025/Q1/...
pattern = "(?P<year>\\\\d{4})/(?P<quarter>Q\\\\d)/.*"
    """
    
    with open(CONFIG_PATH, "w") as f:
        f.write(config_content)
        
    os.environ["VECDB_CONFIG"] = os.path.abspath(CONFIG_PATH)

def run_search(query, smart=False):
    cmd = [VECDB_BIN, "search", query, "--json"]
    if smart:
        cmd.append("--smart")
        
    result = subprocess.run(cmd, capture_output=True, text=True)
    if result.returncode != 0:
        print(f"Search failed: {result.stderr}")
        sys.exit(1)
        
    return json.loads(result.stdout)

def main():
    setup()
    
    # Init DB
    print("Initializing...")
    subprocess.run([VECDB_BIN, "init"], check=True, capture_output=True)
    
    # Clean previous run
    print("Cleaning collection...")
    res = subprocess.run([VECDB_BIN, "delete", "test_path_parsing", "--force"], capture_output=True, text=True)
    if res.returncode != 0:
         print(f"Warning: delete collection failed (ignored if first run): {res.stderr}")
    
    # Ingest
    print(f"Ingesting data from {TEST_DIR}/data...")
    if os.path.exists(f"{TEST_DIR}/data/.vecdb"):
        print("CRITICAL WARNING: .vecdb state dir exists before ingest!")
    cmd = [VECDB_BIN, "ingest", f"{TEST_DIR}/data"]
    subprocess.run(cmd, check=True)
    
    time.sleep(2) # Indexing wait
    
    print("\n--- Test 1: Verify Metadata Extraction & Basic Search ---")
    results = run_search("strategy")
    if len(results) != 2:
        print(f"FAIL: Expected 2 results (2024 & 2025), got {len(results)}")
        for r in results:
            print(f"- {r.get('content')[:50]}")
        sys.exit(1)
            
    # CHECK METADATA
    r0 = results[0]
    meta = r0.get("metadata", {})
    if "year" not in meta or "quarter" not in meta:
        print(f"FAIL: Metadata missing in result: {json.dumps(meta)}")
        sys.exit(1)

    print("PASS: Found both documents with metadata")

    print("\n--- Test 2: Smart Routing (2025) ---")
    # Query: "strategy 2025" should filter to year=2025
    results_2025 = run_search("strategy 2025", smart=True)
    
    if len(results_2025) != 1:
        print(f"FAIL: Expected 1 result for 2025, got {len(results_2025)}")
        for r in results_2025:
             print(f"- {r.get('content')[:50]}")
        sys.exit(1)
        
    if "2025" not in results_2025[0]['content']:
        print("FAIL: Returned document content does not contain 2025")
        sys.exit(1)
    print("PASS: Correctly filtered to 2025")

    print("\n--- Test 3: Smart Routing (2024) ---")
    # Query: "strategy 2024" should filter to year=2024
    results_2024 = run_search("strategy 2024", smart=True)
    
    if len(results_2024) != 1:
        print(f"FAIL: Expected 1 result for 2024, got {len(results_2024)}")
        sys.exit(1)
        
    if "2024" not in results_2024[0]['content']:
        print("FAIL: Returned document content does not contain 2024")
        sys.exit(1)
    print("PASS: Correctly filtered to 2024")

    print("\nALL TESTS PASSED")

if __name__ == "__main__":
    main()
