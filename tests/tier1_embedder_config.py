#!/usr/bin/env python3
"""Tier 1: a local (fastembed) embedder ingests and retrieves.

Asserts, each as a hard failure:

1. the CLI loads with this config;
2. the LOCAL embedder is the one that ran — a config naming a fastembed
   backend that silently resolved elsewhere would still ingest;
3. a query matching the fixture's text returns results;
4. the collection appears in `vecdb list`.

Item 5 of the previous list, "Configuration switching (if Ollama available)",
described something this file never did: `create_test_config(..., "ollama")`
exists and is never called with that argument.

Requires the test Qdrant (6336 gRPC / 6335 HTTP) — see
tests/fixtures/config.toml.
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

def genesis_model(collection):
    """The model name a collection's genesis point records, or None.

    Genesis lives at the nil UUID and is fetched by id — never by filtering on
    `__meta_vecdb`, whose value is a version string rather than a boolean, so a
    match filter on `true` silently returns nothing and the check passes for the
    wrong reason.
    """
    import urllib.request

    http_url = QDRANT_URL.replace(":6336", ":6335").replace(":6334", ":6333")
    try:
        req = urllib.request.Request(
            f"{http_url}/collections/{collection}/points",
            data=json.dumps(
                {"ids": ["00000000-0000-0000-0000-000000000000"], "with_payload": True}
            ).encode(),
            headers={"Content-Type": "application/json"},
            method="POST",
        )
        with urllib.request.urlopen(req, timeout=30) as r:
            points = json.load(r)["result"]
    except Exception as e:
        return f"<unreadable: {e}>"
    if not points:
        return None
    return points[0]["payload"].get("__meta_embedder_model")


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

    # Every check below FAILS the run.
    #
    # They used to log and continue — "⚠ Search returned no results (embedding
    # might need time)", "⚠ Collection not in list" — and `test_local_embedder`
    # returned True regardless. The one thing this file is named for, that a
    # local embedder ingests and retrieves, could not be falsified. It ran in
    # the gate and was counted in the total.
    #
    # `tests/tier0_tests_can_fail.py` now refuses that shape mechanically.
    failures = []
    try:
        # 1. The CLI must load with this config at all.
        result = run_vecdb(["--help"], check=False, config_path=config_path)
        if result.returncode != 0:
            failures.append(
                f"`vecdb --help` exited {result.returncode} with this config; "
                f"nothing below is meaningful.\n      {result.stderr[:300]}"
            )
        else:
            log("✓ CLI loads")

        # 2. Ingest. `run_vecdb(check=True)` already fails the run on non-zero.
        log("Ingesting test files...")
        result = run_vecdb(["ingest", fixture_dir, "-c", TEST_COLLECTION], config_path=config_path)

        # The configured model must be the one that wrote the collection.
        #
        # Asserted against GENESIS, not against stderr. A first cut here checked
        # for "Using local embedder" in the output — but `Core::new` prints that
        # only when `OUTPUT.is_interactive`, and a captured subprocess has no
        # TTY, so the check failed on a correct run. That is the same defect
        # this file is being repaired for: asserting a log line instead of the
        # artifact. The collection records the model that created it; that is
        # the fact worth pinning.
        # Two claims, deliberately separate:
        #   `fastembed:`         the LOCAL backend ran, not an Ollama one
        #   `bge-small-en-v1.5`  and it ran the configured model
        #
        # Not an equality check against the configured string. fastembed
        # resolves `BAAI/bge-small-en-v1.5` to Xenova's ONNX repo, so genesis
        # records `fastembed:Xenova/bge-small-en-v1.5`. That is correct — the
        # digest is what the space guard compares — and pinning the literal
        # would break on any upstream repo rename while proving nothing extra.
        model = genesis_model(TEST_COLLECTION) or ""
        if not model.startswith("fastembed:"):
            failures.append(
                f"genesis records model {model!r} — not a fastembed model, so "
                f"the local embedder is not what wrote this collection."
            )
        elif "bge-small-en-v1.5" not in model:
            failures.append(
                f"genesis records model {model!r}; the config names "
                f"'BAAI/bge-small-en-v1.5'. A different model embedded."
            )
        else:
            log(f"✓ genesis records a local fastembed run of the configured model ({model})")

        # 3. Retrieval. The fixture contains "Vector embeddings are numerical
        #    representations of text", so this query must hit it.
        log("Searching for 'vector embeddings'...")
        result = run_vecdb(
            ["search", "-c", TEST_COLLECTION, "--json", "vector embeddings"],
            config_path=config_path,
        )
        if not result.stdout.strip():
            failures.append("`search --json` produced no output at all")
        else:
            try:
                results = search_results(json.loads(result.stdout),
                                         context="vecdb search --json")
            except json.JSONDecodeError as e:
                failures.append(f"`search --json` did not emit JSON ({e}): "
                                f"{result.stdout[:200]}")
                results = []
            if not results:
                # Not a timing problem: ingest is synchronous and returned
                # already. An empty result set here means the corpus is empty
                # or the query embedded in a different space.
                failures.append(
                    "search returned no results from a collection just ingested. "
                    "Ingest is synchronous, so this is not indexing lag."
                )
            else:
                log(f"✓ search returned {len(results)} result(s), "
                    f"top score {results[0].get('score', 'N/A')}")

        # 4. The collection must be visible to `list`.
        result = run_vecdb(["list"], config_path=config_path)
        if TEST_COLLECTION not in result.stdout:
            failures.append(
                f"'{TEST_COLLECTION}' was ingested but does not appear in "
                f"`vecdb list`:\n      {result.stdout[:300]}"
            )
        else:
            log("✓ collection appears in list")

        if failures:
            for f in failures:
                print(f"[FAIL] {f}", file=sys.stderr)
            fail(f"{len(failures)} local-embedder check(s) failed")

        log("✓ Local embedder test passed")
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
