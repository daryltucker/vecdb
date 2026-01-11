#!/bin/bash
set -e

SCRIPT_PATH="../../scripts/test_runner.sh"

echo "Verifying test_runner.sh..."

# 1. Syntax Check
bash -n "$SCRIPT_PATH" || { echo "Syntax Check Failed"; exit 1; }
echo "Syntax Check: OK"

# 2. Help output check
OUTPUT=$("$SCRIPT_PATH" --help)
if [[ "$OUTPUT" == *"Usage:"* ]]; then
    echo "Help Output: OK"
else
    echo "Help Output Failed"
    exit 1
fi

echo "Infrastructure Verification Passed ✅"
