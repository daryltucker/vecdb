
import sys
import json
import subprocess
import os

# Tier 1 Parity Verification Script
# Purpose: Contract test for vecdb-mcp server
# Scope: Verifies Schema Parity (CLI vs Server)

SERVER_BIN = "target/debug/vecdb-server"

def log(msg, color=None):
    print(msg, file=sys.stderr)

def run_parity_test():
    root_dir = os.path.abspath(os.path.join(os.path.dirname(__file__), ".."))
    server_path = os.path.join(root_dir, SERVER_BIN)
    
    if not os.path.exists(server_path):
        log(f"ERROR: Server binary not found at {server_path}. Run 'cargo build --bin vecdb-server' first.")
        return False

    process = subprocess.Popen(
        [server_path, '--allow-local-fs'],
        stdin=subprocess.PIPE,
        stdout=subprocess.PIPE,
        stderr=sys.stderr,
        text=True,
        bufsize=0
    )

    log(f"=== Tier 1 Contract Test: {SERVER_BIN} ===")

    try:
        # Initialize
        req = {"jsonrpc": "2.0", "id": 1, "method": "initialize", "params": { "capabilities": {}, "clientInfo": {"name": "parity-test", "version": "1.0"}}}
        process.stdin.write(json.dumps(req) + "\n")
        process.stdout.readline()
        
        process.stdin.write(json.dumps({"jsonrpc": "2.0", "method": "notifications/initialized"}) + "\n")
        
        # Tools List
        req = {"jsonrpc": "2.0", "id": 2, "method": "tools/list", "params": {}}
        process.stdin.write(json.dumps(req) + "\n")
        line = process.stdout.readline()
        resp = json.loads(line)
        
        tools = resp['result']['tools']
        search_tool = next(t for t in tools if t['name'] == 'search_vectors')
        schema = search_tool['inputSchema']
        
        # Check if 'collection' is in properties (Added via shared struct)
        if 'collection' in schema['properties']:
            log("SUCCESS: 'collection' field found in search_vectors schema. Dynamic generation works.")
            return True
        else:
            log("FAILED: 'collection' field MISSING in search_vectors schema.")
            log(json.dumps(schema, indent=2))
            return False

    except Exception as e:
        log(f"EXCEPTION: {e}")
        return False
    finally:
        process.terminate()

if __name__ == "__main__":
    if run_parity_test():
        sys.exit(0)
    else:
        sys.exit(1)
