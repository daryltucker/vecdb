
import subprocess
import os
import shutil
import sys

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
from paths import bin_path

# The binary T1.1 built, not `cargo run`.
#
# `cargo run --bin vecdb` REBUILDS vecdb with DEFAULT features at the same path
# T1.1 wrote the cuda-dynamic build to, so this test silently replaced the
# binary every later tier depends on — one of five such points in the manifest.
# The old comment justified it as "to ensure we are testing the current code",
# but run_all.sh already built the current code, with the right features.
VECDB = bin_path("vecdb")


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
        cmd = [VECDB, "ingest", TEST_DIR, "--collection", "test_git"]

        # VECDB_CONFIG pinned explicitly. Inherited, this resolves whatever
        # config the ambient environment names — the operator's real one when
        # the file is run directly rather than through run_all.sh.
        env = {
            **os.environ,
            "VECDB_CONFIG": os.path.join(
                os.path.dirname(os.path.abspath(__file__)), "fixtures", "config.toml"
            ),
        }
        result = subprocess.run(cmd, capture_output=True, text=True, env=env)
        
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
