#!/usr/bin/env python3
"""
Tier 3 Test: Untracked File Audit
Identifies files in tracked directories that are not themselves tracked or ignored.
WARNINGS only.
"""

import subprocess
import os
import sys

def get_tracked_dirs():
    """Get list of directories tracked by git."""
    result = subprocess.run(
        ["git", "ls-files", "--directory"], 
        capture_output=True, text=True, check=True
    )
    return [d.strip().rstrip('/') for d in result.stdout.splitlines() if d.strip()]

def get_tracked_files():
    """Get set of all tracked files."""
    result = subprocess.run(
        ["git", "ls-files"], 
        capture_output=True, text=True, check=True
    )
    return set(f.strip() for f in result.stdout.splitlines() if f.strip())

def is_ignored(path):
    """Check if a path is ignored by git."""
    result = subprocess.run(
        ["git", "check-ignore", "-q", path],
        capture_output=True
    )
    return result.returncode == 0

def main():
    tracked_dirs = get_tracked_dirs()
    tracked_files = get_tracked_files()
    
    warnings = []
    
    print("=== Tier 3: Auditing Untracked Files ===")
    
    for d in tracked_dirs:
        if d == ".": continue
        
        for root, _, files in os.walk(d):
            # Skip .git directories
            if ".git" in root.split(os.sep):
                continue
                
            for file in files:
                file_path = os.path.join(root, file)
                
                # Check if tracked
                if file_path in tracked_files:
                    continue
                
                # Check if ignored
                if is_ignored(file_path):
                    continue
                
                # If neither, it's a loose file in a tracked dir
                warnings.append(file_path)

    if warnings:
        print("\n[WARNING] Found untracked files in tracked directories:")
        for w in warnings:
            print(f"  - {w}")
        print(f"\nTotal Warnings: {len(warnings)}")
    else:
        print("No untracked files found in tracked directories.")

if __name__ == "__main__":
    main()
