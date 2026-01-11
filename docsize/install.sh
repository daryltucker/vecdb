#!/bin/bash
# docsize/install.sh - Install docsize binary to ~/.cargo/bin
# Usage: ./install.sh [--verbose]

set -e

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"

QUIET_FLAG="--quiet"
if [[ "$1" == "--verbose" ]]; then
    QUIET_FLAG=""
fi

echo "=== Installing docsize ==="
echo "Target: ~/.cargo/bin"
echo ""

cd "$SCRIPT_DIR"

cargo install --path . --force $QUIET_FLAG

echo ""
echo "=== Installation Complete ==="
echo "Verify with: docsize --help"
echo ""
