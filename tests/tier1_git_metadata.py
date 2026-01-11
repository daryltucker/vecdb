
import subprocess
import os
import shutil
import time

def run_test():
    TEST_DIR = "tests/fixtures/git_test_repo"
    
    # Clean up
    if os.path.exists(TEST_DIR):
        shutil.rmtree(TEST_DIR)
    os.makedirs(TEST_DIR)

    try:
        print(f"Initializing git repo in {TEST_DIR}...")
        subprocess.run(["git", "init"], cwd=TEST_DIR, check=True, capture_output=True)
        
        # Configure git user for commit
        subprocess.run(["git", "config", "user.email", "test@example.com"], cwd=TEST_DIR, check=True)
        subprocess.run(["git", "config", "user.name", "Test User"], cwd=TEST_DIR, check=True)

        with open(os.path.join(TEST_DIR, "test_doc.md"), "w") as f:
            f.write("# Hello\nThis is a test document.")
        
        subprocess.run(["git", "add", "."], cwd=TEST_DIR, check=True)
        subprocess.run(["git", "commit", "-m", "Initial commit"], cwd=TEST_DIR, check=True)
        
        # Get SHA
        sha_proc = subprocess.run(["git", "rev-parse", "HEAD"], cwd=TEST_DIR, check=True, capture_output=True, text=True)
        expected_sha = sha_proc.stdout.strip()
        print(f"Expected SHA: {expected_sha}")

        # Run Ingest
        print("Running verify ingestion...")
        # Note: We use cargo run to ensure we are testing the current code
        # We need --allow-local-fs equivalent logic or just CLI direct usage
        # vecdb-cli ingest does use local fs
        
        cmd = [
            "cargo", "run", "--quiet", "--bin", "vecdb", "--", 
            "ingest", TEST_DIR, 
            "--collection", "git_test"
        ]
        
        result = subprocess.run(cmd, capture_output=True, text=True)
        
        if result.returncode != 0:
            print("CLI Failed:")
            print(result.stderr)
            exit(1)

        print("CLI Output (Stderr):")
        print(result.stderr)

        if f"Injecting commit_sha: {expected_sha}" in result.stderr:
            print("SUCCESS: Log confirms injection.")
        else:
            print("FAILURE: Log missing injection confirmation.")
            exit(1)

    except Exception as e:
        print(f"Test Failed: {e}")
        exit(1)
    finally:
        if os.path.exists(TEST_DIR):
            shutil.rmtree(TEST_DIR)

if __name__ == "__main__":
    run_test()
