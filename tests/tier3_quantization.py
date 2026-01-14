#!/usr/bin/env python3
import os
import shutil
import subprocess
import sys
import time
import json
import urllib.request

# Setup
BINARY = "./target/debug/vecdb"
TEMP_HOME = "./out/test_quantization_home"
QDRANT_URL = "http://localhost:6333" # Qdrant HTTP API

def cleanup():
    if os.path.exists(TEMP_HOME):
        shutil.rmtree(TEMP_HOME)

def run_vecdb(args, check=True):
    env = os.environ.copy()
    env["HOME"] = os.path.abspath(TEMP_HOME)
    env["XDG_CONFIG_HOME"] = os.path.join(env["HOME"], ".config")
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
    os.makedirs(TEMP_HOME, exist_ok=True)

    # 1. Init
    print("[1] Initializing...")
    run_vecdb(["init"])

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
