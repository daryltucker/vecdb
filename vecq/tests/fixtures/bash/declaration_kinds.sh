#!/usr/bin/env bash
# Every Bash declaration kind reachable by the extractor.
COVERAGE_VAR=1

function explicit_form() {
    echo "$COVERAGE_VAR"
}

terse_form() {
    echo "terse"
}
