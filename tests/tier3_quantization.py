#!/usr/bin/env python3
import os
import shutil
import subprocess
import sys
import time
import json
import urllib.request

import sys, os as _os
sys.path.insert(0, _os.path.dirname(_os.path.abspath(__file__)))
from paths import bin_path

# Setup
BINARY = bin_path("vecdb")
TEMP_HOME = "./out/test_quantization_home"
# ALL TESTS MUST USE TEST QDRANT — NEVER PRODUCTION (6333/6334)
# VECDB_TEST_QDRANT_HTTP_URL is the HTTP REST port (6335). vecdb itself uses gRPC (6336).
QDRANT_URL = os.environ.get("VECDB_TEST_QDRANT_HTTP_URL", "http://localhost:6335")

def cleanup():
    if os.path.exists(TEMP_HOME):
        shutil.rmtree(TEMP_HOME)

def delete_collection(name):
    """Delete a Qdrant collection via REST API (best-effort, ignores 404)."""
    req = urllib.request.Request(
        f"{QDRANT_URL}/collections/{name}",
        method="DELETE"
    )
    try:
        urllib.request.urlopen(req)
    except urllib.error.HTTPError as e:
        if e.code != 404:
            print(f"Warning: failed to delete collection {name}: {e}")

def run_vecdb(args, check=True):
    env = os.environ.copy()
    env["HOME"] = os.path.abspath(TEMP_HOME)
    env["XDG_CONFIG_HOME"] = os.path.join(env["HOME"], ".config")
    # Unset VECDB_CONFIG so XDG_CONFIG_HOME takes effect (run_all.sh force-sets it).
    env.pop("VECDB_CONFIG", None)
    result = subprocess.run([BINARY] + args, env=env, capture_output=True, text=True)
    if check and result.returncode != 0:
        print(f"Command failed: vecdb {' '.join(args)}")
        print("STDOUT:", result.stdout)
        print("STDERR:", result.stderr)
        sys.exit(1)
    return result

def check_collection_quantization(collection_name):
    url = f"{QDRANT_URL}/collections/{collection_name}"
    try:
        with urllib.request.urlopen(url) as response:
            data = json.loads(response.read().decode())
            return data.get("result", {}).get("config", {}).get("quantization_config")
    except Exception as e:
        print(f"Failed to query Qdrant: {e}")
        return None

def main():
    print("=== Tier 3 Quantization Verification ===")
    cleanup()
    delete_collection("test_quant_coll")
    os.makedirs(TEMP_HOME, exist_ok=True)

    # 1. Init
    print("[1] Initializing...")
    run_vecdb(["init"])

    # Point the generated config at test Qdrant.
    #
    # This test runs against a config `vecdb init` writes into a temp HOME, not
    # against the shared fixture, so the isolation gate cannot vet it — the gate
    # inspects the fixture and the test sources, and this file is created at
    # runtime. The endpoint therefore has to be pinned here, and PROVEN pinned
    # before anything is written.
    #
    # It used to be a regex replacing `qdrant_url = "http://localhost:NNNN"`.
    # Once `init` moved to named stores it emitted no such line, the substitution
    # silently matched nothing, and the ingest fell through to the default
    # endpoint — writing a `test_` collection into PRODUCTION. A patch that
    # quietly does nothing is worse than no patch, so this appends an explicit
    # setting and then verifies it took.
    test_grpc_url = os.environ.get("VECDB_TEST_QDRANT_URL", "http://localhost:6336")
    test_port = test_grpc_url.rsplit(":", 1)[-1]
    config_path = os.path.join(TEMP_HOME, ".config/vecdb/config.toml")
    with open(config_path, "r") as f:
        cfg = f.read()
    if "[profiles.default]" not in cfg:
        print("FAILURE: generated config has no [profiles.default] to pin")
        sys.exit(1)
    cfg = cfg.replace("[profiles.default]",
                      f'[profiles.default]\nqdrant_url = "{test_grpc_url}"', 1)
    with open(config_path, "w") as f:
        f.write(cfg)

    # Prove it. `config show` reports the endpoint actually resolved, so this
    # catches a pin that did not take for ANY reason, not just a stale regex.
    shown = run_vecdb(["config", "show"]).stdout
    if test_port not in shown:
        print("FAILURE: config does not resolve to test Qdrant — refusing to write.")
        print(f"  expected port {test_port} in resolved config; got:\n{shown[:600]}")
        sys.exit(1)

    # 2. Config Set-Quantization
    print("[2] Setting quantization to 'binary'...")
    collection = "test_quant_coll"
    run_vecdb(["config", "set-quantization", collection, "binary"])

    # Verify config file string check
    config_path = os.path.join(TEMP_HOME, ".config/vecdb/config.toml")
    with open(config_path, "r") as f:
        content = f.read()
        if 'quantization = "binary"' not in content:
            print(f"FAILURE: Config file content mismatch. Content:\n{content}")
            sys.exit(1)
    print("    Config verification passed.")

    # 3. Ingest (Should create collection with binary quantization)
    print("[3] Ingesting dummy data...")
    dummy_file = os.path.join(TEMP_HOME, "dummy.txt")
    with open(dummy_file, "w") as f:
        f.write("Hello world. This is a vector database test." * 50)
    
    run_vecdb(["ingest", dummy_file, "--collection", collection])

    # 4. Verify Qdrant State
    print("[4] Verifying Qdrant state...")
    q_config = check_collection_quantization(collection)
    if not q_config or q_config.get("binary") is None:
        print(f"FAILURE: Qdrant quantization config mismatch. Expected binary, got: {json.dumps(q_config)}")
        sys.exit(1)
    print("    Qdrant binary quantization verified.")

    # 5. Optimize (Update to Scalar)
    print("[5] Updating config to 'scalar' and optimizing...")
    # First update config override
    run_vecdb(["config", "set-quantization", collection, "scalar"])
    
    # Run optimize command
    run_vecdb(["optimize", collection])
    
    # Wait for optimize to apply
    time.sleep(2) 
    
    q_config = check_collection_quantization(collection)
    if not q_config or q_config.get("scalar") is None:
        print(f"FAILURE: Qdrant quantization config mismatch after optimize. Expected scalar, got: {json.dumps(q_config)}")
        sys.exit(1)
    
    print("    Qdrant scalar quantization verified.")

    print("=== SUCCESS ===")
    cleanup()

if __name__ == "__main__":
    main()
