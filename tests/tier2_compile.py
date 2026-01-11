#!/usr/bin/env python3
import subprocess
import sys

def run_command(cmd, cwd=None):
    """Runs a shell command."""
    print(f"Running: {cmd}")
    result = subprocess.run(cmd, shell=True, cwd=cwd, capture_output=True, text=True)
    if result.returncode != 0:
        print(f"FAILED: {cmd}")
        print("STDOUT:", result.stdout)
        print("STDERR:", result.stderr)
        return False
    return True

def test_workspace_compilation():
    """Verifies that the entire workspace validates."""
    print("--- Checking Workspace Compilation ---")
    # cargo check --workspace is faster for CI/Tier 2 than full build
    if not run_command("cargo check --workspace"):
        sys.exit(1)
    
    # Also explicitly check binaries to be safe about entry points
    print("--- Verifying Binaries ---")
    
    # Check vecdb-cli (binary name: vecdb)
    if not run_command("cargo check -p vecdb-cli --bin vecdb"):
        print("FAILED: vecdb-cli check")
        sys.exit(1)

    # Check vecdb-server (binary name: vecdb-server)
    if not run_command("cargo check -p vecdb-server --bin vecdb-server"):
        print("FAILED: vecdb-server check")
        sys.exit(1)

    # Check vecq (binary name: vecq)
    if not run_command("cargo check -p vecq --bin vecq"):
        print("FAILED: vecq check")
        sys.exit(1) 

    print("SUCCESS: Workspace consistency check passed.")

if __name__ == "__main__":
    test_workspace_compilation()
