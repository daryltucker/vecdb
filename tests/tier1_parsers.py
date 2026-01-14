#!/usr/bin/env python3
import os
import shutil
import subprocess
import sys
import time

# --- Configuration ---
BINARY_PATH = "./target/debug/vecdb"
TEST_DIR = "tier1_parsers_test_env"

def run_command(cmd, check=True):
    """Runs a shell command."""
    print(f"Running: {cmd}")
    result = subprocess.run(cmd, shell=True, capture_output=True, text=True)
    if check and result.returncode != 0:
        print(f"Error running command: {cmd}")
        print("STDOUT:", result.stdout)
        print("STDERR:", result.stderr)
        sys.exit(1)
    return result

def setup():
    """Sets up the test environment."""
    if os.path.exists(TEST_DIR):
        shutil.rmtree(TEST_DIR)
    os.makedirs(TEST_DIR)
    
    # Create test.json
    with open(f"{TEST_DIR}/test.json", "w") as f:
        f.write('{"config": {"server": {"port": 8080, "host": "0.0.0.0"}}, "users": [{"name": "alice"}, {"name": "bob"}]}')

    # Create test.yaml
    with open(f"{TEST_DIR}/test.yaml", "w") as f:
        f.write("""
app:
  database:
    host: localhost
    port: 5432
  features:
    - logging
    - metrics
""")

def test_ingestion():
    """Ingests the test directory."""
    print("--- Ingesting ---")
    run_command(f"{BINARY_PATH} ingest {TEST_DIR}")

def test_search_json():
    """Verifies JSON flattening."""
    print("--- Verifying JSON ---")
    # Search for flattened key
    # Search for flattened key in the JSON object
    result = run_command(f"{BINARY_PATH} search 'config.server.port'")
    
    # With recursive AST/TreeSitter, we might get the full JSON object for the block
    # Expected: {"server":{"host":"0.0.0.0","port":8080}} or similar structure
    if '"port":8080' not in result.stdout.replace(" ", ""):
        print("FAILED: JSON content not found.")
        print("Expected '\"port\":8080' (ignoring whitespace) in output.")
        print("Actual Output:", result.stdout)
        sys.exit(1)
    
    # Search for array content
    result = run_command(f"{BINARY_PATH} search 'users[0].name'")
    # Should find alice
    if 'alice' not in result.stdout:
        print("FAILED: JSON array content (alice) not found.")
        print(result.stdout)
        sys.exit(1)
    print("JSON Verification Passed")

def test_search_yaml():
    """Verifies YAML flattening."""
    print("--- Verifying YAML ---")
    # Search for flattened key
    result = run_command(f"{BINARY_PATH} search 'app.database.host'")
    
    # TreeSitter/Recursive parser preserves structure, it does NOT flatten keys like the old linear parser.
    # So we should expect to find the content block containing the key.
    
    print("YAML Search Result Preview: \n" + result.stdout[:200])

    # Check that the content was found
    if "host: localhost" in result.stdout:
        print("YAML Content found.")
    else:
        print("FAILED: YAML content 'host: localhost' not found.")
        print("Full Output:\n", result.stdout)
        sys.exit(1)

    print("YAML Verification Passed")

def cleanup():
    """Cleans up."""
    # if os.path.exists(TEST_DIR):
    #     shutil.rmtree(TEST_DIR)
    pass

def main():
    try:
        setup()
        test_ingestion()
        test_search_json()
        test_search_yaml()
        print("\nSUCCESS: All Parser Tier 1 tests passed!")
    except Exception as e:
        print(f"\nAn error occurred: {e}")
        sys.exit(1)
    finally:
        cleanup()

if __name__ == "__main__":
    main()
