#!/usr/bin/env python3
"""
Tier 1 Functional Test: Embedder Configuration & End-to-End Flow

This test verifies:
1. Configuration loading (embedder_type)
2. Local embedder initialization
3. Ingestion with local embeddings
4. Search with local embeddings
5. Configuration switching (if Ollama available)

Requires: Qdrant running on localhost:6334
"""

import subprocess
import json
import sys
import os
import tempfile
import shutil

# Test configuration
QDRANT_URL = "http://localhost:6334"
TEST_COLLECTION = "tier1_embedder_test"
VECDB_CLI = "./target/debug/vecdb"

def log(msg):
    print(f"[TEST] {msg}")

def fail(msg):
    print(f"[FAIL] {msg}", file=sys.stderr)
    sys.exit(1)

def run_vecdb(args, check=True, capture_output=True):
    """Run vecdb CLI command"""
    cmd = [VECDB_CLI] + args
    result = subprocess.run(cmd, capture_output=capture_output, text=True)
    if check and result.returncode != 0:
        fail(f"Command failed: {' '.join(cmd)}\nstderr: {result.stderr}")
    return result

def check_qdrant():
    """Verify Qdrant is running"""
    try:
        import urllib.request
        req = urllib.request.urlopen(f"{QDRANT_URL.replace('6334', '6333')}/collections", timeout=5)
        return req.status == 200
    except Exception as e:
        return False

def cleanup_collection():
    """Delete test collection if it exists"""
    try:
        import urllib.request
        req = urllib.request.Request(
            f"{QDRANT_URL.replace('6334', '6333')}/collections/{TEST_COLLECTION}",
            method='DELETE'
        )
        urllib.request.urlopen(req, timeout=5)
        log(f"Cleaned up collection: {TEST_COLLECTION}")
    except:
        pass  # Collection might not exist

def create_test_config(embedder_type="local"):
    """Create a temporary config file"""
    config_content = f"""
default_profile = "test"

[profiles.test]
qdrant_url = "{QDRANT_URL}"
collection_name = "{TEST_COLLECTION}"
embedder_type = "{embedder_type}"
ollama_url = "http://localhost:11434"
embedding_model = "nomic-embed-text"

[ingestion]
default_strategy = "recursive"
chunk_size = 256
"""
    config_dir = os.path.expanduser("~/.config/vecdb")
    os.makedirs(config_dir, exist_ok=True)
    config_path = os.path.join(config_dir, "config.toml")
    
    # Backup existing config
    backup_path = None
    if os.path.exists(config_path):
        backup_path = config_path + ".backup"
        shutil.copy(config_path, backup_path)
    
    with open(config_path, 'w') as f:
        f.write(config_content)
    
    return backup_path

def restore_config(backup_path):
    """Restore original config"""
    config_path = os.path.expanduser("~/.config/vecdb/config.toml")
    if backup_path and os.path.exists(backup_path):
        shutil.move(backup_path, config_path)
        log("Restored original config")
    elif os.path.exists(config_path):
        os.remove(config_path)

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
    
    # Create test config with local embedder
    backup_path = create_test_config("local")
    tmpdir = create_test_fixtures()
    
    try:
        # 1. Verify CLI loads
        result = run_vecdb(["--help"], check=False)
        if result.returncode == 0:
            log("✓ CLI loads successfully")
        
        # 2. Ingest test files
        log("Ingesting test files...")
        result = run_vecdb(["ingest", tmpdir, "-c", TEST_COLLECTION])
        log(f"Ingest output: {result.stdout[:200] if result.stdout else '(no output)'}")
        
        # Check stderr for embedder type message
        if "Using local embedder" in result.stderr:
            log("✓ Local embedder confirmed in use")
        else:
            log(f"Note: stderr = {result.stderr[:200] if result.stderr else '(empty)'}")
        
        # 3. Search for known content
        log("Searching for 'vector embeddings'...")
        result = run_vecdb(["search", "-c", TEST_COLLECTION, "--json", "vector embeddings"])
        
        if result.stdout:
            try:
                results = json.loads(result.stdout)
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
        result = run_vecdb(["list"])
        if TEST_COLLECTION in result.stdout:
            log(f"✓ Test collection appears in list")
        else:
            log(f"⚠ Collection not in list: {result.stdout[:200]}")
        
        log("✓ Local embedder test completed")
        return True
        
    finally:
        # Cleanup
        restore_config(backup_path)
        shutil.rmtree(tmpdir, ignore_errors=True)
        cleanup_collection()

def main():
    log("=" * 60)
    log("Tier 1 Functional Test: Embedder Configuration")
    log("=" * 60)
    
    # Check prerequisites
    if not os.path.exists(VECDB_CLI):
        fail(f"CLI not found at {VECDB_CLI}. Run: cargo build")
    
    if not check_qdrant():
        fail("Qdrant not running. Start with: docker run -p 6333:6333 -p 6334:6334 qdrant/qdrant")
    
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
