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
    """Directories containing at least one tracked file.

    Derived from `git ls-files`, NOT from `git ls-files --directory`. The
    latter returns *file* paths — `--directory` only collapses output when
    combined with `--others`. This function used to return those file paths,
    which `os.walk()` then yielded nothing for, so the audit scanned zero
    directories and reported "no untracked files" unconditionally. It was a
    permanently-passing test inside the release gate.
    """
    result = subprocess.run(
        ["git", "ls-files"],
        capture_output=True, text=True, check=True
    )
    dirs = set()
    for f in result.stdout.splitlines():
        f = f.strip()
        if f:
            dirs.add(os.path.dirname(f) or ".")
    return sorted(dirs)

def get_untracked_files():
    """Files git considers untracked and not ignored.

    `--exclude-standard` applies .gitignore/.git/info/exclude exactly as git
    itself does, which is both faster and more faithful than one
    `git check-ignore` subprocess per file.
    """
    result = subprocess.run(
        ["git", "ls-files", "--others", "--exclude-standard"],
        capture_output=True, text=True, check=True
    )
    return [f.strip() for f in result.stdout.splitlines() if f.strip()]

def main():
    tracked_dirs = set(get_tracked_dirs())

    print("=== Tier 3: Auditing Untracked Files ===")

    # A loose file is one git does not track, is not ignored, and which sits in
    # a directory that already holds tracked files — i.e. somewhere that looks
    # like it belongs to the project.
    warnings = [
        path for path in get_untracked_files()
        if (os.path.dirname(path) or ".") in tracked_dirs
    ]

    if warnings:
        print("\n[WARNING] Found untracked files in tracked directories:")
        for w in warnings:
            print(f"  - {w}")
        print(f"\nTotal Warnings: {len(warnings)}")
    else:
        print("No untracked files found in tracked directories.")

if __name__ == "__main__":
    main()
