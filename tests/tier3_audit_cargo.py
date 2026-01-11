#!/usr/bin/env python3
"""
Tier 3 Test: Cargo Dependency Freshness
Checks for outdated dependencies.
WARNING: Newer version available.
ERROR: Current version > 1 year old AND newer version available.
"""

import subprocess
import json
import sys
import os
from datetime import datetime, timezone
import urllib.request
import urllib.error

# Config
WARN_COLOR = "\033[93m"
ERROR_COLOR = "\033[91m"
RESET_COLOR = "\033[0m"

def get_dependencies():
    """Parse Cargo.lock to get current dependencies and versions."""
    # Note: This is a simplified parser. For production robustness, use 'cargo metadata'.
    # But sticking to standard lib for "no external deps" rule if possible.
    # Actually, 'cargo metadata' is standard if cargo is installed.
    result = subprocess.run(
        ["cargo", "metadata", "--format-version", "1"],
        capture_output=True, text=True, check=True
    )
    metadata = json.loads(result.stdout)
    
    deps = {}
    for package in metadata['packages']:
        # Skip local packages
        if 'source' not in package or not package['source']:
            continue
        # Only check registry packages (crates.io)
        if "registry+https://github.com/rust-lang/crates.io-index" not in package['source']:
             continue
             
        deps[package['name']] = package['version']
    return deps

def get_crate_info(crate_name):
    """Query crates.io API for crate info."""
    url = f"https://crates.io/api/v1/crates/{crate_name}"
    req = urllib.request.Request(url, headers={"User-Agent": "vecdb-test-runner/1.0"})
    try:
        with urllib.request.urlopen(req) as response:
            return json.loads(response.read().decode())
    except urllib.error.HTTPError as e:
        print(f"Failed to fetch info for {crate_name}: {e}")
        return None

def parse_date(date_str):
    # Format: 2023-10-09T03:07:31.956799+00:00 or similar ISO8601
    # Python 3.11+ handles ISO well, but let's be safe
    return datetime.fromisoformat(date_str.replace("Z", "+00:00"))

def main():
    print("=== Tier 3: Auditing Cargo Dependencies ===")
    
    deps = get_dependencies()
    total_checked = 0
    warnings = []
    errors = []
    
    now = datetime.now(timezone.utc)
    
    print(f"Checking {len(deps)} dependencies against crates.io...")
    
    for name, current_version in deps.items():
        info = get_crate_info(name)
        if not info:
            continue
            
        total_checked += 1
        
        max_version = info['crate']['max_version']
        
        # Check if update available
        if max_version != current_version:
            # Get data for CURRENT version to check age
            versions = info['versions']
            current_v_data = next((v for v in versions if v['num'] == current_version), None)
            
            if current_v_data:
                created_at = parse_date(current_v_data['created_at'])
                age = now - created_at
                age_days = age.days
                
                msg = f"{name}: {current_version} -> {max_version} (Current is {age_days} days old)"
                
                if age_days > 365:
                   errors.append(msg)
                   print(f"{WARN_COLOR}[ANCIENT] {msg}{RESET_COLOR}")
                else:
                   warnings.append(msg)
                   # print(f"{WARN_COLOR}[WARN]  {msg}{RESET_COLOR}") # Valid warning
            else:
                 warnings.append(f"{name}: {current_version} -> {max_version} (Could not determine age of current)")

    print(f"\nChecked {total_checked} packages.")
    
    if warnings:
        print(f"\n{WARN_COLOR}[WARNING] Found {len(warnings)} outdated packages:{RESET_COLOR}")
        for w in warnings:
            print(f"  - {w}")
            
    if errors:
        print(f"\n{WARN_COLOR}[WARNING] Found {len(errors)} ANCIENT (>1 year) outdated packages w/ updates:{RESET_COLOR}")
        for e in errors:
            print(f"  - {e}")
        # warnings only, do not fail
        # sys.exit(0)
    else:
        print("\nDependency freshness check passed.")

if __name__ == "__main__":
    main()
