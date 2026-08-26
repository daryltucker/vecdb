#!/bin/bash
# ═══════════════════════════════════════════════════════════════════
# VECDB-MCP: COMPLETE TEST SUITE
# ═══════════════════════════════════════════════════════════════════
#
# ANTI-CHEAT MANDATE:
#   This script is the SINGLE SOURCE OF TRUTH for which tests must
#   pass before any release. Running a subset of these tests is a
#   release blocker. If you add a test file, you MUST add it here.
#
# AUTHORITY:
#   - Makefile `test` target delegates to this script.
#   - CLAUDE.md "Testing" summarises the tier semantics and the hard rules.
#   - THIS FILE is the full manifest. (Earlier revisions deferred to
#     docs/planning/TESTING.md and docs/planning/V1_AUDIT.md; neither exists in
#     this repo. Planning docs live in ~/Projects/docs/vecdb/planning/.)
#
# USAGE:
#   ./tests/run_all.sh           # Run everything
#   JOBS=4 ./tests/run_all.sh    # Override parallelism
#
# ═══════════════════════════════════════════════════════════════════

set -e
set -o pipefail

PROJECT_ROOT=$(dirname "$0")/..
cd "$PROJECT_ROOT"

# ═══════════════════════════════════════════════════════════════════
# PRODUCTION QDRANT LOCKOUT — NON-BYPASSABLE
#
# ALL TESTS MUST ALWAYS USE THE TESTING CONFIGURATION.
# NEVER HIT PRODUCTION QDRANT (ports 6333/6334).
#
# This is enforced here at the shell level BEFORE any test runs.
# The VECDB_CONFIG variable is FORCED to the test fixture regardless
# of what was set in the caller's environment.
# Any test that ignores VECDB_CONFIG or hardcodes production ports
# will be caught by tier0_qdrant_isolation.py (T0.0) and block the run.
# ═══════════════════════════════════════════════════════════════════
readonly TEST_CONFIG_REL="tests/fixtures/config.toml"

if [ ! -f "$TEST_CONFIG_REL" ]; then
    echo "FATAL: Test config not found at $TEST_CONFIG_REL" >&2
    echo "       Run from project root: ./tests/run_all.sh" >&2
    exit 1
fi

# ABSOLUTE, deliberately. A relative VECDB_CONFIG is resolved against each
# process's own CWD, and `cargo test -p <crate>` runs its test binaries with
# CWD at the CRATE root — so "tests/fixtures/config.toml" silently meant
# vecdb-cli/tests/fixtures/config.toml for every Rust tier-2 test. That file
# was a stale pre-three-layer config whose default collection was "docs", an
# unprefixed production name. The tests were not using the fixture they named.
readonly TEST_CONFIG="$(cd "$(dirname "$TEST_CONFIG_REL")" && pwd)/$(basename "$TEST_CONFIG_REL")"

# Force — overwrite any caller-provided VECDB_CONFIG.
export VECDB_CONFIG="$TEST_CONFIG"

# Also set the Rust-tier test URL so tier2_qdrant.rs tests hit test Qdrant.
export VECDB_TEST_QDRANT_URL="http://localhost:6336"
# HTTP REST port (for tests that query Qdrant REST API directly, e.g. tier3_quantization.py).
export VECDB_TEST_QDRANT_HTTP_URL="http://localhost:6335"

# ==========================================
# RESOURCE MANAGEMENT
# ==========================================
TOTAL_CORES=$(nproc)
HALF_CORES=$((TOTAL_CORES / 2))
if [ "$HALF_CORES" -lt 1 ]; then HALF_CORES=1; fi

JOBS=${JOBS:-$HALF_CORES}
export CARGO_BUILD_JOBS=$JOBS
export RAYON_NUM_THREADS=$JOBS

# ==========================================
# LOGGING
# ==========================================
mkdir -p logs
TIMESTAMP=$(date +%Y%m%d_%H%M%S)
LOGFILE="logs/test_${TIMESTAMP}.log"

log() {
    echo "$1" | tee -a "$LOGFILE"
}

run_test() {
    local label="$1"
    shift
    log ""
    log "  [$label] $*"
    "$@" 2>&1 | tee -a "$LOGFILE"
}

# ==========================================
# COUNTERS
# ==========================================
PASS=0
TOTAL=0
count() { TOTAL=$((TOTAL + 1)); }
passed() { PASS=$((PASS + 1)); }

log "═══════════════════════════════════════════════════════════"
log "  VECDB-MCP COMPLETE TEST SUITE"
log "  $(date)"
log "  Cores: $TOTAL_CORES | Build Jobs: $JOBS"
log "═══════════════════════════════════════════════════════════"

# ══════════════════════════════════════════
# TIER 0: INFRASTRUCTURE
# ══════════════════════════════════════════
log ""
log "━━━ TIER 0: Infrastructure ━━━"

# T0.0 MUST run first: proves the test suite cannot touch production Qdrant.
# This is a hard gate — if it fails, Qdrant-touching tests are blocked.
count; run_test "T0.0" python3 tests/tier0_qdrant_isolation.py; passed

# T0.05 runs immediately after the isolation gate and before any test that
# writes to Qdrant. It empties the test instance so a green run cannot be
# inherited from a previous one. Ordering matters: the isolation gate proves we
# are pointed at the test instance, and only then is a full wipe safe.
count; run_test "T0.05" python3 tests/tier0_reset_qdrant.py; passed

# T0.06 is a source scan, so it runs before anything is built or started: a
# hardcoded target path makes every later tier report "binary not found" for a
# perfectly good build, which is a confusing way to learn about it.
count; run_test "T0.06" python3 tests/tier0_target_dir_isolation.py; passed

count; run_test "T0.1" bash ./tests/fixtures/init.sh; passed
count; run_test "T0.2" python3 tests/tier1_qdrant.py; passed

log ""
log "  ┌──────────────────────────────────────────────┐"
log "  │ GATE: Tier 0 PASSED → Proceeding to Tier 1   │"
log "  └──────────────────────────────────────────────┘"

# ══════════════════════════════════════════
# TIER 1: UNIT / CONTRACT (Python + Bash)
# ══════════════════════════════════════════
log ""
log "━━━ TIER 1: Unit / Contract Tests ━━━"

count; run_test "T1.1" cargo build --bin vecdb --bin vecdb-server --bin vecq --quiet; passed
count; run_test "T1.2" python3 tests/tier1_parity.py; passed
count; run_test "T1.3" python3 tests/tier1_security.py; passed
count; run_test "T1.4" python3 tests/tier1_mcp.py; passed
count; run_test "T1.5" bash tests/tier1_parsers.sh; passed
count; run_test "T1.6" python3 tests/tier1_config.py; passed
count; run_test "T1.7" python3 tests/tier1_embedder_config.py; passed
count; run_test "T1.8" python3 tests/tier1_git_history.py; passed
count; run_test "T1.9" python3 tests/tier1_git_metadata.py; passed
count; run_test "T1.10" python3 tests/tier1_incremental.py; passed
count; run_test "T1.11" python3 tests/tier1_parsers.py; passed
count; run_test "T1.12" python3 tests/tier1_query.py; passed
count; run_test "T1.13" python3 tests/tier1_asm_deduplication.py; passed
count; run_test "T1.14" python3 tests/tier1_asm_sequencing.py; passed
count; run_test "T1.15" python3 tests/tier1_asm_state_diff.py; passed
count; run_test "T1.16" python3 tests/tier1_vecdbrc_warning.py; passed
count; run_test "T1.17" python3 tests/tier1_oversize_policy.py; passed
count; run_test "T1.18" python3 tests/tier1_dry_run.py; passed

# ══════════════════════════════════════════
# TIER 1.5: RUST UNIT TESTS (cargo test)
# ══════════════════════════════════════════
log ""
log "━━━ TIER 1.5: Rust Unit Tests ━━━"

count; run_test "T1.5.1" cargo test -p vecq -- --nocapture; passed
count; run_test "T1.5.2" cargo test -p vecdb-asm -- --nocapture; passed
count; run_test "T1.5.3" cargo test -p vecdb-common -- --nocapture; passed
count; run_test "T1.5.4" cargo test -p vecdb-core --lib -- --nocapture; passed

log ""
log "  ┌──────────────────────────────────────────────┐"
log "  │ GATE: Tier 1 PASSED → Proceeding to Tier 2   │"
log "  │   Proven: individual components work          │"
log "  └──────────────────────────────────────────────┘"

# ══════════════════════════════════════════
# TIER 2: INTEGRATION (Rust + Python)
# ══════════════════════════════════════════
log ""
log "━━━ TIER 2: Integration Tests ━━━"

# Rust integration tests (crate-level tests/ directories)
count; run_test "T2.1" cargo test -p vecdb-core --tests -- --nocapture; passed
count; run_test "T2.2" cargo test -p vecdb-cli --tests -- --nocapture; passed
count; run_test "T2.3" cargo test -p vecdb-server --tests -- --nocapture; passed

# Python integration tests
count; run_test "T2.4" python3 tests/tier2_cli_compliance.py; passed
# T2.5 verifies docs/CONFIG.md is regenerated, not merely non-empty.
# It replaced tests/tier2_config_compliance.py, which only checked that each
# field name appeared somewhere in the file — a wrong type, a wrong default, or
# a description contradicting the code all passed. Both of those happened.
count; run_test "T2.5" cargo run -q -p xtask -- gen-config-docs --check; passed
count; run_test "T2.6" python3 tests/tier2_facets.py; passed
count; run_test "T2.6b" python3 tests/tier2_vecdbrc_routing.py; passed
count; run_test "T2.7b" python3 tests/tier2_stale_chunk_purge.py; passed
count; run_test "T2.7c" python3 tests/tier2_dry_run_no_state.py; passed
count; run_test "T2.7" python3 tests/tier2_path_parsing.py; passed
count; run_test "T2.8" python3 tests/tier2_parsers_all.py; passed
count; run_test "T2.9" python3 tests/tier2_compile.py; passed

log ""
log "  ┌──────────────────────────────────────────────┐"
log "  │ GATE: Tier 2 PASSED → Proceeding to Tier 3   │"
log "  │   Proven: components integrate correctly      │"
log "  │   Proven: embedder doesn't hang under load    │"
log "  └──────────────────────────────────────────────┘"

# ══════════════════════════════════════════
# TIER 3: REALITY (End-to-End)
# ══════════════════════════════════════════
log ""
log "━━━ TIER 3: Reality Tests (End-to-End) ━━━"

count; run_test "T3.1" cargo test -p vecdb-cli --test cli_integration -- --nocapture; passed
count; run_test "T3.2" python3 tests/tier3_mcp_e2e.py; passed
count; run_test "T3.3" python3 tests/tier3_mcp_history.py; passed
count; run_test "T3.4" python3 tests/tier3_mcp_resources.py; passed
count; run_test "T3.5" python3 tests/tier3_mcp_server.py; passed
count; run_test "T3.6" python3 tests/tier3_quantization.py; passed

log ""
log "  ┌──────────────────────────────────────────────┐"
log "  │ GATE: Tier 3 PASSED → Proceeding to Tier 4   │"
log "  │   Proven: full E2E flow works at toy scale    │"
log "  └──────────────────────────────────────────────┘"

# ══════════════════════════════════════════
# TIER 4: AGENT REALITY (Production Gauntlet)
# ══════════════════════════════════════════
log ""
log "━━━ TIER 4: Agent Reality (Real Data, Real Scale) ━━━"

count; run_test "T4.1" python3 tests/tier4_realistic_ingest.py; passed
count; run_test "T4.2" python3 tests/tier4_mixed_formats.py; passed
count; run_test "T4.3" python3 tests/tier4_agent_workflow.py; passed

# ══════════════════════════════════════════
# POST-TEST: AUDIT & VERIFICATION
# ══════════════════════════════════════════
log ""
log "━━━ POST-TEST: Audit & Verification ━━━"
log "NOTE: These run LAST for high visibility of warnings."

count; run_test "P.1" python3 tests/tier3_audit_files.py; passed
count; run_test "P.2" python3 tests/tier3_audit_cargo.py; passed
count; run_test "P.3" python3 tests/verify_installed_binary.py; passed

# ══════════════════════════════════════════
# RESULTS
# ══════════════════════════════════════════
log ""
log "═══════════════════════════════════════════════════════════"
log "  RESULTS: $PASS / $TOTAL tests passed"
log "═══════════════════════════════════════════════════════════"

if [ "$PASS" -eq "$TOTAL" ]; then
    log "  ✅ ALL SYSTEMS GREEN"
else
    log "  ❌ FAILURES DETECTED ($((TOTAL - PASS)) failed)"
    exit 1
fi

log ""
log "  Log: $LOGFILE"
log "═══════════════════════════════════════════════════════════"
