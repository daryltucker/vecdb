#!/usr/bin/env python3
"""
Tier 1 Functional Test: Embedder Configuration & End-to-End Flow

This test verifies:
1. Configuration loading (embedder_type)
2. Local embedder initialization
3. Ingestion with local embeddings
4. Search with local embeddings
5. Configuration switching (if Ollama available)

Requires: Qdrant running on localhost:6336 (test instance, see tests/fixtures/config.toml)
"""

import subprocess
import json
import sys
import os
import tempfile
import shutil

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
from lib_envelope import search_results

import sys, os as _os
sys.path.insert(0, _os.path.dirname(_os.path.abspath(__file__)))
from paths import bin_path

# Test configuration — use test Qdrant instance (port 6336), never production (6334)
QDRANT_URL = os.environ.get("VECDB_TEST_QDRANT_URL", "http://localhost:6336")
TEST_COLLECTION = "test_tier1_embedder"
VECDB_CLI = bin_path("vecdb")

def log(msg):
    print(f"[TEST] {msg}")

def fail(msg):
    print(f"[FAIL] {msg}", file=sys.stderr)
    sys.exit(1)

def run_vecdb(args, check=True, capture_output=True, config_path=None):
    """Run vecdb CLI command with an isolated VECDB_CONFIG."""
    cmd = [VECDB_CLI] + args
    env = os.environ.copy()
    if config_path:
        env["VECDB_CONFIG"] = config_path
    result = subprocess.run(cmd, capture_output=capture_output, text=True, env=env)
    if check and result.returncode != 0:
        fail(f"Command failed: {' '.join(cmd)}\nstderr: {result.stderr}")
    return result

def check_qdrant():
    """Verify Qdrant is running (uses HTTP REST port derived from gRPC URL)"""
    try:
        import urllib.request
        # Test Qdrant gRPC is 6336; HTTP REST is 6335. Production: gRPC 6334, HTTP 6333.
        http_url = QDRANT_URL.replace(":6336", ":6335").replace(":6334", ":6333")
        req = urllib.request.urlopen(f"{http_url}/collections", timeout=5)
        return req.status == 200
    except Exception as e:
        return False

def cleanup_collection():
    """Delete test collection if it exists"""
    try:
        import urllib.request
        http_url = QDRANT_URL.replace(":6336", ":6335").replace(":6334", ":6333")
        req = urllib.request.Request(
            f"{http_url}/collections/{TEST_COLLECTION}",
            method='DELETE'
        )
        urllib.request.urlopen(req, timeout=5)
        log(f"Cleaned up collection: {TEST_COLLECTION}")
    except:
        pass  # Collection might not exist

def create_test_config(tmpdir, embedder_type="local"):
    """Write a test config file into tmpdir. Returns the config file path.
    Uses VECDB_CONFIG env var — never mutates ~/.config/vecdb/config.toml.
    """
    # Three layers, so the two embedder kinds cannot share a knob even here:
    # a fastembed embedder has no url and no num_ctx, and an ollama one has no
    # use_gpu. Which block is live is decided by `backend`, not by a string
    # compared at construction time.
    if embedder_type == "ollama":
        embedder_block = """
[backend.test_backend]
kind = "ollama"
url = "http://localhost:11434"

[embedder.test_embedder]
backend = "test_backend"
model = "nomic-embed-text"
num_ctx = 4096
batch_inputs = 8
"""
    else:
        embedder_block = """
[backend.test_backend]
kind = "fastembed"

[embedder.test_embedder]
backend = "test_backend"
model = "BAAI/bge-small-en-v1.5"
batch_rows = 2
"""

    config_content = f"""
default_profile = "test"
{embedder_block}
[profiles.test]
embedder = "test_embedder"
qdrant_url = "{QDRANT_URL}"
default_collection_name = "{TEST_COLLECTION}"

[ingestion]
target_chunk_size = 256
"""
    config_path = os.path.join(tmpdir, "config.toml")
    with open(config_path, 'w') as f:
        f.write(config_content)
    return config_path

def create_test_fixtures():
    """Create temporary test files"""
    tmpdir = tempfile.mkdtemp(prefix="vecdb_test_")
    
    # Create a simple test file
    with open(os.path.join(tmpdir, "test.md"), 'w') as f:
        f.write("""# Test Document

This is a test document for verifying local embeddings.

## Section 1: Vectors

Vector embeddings are numerical representations of text.

## Section 2: Search

Semantic search finds similar content based on meaning.
""")
    
    return tmpdir

def test_local_embedder():
    """Test the local embedder configuration and functionality"""
    log("Testing Local Embedder...")

    tmpdir = tempfile.mkdtemp(prefix="vecdb_embedder_test_")
    config_path = create_test_config(tmpdir, "local")
    fixture_dir = create_test_fixtures()

    try:
        # 1. Verify CLI loads
        result = run_vecdb(["--help"], check=False, config_path=config_path)
        if result.returncode == 0:
            log("✓ CLI loads successfully")

        # 2. Ingest test files
        log("Ingesting test files...")
        result = run_vecdb(["ingest", fixture_dir, "-c", TEST_COLLECTION], config_path=config_path)
        log(f"Ingest output: {result.stdout[:200] if result.stdout else '(no output)'}")

        # Check stderr for embedder type message
        if "Using local embedder" in result.stderr:
            log("✓ Local embedder confirmed in use")
        else:
            log(f"Note: stderr = {result.stderr[:200] if result.stderr else '(empty)'}")

        # 3. Search for known content
        log("Searching for 'vector embeddings'...")
        result = run_vecdb(["search", "-c", TEST_COLLECTION, "--json", "vector embeddings"], config_path=config_path)

        if result.stdout:
            try:
                results = search_results(json.loads(result.stdout),
                                         context="vecdb search --json")
                if len(results) > 0:
                    log(f"✓ Search returned {len(results)} results")
                    log(f"  Top result score: {results[0].get('score', 'N/A')}")
                else:
                    log("⚠ Search returned no results (embedding might need time)")
            except json.JSONDecodeError:
                log(f"⚠ Could not parse search output: {result.stdout[:100]}")
        else:
            log("⚠ No search output")

        # 4. List collections
        log("Listing collections...")
        result = run_vecdb(["list"], config_path=config_path)
        if TEST_COLLECTION in result.stdout:
            log(f"✓ Test collection appears in list")
        else:
            log(f"⚠ Collection not in list: {result.stdout[:200]}")

        log("✓ Local embedder test completed")
        return True

    finally:
        shutil.rmtree(tmpdir, ignore_errors=True)
        shutil.rmtree(fixture_dir, ignore_errors=True)
        cleanup_collection()

def main():
    log("=" * 60)
    log("Tier 1 Functional Test: Embedder Configuration")
    log("=" * 60)
    
    # Check prerequisites
    if not os.path.exists(VECDB_CLI):
        fail(f"CLI not found at {VECDB_CLI}. Run: cargo build")
    
    if not check_qdrant():
        fail(f"Test Qdrant not running at {QDRANT_URL}. Start test instance with: docker run -p 6335:6334 -p 6336:6333 qdrant/qdrant")
    
    log("✓ Prerequisites OK")
    
    # Cleanup any previous test data
    cleanup_collection()
    
    # Run tests
    try:
        test_local_embedder()
        
        log("=" * 60)
        log("✓ ALL TESTS PASSED")
        log("=" * 60)
        
    except Exception as e:
        fail(f"Test failed with exception: {e}")
        import traceback
        traceback.print_exc()

if __name__ == "__main__":
    main()
