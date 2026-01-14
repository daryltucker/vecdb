
import sys
import json
import subprocess
import os
import time

try:
    import tomllib  # Python 3.11+
except ImportError:
    try:
        import tomli as tomllib
    except ImportError:
        print("ERROR: Need Python 3.11+ or 'pip install tomli'", file=sys.stderr)
        sys.exit(1)

# Tier 1 MCP Verification Script
# Purpose: Functional test of the vecdb-mcp server
# Scope: Initialization -> Ingestion (local) -> Search (semantic)

SERVER_BIN = "target/debug/vecdb-server"
LUA_FIXTURE_PATH = "tests/fixtures/external/tiny_tier1"

def log(msg, color=None):
    # Simple stderr logging
    print(msg, file=sys.stderr)

def load_test_config():
    """Load test configuration to get Qdrant URL"""
    config_path = os.path.join(os.path.dirname(__file__), "fixtures", "config.toml")
    with open(config_path, "rb") as f:
        config = tomllib.load(f)
    return config.get("qdrant_url", "http://localhost:6333")  # Default to prod if missing

def cleanup_tier1():
    """Cleanup phase - remove state files so ingestion actually processes files"""
    state_file = os.path.join(
        os.path.dirname(__file__),
        "..",
        LUA_FIXTURE_PATH,
        ".vecdb",
        "state.toml"
    )
    state_file = os.path.abspath(state_file)
    if os.path.exists(state_file):
        os.remove(state_file)
        log(f"[CLEANUP] Removed {state_file}")

def run_tier1_test():
    # Tier 1 Cleanup - ensure clean state
    cleanup_tier1()
    
    # Load test config and set Qdrant URL
    test_qdrant_url = load_test_config()
    log(f"[CONFIG] Using test Qdrant: {test_qdrant_url}")
    
    root_dir = os.path.abspath(os.path.join(os.path.dirname(__file__), ".."))
    server_path = os.path.join(root_dir, SERVER_BIN)
    lua_path = os.path.join(root_dir, LUA_FIXTURE_PATH)
    
    if not os.path.exists(server_path):
        log(f"ERROR: Server binary not found at {server_path}. Run 'cargo build --bin vecdb-server' first.")
        return False
        
    if not os.path.exists(lua_path):
        log(f"ERROR: Fixture not found at {lua_path}.")
        return False

    # Start Server with test Qdrant URL
    # Note: --allow-local-fs is REQUIRED for ingest_path
    config_path = os.path.join(os.path.dirname(__file__), "fixtures", "config.toml")
    process = subprocess.Popen(
        [server_path, '--allow-local-fs', '--stdio'],
        stdin=subprocess.PIPE,
        stdout=subprocess.PIPE,
        stderr=sys.stderr,
        text=True,
        bufsize=0,
        env={
            **os.environ, 
            "QDRANT_URL": test_qdrant_url,
            "VECDB_CONFIG": os.path.abspath(config_path) 
        }
    )

    log(f"=== Tier 1 Functional Test: {SERVER_BIN} ===")

    def send_request(method, params, req_id):
        req = {
            "jsonrpc": "2.0",
            "id": req_id,
            "method": method,
            "params": params
        }
        process.stdin.write(json.dumps(req) + "\n")
        process.stdin.flush()
        
        line = process.stdout.readline()
        if not line:
            return None
        return json.loads(line)

    try:
        # 1. Initialize
        log("[1/4] Initializing...")
        send_request("initialize", {"capabilities": {}, "clientInfo": {"name": "tier1", "version": "1.0"}}, 1)
        process.stdin.write(json.dumps({"jsonrpc": "2.0", "method": "notifications/initialized"}) + "\n")
        
        # 2. Verify Tool Existence
        log("[2/4] Verifying Tools...")
        resp = send_request("tools/list", {}, 2)
        tools = [t['name'] for t in resp['result']['tools']]
        if "ingest_path" not in tools:
            log("FAIL: ingest_path tool missing.")
            return False
            
        # 3. Ingest Data (Sync)
        # Note: vecdb-server handles this synchronously for now.
        log(f"[3/4] Ingesting {LUA_FIXTURE_PATH}...")
        start_time = time.time()
        resp = send_request("tools/call", {
            "name": "ingest_path",
            "arguments": {
                "path": lua_path,
                "collection": "tier1_lua"
            }
        }, 3)
        duration = time.time() - start_time
        
        if "error" in resp:
            log(f"FAIL: Ingestion failed: {resp['error']}")
            return False
        log(f"      Ingestion complete in {duration:.2f}s")
        
        # 4. Embed (Direct Call)
        log("[4/5] Testing direct 'embed' call...")
        resp = send_request("tools/call", {
            "name": "embed",
            "arguments": {
                "texts": ["hello world"]
            }
        }, 4)
        
        if "error" in resp:
            log(f"FAIL: Embed failed: {resp['error']}")
            return False
        
        embed_json = resp['result']['content'][0]['text']
        vectors = json.loads(embed_json)
        if len(vectors) != 1 or len(vectors[0]) == 0:
            log("FAIL: Embed returned invalid vector")
            return False
        log("SUCCESS: Embed tool functional.")

        # 5. Search
        log("[5/5] Searching for 'bananas'...")
        resp = send_request("tools/call", {
            "name": "search_vectors",
            "arguments": {
                "query": "bananas",
                "collection": "tier1_lua",
                "json": True,
                "smart": False
            }
        }, 4)
        
        if "error" in resp:
            log(f"FAIL: Search failed: {resp['error']}")
            return False
            
        content_json = resp['result']['content'][0]['text']
        results = json.loads(content_json)
        
        if len(results) > 0:
            log(f"SUCCESS: Found {len(results)} matches.")
            # SearchResult has 'metadata', not 'payload'
            filepath = results[0].get('metadata', {}).get('path', 'unknown')
            log(f"         Top match: {filepath}")
            return True
        else:
            log("FAIL: No results returned.")
            return False

    except Exception as e:
        log(f"EXCEPTION: {e}")
        return False
    finally:
        process.terminate()

if __name__ == "__main__":
    if run_tier1_test():
        sys.exit(0)
    else:
        sys.exit(1)
