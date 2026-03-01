
import subprocess
import os
import shutil
import time

def run_test():
    TEST_DIR = "tests/fixtures/inc_test_repo"
    COLLECTION = "inc_test"
    
    # Clean up
    if os.path.exists(TEST_DIR):
        shutil.rmtree(TEST_DIR)
    os.makedirs(TEST_DIR)
    
    # Ensure fresh collection for Tier 1 test
    subprocess.run(["cargo", "run", "--quiet", "--bin", "vecdb", "--", "delete", COLLECTION, "--yes"], capture_output=True)

    try:
        print(f"Initializing repo in {TEST_DIR}...")
        
        # 1. Create file 1
        with open(os.path.join(TEST_DIR, "file1.txt"), "w") as f:
            f.write("Content A")
            
        # 2. Create file 2 (ignored by default ignore - BUT SHOULD BE SCANNED NOW)
        with open(os.path.join(TEST_DIR, ".gitignore"), "w") as f:
            f.write("ignored.txt\n")
        with open(os.path.join(TEST_DIR, "ignored.txt"), "w") as f:
            f.write("Should NOT be ignored anymore")

        # 3. Create file 3 (ignored by vectorignore)
        with open(os.path.join(TEST_DIR, ".vectorignore"), "w") as f:
            f.write("vector_ignored.txt\n")
        with open(os.path.join(TEST_DIR, "vector_ignored.txt"), "w") as f:
            f.write("Should be ignored by separate file")

        # Run Ingest 1
        print("--- Run 1 ---")
        cmd = [
            "cargo", "run", "--quiet", "--bin", "vecdb", "--", 
            "ingest", TEST_DIR, 
            "--collection", COLLECTION
        ]
        res1 = subprocess.run(cmd, capture_output=True, text=True)
        if res1.returncode != 0: raise Exception(res1.stderr)
        
        if "Processed 2" not in res1.stderr: 
             # file1.txt, ignored.txt processed.
             # .gitignore and .vectorignore are skipped by the 'ignore' walker when active.
             # vector_ignored.txt skipped by .vectorignore.
             print("FAILURE: Run 1 should process exactly 2 files (file1, ignored.txt).")
             print(f"Actual Output: {res1.stderr}")
             exit(1)
             
        # Run Ingest 2 (No changes)
        print("--- Run 2 (No Change) ---")
        res2 = subprocess.run(cmd, capture_output=True, text=True)
        print(res2.stderr)
        if "Processed 0" not in res2.stderr:
             print("FAILURE: Run 2 should process 0 files.")
             exit(1)
             
        # Modify file1
        with open(os.path.join(TEST_DIR, "file1.txt"), "w") as f:
            f.write("Content B (Changed)")
            
        # Run Ingest 3 (Change)
        print("--- Run 3 (Modified) ---")
        res3 = subprocess.run(cmd, capture_output=True, text=True)
        print(res3.stderr)
        if "Processed 1" not in res3.stderr:
             print("FAILURE: Run 3 should process 1 file.")
             exit(1)

        print("SUCCESS: Incremental Ingestion Verified.")

    except Exception as e:
        print(f"Test Failed: {e}")
        exit(1)
    finally:
        if os.path.exists(TEST_DIR):
            shutil.rmtree(TEST_DIR)

if __name__ == "__main__":
    run_test()
