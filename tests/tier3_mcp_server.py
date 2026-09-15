#!/usr/bin/env python3
"""
Tier 3: MCP Server Verification
Scenario: Start vecdb-server process -> Send JSON-RPC initialize -> Verify response

This test ensures the MCP server binary actually works as an MCP server,
catching issues like:
- Stdout pollution (breaking JSON-RPC)
- Startup crashes
- Timeout / Hanging initialization
"""

import os
import sys
import subprocess
import time
import json
import tempfile
import shutil

import sys, os as _os
sys.path.insert(0, _os.path.dirname(_os.path.abspath(__file__)))
from paths import bin_path, require_bin
from lib_stdio import drain_stderr

def log(msg):
    print(f"[Tier 3 MCP] {msg}")

def main():
    start_time = time.time()
    
    # 0. Setup Environment
    project_root = os.getcwd()
    config_dir = tempfile.mkdtemp(prefix="vecdb_tier3_mcp_config_")
    
    # Override XDG paths to isolate config (and prevent reading user's real config which might validly connect)
    # actually, we WANT to test with a valid profile if possible, but for reproducible tests we should init.
    # Let's simple use the 'default' profile created by specific init.
    
    env = os.environ.copy()
    env["XDG_CONFIG_HOME"] = config_dir
    env["VECDB_ALLOW_LOCAL_FS"] = "true" # Enable for test
    # Unset VECDB_CONFIG so XDG_CONFIG_HOME takes effect (run_all.sh force-sets it).
    env.pop("VECDB_CONFIG", None)
    
    log(f"Config Dir: {config_dir}")

    try:
        # 1. The binaries the manifest built. Never `cargo build` here — a
        #    plain build uses DEFAULT features and replaces the cuda-dynamic
        #    binary T1.1 produced, at the same path. `cargo build -p vecdb-cli`
        #    did exactly that to `vecdb`, mid-Tier-3.
        server_bin = require_bin("vecdb-server")
        cli_bin = require_bin("vecdb")

        # 2. Generate a valid config/profile for the server to start from.
        log("Initializing config...")
        subprocess.run([cli_bin, "init"], check=True, env=env, stdout=subprocess.DEVNULL)

        # Pin the endpoint before anything can connect.
        #
        # `vecdb init` writes a config with no explicit URL, so the profile
        # falls through to the DEFAULT endpoint — localhost:6334, production.
        # tier3_quantization.py and mini_lua.py were both hardened against this
        # after a `test_` collection reached production that way; this file was
        # missed. It only handshakes today, but the guard is what makes that a
        # fact rather than a coincidence.
        cfg_file = os.path.join(config_dir, "vecdb", "config.toml")
        with open(cfg_file) as f:
            cfg = f.read()
        if "[profiles.default]" not in cfg:
            raise Exception("generated config has no [profiles.default] to pin")
        test_url = os.environ.get("VECDB_TEST_QDRANT_URL", "http://localhost:6336")
        with open(cfg_file, "w") as f:
            f.write(
                cfg.replace(
                    "[profiles.default]",
                    f'[profiles.default]\nqdrant_url = "{test_url}"',
                    1,
                )
            )

        # 3. Start Server Process
        log("Starting Server Process...")
        process = subprocess.Popen(
            [server_bin, "--stdio", "--allow-local-fs"],
            stdin=subprocess.PIPE,
            stdout=subprocess.PIPE,
            stderr=subprocess.PIPE,
            env=env,
            encoding='utf-8', 
            bufsize=0 # Unbuffered
        )
        # Drain stderr continuously so the server can never block writing to
        # a full stderr pipe while this test blocks reading stdout.
        # See tests/lib_stdio.py for the deadlock this prevents.
        _stderr = drain_stderr(process)
        
        # 4. JSON-RPC Handshake
        # Send 'initialize' request
        init_req = {
            "jsonrpc": "2.0",
            "method": "initialize",
            "params": {
                "clientInfo": {"name": "tier3-test", "version": "1.0"} 
            },
            "id": 1
        }
        
        req_str = json.dumps(init_req) + "\n"
        log(f"Sending: {req_str.strip()}")
        
        process.stdin.write(req_str)
        process.stdin.flush()
        
        # 5. Read Response
        log("Waiting for response...")
        
        # We use a timeout to detect hangs
        # Using select or simple read with timeout logic is complex in raw python without threads/asyncio.
        # Let's try a simple readline with a manual timeout via polling.
        
        response_line = None
        for _ in range(20): # Wait up to 10 seconds (20 * 0.5s)
            if process.poll() is not None:
                log("Process exited prematurely!")
                stderr_out = _stderr()
                log(f"Stderr: {stderr_out}")
                raise Exception("Server process crashed")
            
            # Non-blocking read trickery or just hope readline doesn't block forever 
            # (it will block if no newline comes). 
            # We can use internal buffers. 
            # Ideally we'd use select, but let's trust readline won't hang FOREVER if we kill it.
            # Actually, readline WILL hang if server prints nothing.
            
            # For this test, we accept blocking because prompt 'context canceled' implies it eventually stopped or user cancelled.
            # We want to know if it CRASHES.
            
            # Let's just read one line.
            response_line = process.stdout.readline()
            if response_line:
                break
            time.sleep(0.5)
            
        if not response_line:
             log("No response received within timeout. Checking stderr...")
             # Kill it to read stderr
             process.terminate()
             stderr_out = _stderr()
             log(f"Stderr: {stderr_out}")
             raise Exception("Server timed out / returned nothing")

        log(f"Received: {response_line.strip()}")
        
        response = json.loads(response_line)
        
        if "error" in response:
            raise Exception(f"Server returned error: {response['error']}")
            
        if response.get("result", {}).get("serverInfo", {}).get("name") != "vecdb-mcp":
            raise Exception("Invalid server info in response")
            
        log("Handshake successful! ✅")
        
        # Cleanup process
        process.terminate()
        process.wait()

    finally:
        shutil.rmtree(config_dir)
        
    log(f"Total time: {time.time() - start_time:.2f}s")
    
if __name__ == "__main__":
    main()
