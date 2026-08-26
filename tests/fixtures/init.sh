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

# 5. tiny_tier1 — one small sample per supported extension.
#
# Generated locally rather than fetched: Tier 1 gates the whole run, so it must
# not depend on the network being up. tests/tier1_parsers.sh asserts each file
# detects as its expected FileType (never Unknown), and tests/tier1_mcp.py uses
# the directory as a small ingestion corpus.
#
# Keep every sample non-trivial enough to parse into at least one symbol — an
# empty file detects correctly while proving nothing about the parser.
make_tiny_tier1() {
    local dest="$EXTERNAL_DIR/tiny_tier1"
    if [ -d "$dest" ] && [ -f "$dest/sample.rs" ]; then
        echo "[SKIP] tiny_tier1 already exists."
        return
    fi
    echo "[GEN]   tiny_tier1 (local, no network)"
    mkdir -p "$dest"

    cat > "$dest/sample.md" <<'EOF'
# Sample Document

A heading and a paragraph so the Markdown parser has structure to find.

## Retrieval

This fixture exists to prove the parser reports Markdown, not Unknown.
EOF

    cat > "$dest/sample.rs" <<'EOF'
/// Compute the similarity between two vectors.
pub fn cosine_similarity(a: &[f32], b: &[f32]) -> f32 {
    let dot: f32 = a.iter().zip(b).map(|(x, y)| x * y).sum();
    let na: f32 = a.iter().map(|x| x * x).sum::<f32>().sqrt();
    let nb: f32 = b.iter().map(|x| x * x).sum::<f32>().sqrt();
    if na == 0.0 || nb == 0.0 { 0.0 } else { dot / (na * nb) }
}

pub struct Chunk {
    pub text: String,
    pub offset: usize,
}
EOF

    cat > "$dest/sample.py" <<'EOF'
"""Sample module for parser detection."""


def chunk_text(text: str, size: int = 512) -> list[str]:
    """Split text into fixed-size chunks."""
    return [text[i:i + size] for i in range(0, len(text), size)]


class Embedder:
    def __init__(self, dimension: int) -> None:
        self.dimension = dimension
EOF

    cat > "$dest/sample.c" <<'EOF'
#include <stddef.h>

/* Sum a vector of floats. */
float vector_sum(const float *values, size_t count) {
    float total = 0.0f;
    for (size_t i = 0; i < count; i++) {
        total += values[i];
    }
    return total;
}
EOF

    cat > "$dest/sample.cpp" <<'EOF'
#include <vector>
#include <numeric>

namespace vecdb {

// Mean of a vector, or zero when empty.
double mean(const std::vector<double> &values) {
    if (values.empty()) {
        return 0.0;
    }
    return std::accumulate(values.begin(), values.end(), 0.0) / values.size();
}

}  // namespace vecdb
EOF

    cat > "$dest/sample.cu" <<'EOF'
// Elementwise vector addition on the device.
__global__ void vector_add(const float *a, const float *b, float *out, int n) {
    int i = blockIdx.x * blockDim.x + threadIdx.x;
    if (i < n) {
        out[i] = a[i] + b[i];
    }
}

extern "C" void launch_vector_add(const float *a, const float *b, float *out, int n) {
    vector_add<<<(n + 255) / 256, 256>>>(a, b, out, n);
}
EOF

    cat > "$dest/sample.go" <<'EOF'
package sample

// Normalize scales a vector to unit length.
func Normalize(v []float64) []float64 {
	var norm float64
	for _, x := range v {
		norm += x * x
	}
	if norm == 0 {
		return v
	}
	out := make([]float64, len(v))
	for i, x := range v {
		out[i] = x / norm
	}
	return out
}
EOF

    cat > "$dest/sample.sh" <<'EOF'
#!/bin/bash
# Sample script for parser detection.
set -euo pipefail

collection="${1:-test_sample}"

report_collection() {
    echo "collection: $collection"
}

report_collection
EOF

    cat > "$dest/sample.txt" <<'EOF'
Plain text fixture.

Used to confirm the Text parser is selected for .txt files and that plain
prose still chunks into something searchable.
EOF

    chmod +x "$dest/sample.sh"
}

make_tiny_tier1

echo "=== Fixtures Ready ==="
echo "You can now run Tier 3 tests that depend on 'external/'."
