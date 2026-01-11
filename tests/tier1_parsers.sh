#!/bin/bash
set -e

# Tier 1 Parsers Test
# Verifies that each parser works on real files and returns correct metadata
# Goal: Ensure we never see "file_type": "Unknown" again for supported files

PROJECT_ROOT=$(dirname "$0")/..
cd "$PROJECT_ROOT"

VECQ="target/debug/vecq"
if [ ! -f "$VECQ" ]; then
    echo "Building vecq..."
    cargo build --bin vecq --quiet
fi

echo "=== Testing Parsers ==="

# List of extensions and expected types
declare -A EXT_TYPES=( 
    ["md"]="Markdown"
    ["rs"]="Rust" 
    ["py"]="Python"
    ["c"]="C"
    ["cpp"]="C++"
    ["cu"]="CUDA"
    ["go"]="Go"
    ["sh"]="Bash"
    ["txt"]="Text"
)

FAILED=0

for ext in "${!EXT_TYPES[@]}"; do
    expected_type="${EXT_TYPES[$ext]}"
    file="tests/fixtures/external/tiny_tier1/sample.$ext"
    
    echo -n "Testing $expected_type ($ext)... "
    
    if [ ! -f "$file" ]; then
        echo "❌ Fixture missing: $file"
        FAILED=1
        continue
    fi
    
    # Run vecq conversion
    output=$($VECQ "$file" --convert --pretty 2>&1)
    exit_code=$?
    
    if [ $exit_code -ne 0 ]; then
        echo "❌ Failed (Exit Code: $exit_code)"
        echo "$output"
        FAILED=1
        continue
    fi
    
    # Extract file_type from JSON output (robustly)
    if command -v python3 &> /dev/null; then
        detected_type=$(echo "$output" | python3 -c "import sys, json; print(json.load(sys.stdin)['metadata']['file_type'])")
    else
        # Fallback: look for file_type inside metadata block
        detected_type=$(echo "$output" | grep -A 20 '"metadata":' | grep '"file_type":' | head -1 | cut -d'"' -f4)
    fi
    
    if [ "$detected_type" == "$expected_type" ]; then
        echo "✅ OK"
    else
        echo "❌ Failed (Got: '$detected_type', Expected: '$expected_type')"
        FAILED=1
    fi
done

if [ $FAILED -eq 0 ]; then
    echo "=== ALL PARSERS VERIFIED ==="
    exit 0
else
    echo "=== PARSER VERIFICATION FAILED ==="
    exit 1
fi
