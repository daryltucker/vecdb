#!/usr/bin/env python3
import subprocess
import os
import shutil
import time
import json

# Configuration
VECDB_BINARY = "./target/debug/vecdb"
VECQ_BINARY = "./target/debug/vecq"
TEST_REPO_DIR = "tests_tmp_repo"

def run_cmd(cmd, cwd=".", check=True):
    # eprint(f"Running: {cmd}")
    result = subprocess.run(cmd, shell=True, cwd=cwd, capture_output=True, text=True)
    if check and result.returncode != 0:
        print(f"Command failed: {cmd}")
        print("STDOUT:", result.stdout)
        print("STDERR:", result.stderr)
        exit(1)
    return result

def setup_test_repo():
    if os.path.exists(TEST_REPO_DIR):
        shutil.rmtree(TEST_REPO_DIR)
    os.makedirs(TEST_REPO_DIR)
    
    run_cmd("git init", cwd=TEST_REPO_DIR)
    run_cmd('git config user.email "test@example.com"', cwd=TEST_REPO_DIR)
    run_cmd('git config user.name "Test User"', cwd=TEST_REPO_DIR)
    
    # Commit 1
    with open(f"{TEST_REPO_DIR}/main.rs", "w") as f:
        f.write("fn main() { println!(\"Version 1\"); }")
    run_cmd("git add .", cwd=TEST_REPO_DIR)
    run_cmd("git commit -m 'Initial commit'", cwd=TEST_REPO_DIR)
    rev1 = run_cmd("git rev-parse HEAD", cwd=TEST_REPO_DIR).stdout.strip()
    
    # Commit 2
    with open(f"{TEST_REPO_DIR}/main.rs", "w") as f:
        f.write("fn main() { println!(\"Version 2 - Changed\"); }")
    run_cmd("git add .", cwd=TEST_REPO_DIR)
    run_cmd("git commit -m 'Update to V2'", cwd=TEST_REPO_DIR)
    rev2 = run_cmd("git rev-parse HEAD", cwd=TEST_REPO_DIR).stdout.strip()
    
    return rev1, rev2

def test_history_ingest():
    print("--- Setting up Test Repo ---")
    rev1, rev2 = setup_test_repo()
    print(f"Rev1: {rev1}")
    print(f"Rev2: {rev2}")

    # Build binaries first to ensure readiness
    # run_cmd("cargo build --release --workspace")

    print("\n--- Ingesting Version 2 (Current) ---")
    # Standard ingest
    run_cmd(f"{VECDB_BINARY} ingest {TEST_REPO_DIR} --collection history_test")
    
    print("\n--- Ingesting Version 1 (Time Travel) ---")
    # Use absolute path for safety/clarity in test
    abs_repo_path = os.path.abspath(TEST_REPO_DIR)
    run_cmd(f"{VECDB_BINARY} history ingest -r {rev1} {abs_repo_path} --collection history_test")

    print("\n--- Verifying Results ---")
    # We search for "Version" which should appear in both
    # We expect 2 chunks, one matching V1 and one matching V2
    
    # Check V1 content
    search_cmd_v1 = f"{VECDB_BINARY} search --json 'Version 1' --collection history_test"
    out_v1 = run_cmd(search_cmd_v1).stdout
    print("V1 Search Output:", out_v1)
    
    # Check V2 content
    search_cmd_v2 = f"{VECDB_BINARY} search --json 'Version 2' --collection history_test"
    out_v2 = run_cmd(search_cmd_v2).stdout
    print("V2 Search Output:", out_v2)
    
    results_v1 = json.loads(out_v1)
    results_v2 = json.loads(out_v2)
    
    # Validation logic
    found_v1 = any("Version 1" in r['content'] for r in results_v1)
    found_v2 = any("Version 2" in r['content'] for r in results_v2)
    
    if not found_v1:
        print("FAIL: Did not find 'Version 1' content from historic revision.")
        exit(1)
        
    if not found_v2:
        print("FAIL: Did not find 'Version 2' content from current revision.")
        exit(1)

    # Check Metadata Divergence
    # One result should represent rev1, other rev2
    # Note: Search results might mix them, so let's deep inspect metadata if possible
    # But since IDs are unique, we just confirmed both exist in the index simultaneously.
    
    print("SUCCESS: Both versions coexist in the vector index.")

    # Cleanup
    shutil.rmtree(TEST_REPO_DIR)

if __name__ == "__main__":
    test_history_ingest()
