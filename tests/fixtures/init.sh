#!/bin/bash
# tests/fixtures/init.sh
# Purpose: Initialize external test fixtures (download/clone) without bloating the repo.
# Usage: ./tests/fixtures/init.sh

set -e

FIXTURE_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
EXTERNAL_DIR="$FIXTURE_DIR/external"
mkdir -p "$EXTERNAL_DIR"

echo "=== Initializing Test Fixtures ==="
echo "Target: $EXTERNAL_DIR"

# helper: fetch_git <url> <dest_name> [branch]
fetch_git() {
    local url=$1
    local name=$2
    local branch=$3
    local dest="$EXTERNAL_DIR/$name"

    if [ -d "$dest" ]; then
        echo "[SKIP] $name already exists."
    else
        echo "[FETCH] Cloning $name..."
        git clone --depth 1 "$url" "$dest" ${branch:+-b $branch}
        rm -rf "$dest/.git"
    fi
}

# helper: fetch_file <url> <filename>
fetch_file() {
    local url=$1
    local name=$2
    local dest="$EXTERNAL_DIR/$name"

    if [ -f "$dest" ]; then
        echo "[SKIP] $name already exists."
    else
        echo "[FETCH] Downloading $name..."
        curl -L -o "$dest" "$url"
    fi
}

# helper: fetch_tarball <url> <name>
fetch_tarball() {
    local url=$1
    local name=$2
    local dest="$EXTERNAL_DIR/$name"

    if [ -d "$dest" ]; then
        echo "[SKIP] $name already exists."
    else
        echo "[FETCH] Downloading/Extracting $name..."
        local tarname="$(basename "$url")"
        curl -L -R -O "$url"
        tar zxf "$tarname" -C "$EXTERNAL_DIR"
        rm "$tarname"
    fi
}

# --- DEFINITIONS ---

# 1. Linux Kernel (Subset for Stress Testing)
# UNCOMMENT to enable massive stress testing (~1.5GB)
# fetch_git "https://github.com/torvalds/linux.git" "linux-kernel"

# 2. CUDA Samples (for .cu parser testing)
fetch_git "https://github.com/NVIDIA/cuda-samples.git" "cuda-samples" "master"


# 3. Large Text Corpus (Project Gutenberg)
fetch_file "https://www.gutenberg.org/files/1342/1342-0.txt" "pride-and-prejudice.txt"

# 4. Lua 5.4.6 (Source Code)
# Used for C parser stress testing without git overhead
fetch_tarball "https://www.lua.org/ftp/lua-5.4.6.tar.gz" "lua-5.4.6"

echo "=== Fixtures Ready ==="
echo "You can now run Tier 3 tests that depend on 'external/'."
