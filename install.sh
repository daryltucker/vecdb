#!/bin/bash
# install.sh - Install vecdb binaries to ~/.cargo/bin
# Usage: ./install.sh [--verbose]
# 
# By default, uses --quiet for clean output.
# Use --verbose to see full compilation output.

# THIS SCRIPT IS A **HELPER** FOR DEVELOPERS
# THIS IS NOT THE INTENDED INSTALLATION METHOD FOR MOST USERS
# OUR USERS MUST BE ABLE TO `cargo install` AND NOT RELY ON THIS SCRIPT OR OTHER WRAPPERS

set -e

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"

bash ${SCRIPT_DIR}/scripts/prune_target.sh

# Parse args
QUIET_FLAG="--quiet"
if [[ "$1" == "--verbose" ]]; then
    QUIET_FLAG=""
fi

echo "=== Installing vecdb binaries ==="
echo "Target: ~/.cargo/bin"
echo ""

cd "$SCRIPT_DIR"

# Install vecq
echo "[1/4] Installing vecq (jq for source code)..."
cargo install --path vecq --force $QUIET_FLAG

# Install CLI (with CUDA)
echo "[2/4] Installing vecdb (CLI with CUDA)..."
# 2. Build binaries (Standard Cargo Build)
# 'ort' crate with 'download-binaries' feature will handle libs at runtime.
echo "Building Release Binaries..."

cargo install --path vecdb-cli --force $QUIET_FLAG
cargo install --path vecdb-server --force $QUIET_FLAG
cargo install --path docsize --force $QUIET_FLAG
cargo install --path vecdb-asm --force $QUIET_FLAG



echo ""
echo "=== Installation Complete ==="
echo "Installed to ~/.cargo/bin"
echo "  - vecdb"
echo "  - vecdb-server"
echo "  - docsize"
echo "  - vecdb-asm"
echo "  - vecq"

echo ""
echo "=== Installation Complete ==="
echo "Installed:"
echo "  - vecq         (jq for source code)"
echo "  - vecdb        (CLI tool, CUDA enabled)"
echo "  - vecdb-server (MCP server, CUDA enabled)"
echo "  - docsize      (LLM wrapper)"
echo "  - vecdb-asm    (Knowledge Assembler)"
echo ""
echo "Verify with: vecq --help && vecdb --help && docsize --help && vecdb-asm --help"
echo ""

# Autocomplete setup
SETUP_AUTOCOMPLETE=false
if [[ "$SHELL" == */bash ]]; then
    SHELL_NAME="bash"
    RC_FILE="$HOME/.bashrc"
    SETUP_AUTOCOMPLETE=true
elif [[ "$SHELL" == */zsh ]]; then
    SHELL_NAME="zsh"
    RC_FILE="$HOME/.zshrc"
    SETUP_AUTOCOMPLETE=true
fi

if [ "$SETUP_AUTOCOMPLETE" = true ]; then
    echo "=== Autocomplete Setup ==="
    COMP_DIR="$HOME/.local/share/vecdb/completions"
    mkdir -p "$COMP_DIR"

    ~/.cargo/bin/vecdb completions "$SHELL_NAME" > "$COMP_DIR/vecdb"
    ~/.cargo/bin/vecq completions "$SHELL_NAME" > "$COMP_DIR/vecq"

    SOURCE_CMD="source $COMP_DIR/vecdb && source $COMP_DIR/vecq"
    
    if ! grep -q "$COMP_DIR/vecdb" "$RC_FILE"; then
        echo "Detected $SHELL_NAME. To enable autocomplete, add this to your $RC_FILE:"
        echo ""
        echo "  # vecdb completions"
        echo "  [ -f \"$COMP_DIR/vecdb\" ] && . \"$COMP_DIR/vecdb\""
        echo "  [ -f \"$COMP_DIR/vecq\" ] && . \"$COMP_DIR/vecq\""
        echo ""
        read -p "Would you like me to add this to your $RC_FILE now? (y/N) " -n 1 -r
        echo ""
        if [[ $REPLY =~ ^[Yy]$ ]]; then
            echo "" >> "$RC_FILE"
            echo "# vecdb completions" >> "$RC_FILE"
            echo "[ -f \"$COMP_DIR/vecdb\" ] && . \"$COMP_DIR/vecdb\"" >> "$RC_FILE"
            echo "[ -f \"$COMP_DIR/vecq\" ] && . \"$COMP_DIR/vecq\"" >> "$RC_FILE"
            echo "Added to $RC_FILE. Please restart your shell or run: $SOURCE_CMD"
        fi
    else
        echo "Autocomplete already configured in $RC_FILE."
    fi
fi

echo "Tip: Use './install.sh --verbose' to see compilation output"
