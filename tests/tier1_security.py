#!/usr/bin/env python3
"""
Tier 1 Security Test
Purpose: Verify that security gates work correctly.
Scope: ingest_path MUST fail without --allow-local-fs
"""

import sys
import json
import subprocess
import os

SERVER_BIN = "target/debug/vecdb-server"

def log(msg):
    print(msg, file=sys.stderr)

def run_security_test():
    root_dir = os.path.abspath(os.path.join(os.path.dirname(__file__), ".."))
    server_path = os.path.join(root_dir, SERVER_BIN)
    
    if not os.path.exists(server_path):
        log(f"ERROR: Server binary not found at {server_path}.")
        return False

    # Start server WITHOUT --allow-local-fs (default secure mode)
    process = subprocess.Popen(
        [server_path],  # NO --allow-local-fs flag
        stdin=subprocess.PIPE,
        stdout=subprocess.PIPE,
        stderr=subprocess.PIPE,
        text=True,
        bufsize=0
    )

    log("=== Tier 1 Security Test: API-Only Mode ===")

    def send_request(method, params, req_id):
        req = {"jsonrpc": "2.0", "id": req_id, "method": method, "params": params}
        process.stdin.write(json.dumps(req) + "\n")
        process.stdin.flush()
        line = process.stdout.readline()
        if not line:
            return None
        return json.loads(line)

    try:
        # Initialize
        send_request("initialize", {"capabilities": {}}, 1)
        process.stdin.write(json.dumps({"jsonrpc": "2.0", "method": "notifications/initialized"}) + "\n")
        
        # Attempt ingest_path (SHOULD FAIL)
        log("[1/2] Attempting ingest_path without --allow-local-fs...")
        resp = send_request("tools/call", {
            "name": "ingest_path",
            "arguments": {
                "path": "/tmp/test",
                "collection": "test"
            }
        }, 2)
        
        # Check for security error
        if "error" in resp and "Security Error" in resp["error"].get("message", ""):
            log("SUCCESS: ingest_path correctly blocked.")
        else:
            log(f"FAIL: ingest_path should have been blocked. Got: {resp}")
            return False
        
        # Verify safe tools still work (embed)
        log("[2/2] Verifying 'embed' still works in API-only mode...")
        resp = send_request("tools/call", {
            "name": "embed",
            "arguments": {"texts": ["test"]}
        }, 3)
        
        if "error" in resp:
            log(f"FAIL: embed should work. Got error: {resp['error']}")
            return False
        
        log("SUCCESS: embed works in API-only mode.")
        return True

    except Exception as e:
        log(f"EXCEPTION: {e}")
        return False
    finally:
        process.terminate()

if __name__ == "__main__":
    if run_security_test():
        sys.exit(0)
    else:
        sys.exit(1)
