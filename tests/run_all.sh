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
#   - THIS FILE is the full manifest. Earlier revisions deferred to planning
#     documents that do not live in this repository; do not reintroduce a
#     pointer to one. The manifest is here or it is nowhere.
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

# ABSOLUTE, deliberately. A relative VECDB_CONFIG resolves against each
# process's own CWD, and `cargo test -p <crate>` runs its test binaries with CWD
# at the CRATE root — so a relative path silently names a different file per
# crate. Enforced by tier0_qdrant_isolation.py, which fails a relative value.
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

# Pin the ONNX Runtime CUDA flavour, matching the Makefile and both workflows.
# Unset, ort-sys picks cu12 or cu13 by sniffing the build machine's nvcc — so
# the gate would otherwise validate whichever artifact this workstation's PATH
# happened to imply, rather than the one we publish. The suite must build what
# ships; T2.5c asserts the result. Set here and not only in the Makefile
# because the gate is invoked as `bash tests/run_all.sh`, bypassing make.
export ORT_CUDA_VERSION=${ORT_CUDA_VERSION:-12}

# ==========================================
# LOGGING
# ==========================================
mkdir -p logs
TIMESTAMP=$(date +%Y%m%d_%H%M%S)
LOGFILE="logs/test_${TIMESTAMP}.log"

log() {
    echo "$1" | tee -a "$LOGFILE"
}

# Per-test wall-clock ceiling.
#
# A HANG MUST FAIL. Without this the suite has no way to tell "slow" from
# "wedged forever", and it waits for the latter indefinitely: on 2026-257 T4.1
# deadlocked against its own server (undrained stderr pipe — see
# tests/lib_stdio.py) and sat there for 40 minutes with every thread asleep,
# producing no output and no verdict. Nobody was going to get a red result; the
# run simply never ended.
#
# 1800s is far above any legitimate test. The slowest leg of a cold run, T1.1's
# full debug build of an ~83 GB workspace, has been measured well inside it, and
# every Tier 4 file declares a budget of 900s in its own header. A test that
# reaches this ceiling is broken, not busy.
#
# Override for a genuinely slower machine: VECDB_TEST_TIMEOUT=3600 make tests
TEST_TIMEOUT_SECS="${VECDB_TEST_TIMEOUT:-1800}"

run_test() {
    local label="$1"
    shift
    log ""
    log "  [$label] $*"

    # `|| status=$?` keeps `set -e` from aborting before the timeout can be
    # reported as a timeout rather than as an anonymous non-zero exit. With
    # `pipefail` set, the status is the command's, not tee's.
    #
    # -k 30: SIGTERM first so a test can tear down its containers and
    # collections, then SIGKILL 30s later if it ignored that. A wedged test
    # holding a test collection is a mess for the NEXT run, not just this one.
    local status=0
    timeout -k 30 "$TEST_TIMEOUT_SECS" "$@" 2>&1 | tee -a "$LOGFILE" || status=$?

    # 124 = timeout fired; 137 = it had to escalate to SIGKILL.
    if [ "$status" -eq 124 ] || [ "$status" -eq 137 ]; then
        log ""
        log "  ✗ [$label] TIMED OUT after ${TEST_TIMEOUT_SECS}s and was killed."
        log "    This is a FAILURE. A hang is not a slow pass."
        log "    If it deadlocked against vecdb-server, check that the test"
        log "    drains stderr — see tests/lib_stdio.py."
        return 1
    fi
    return "$status"
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

# T0.08 is a source scan, so it runs before anything is built.
#
# Six gate entries could not fail — not "did not", *could not*. They logged a
# problem and returned success, and each was counted in the total. This refuses
# the shape mechanically: an if/else where both arms report and neither can
# fail the run.
count; run_test "T0.08" python3 tests/tier0_tests_can_fail.py; passed

# A privacy scan used to sit here. It moved OUT of this repository, to
# sysadmin/githooks/pre-push + sysadmin/tools/privacy-scan.py, because it
# encodes a disclosure policy rather than anything about vecdb — and the same
# policy applies to every repository that gets pushed, so a copy per project
# would drift. It also runs at the moment exposure actually happens: pushing to
# a public forge, not committing to a local branch.

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

# T1.1 builds the binaries every later tier shells out to.
#
# Build with `cuda-dynamic` when this machine has an ONNX Runtime to load. A
# default build cannot register CUDA at all, so every GPU assertion downstream
# would either fail or quietly degrade to CPU — which is how the 2026-254 GPU
# defects survived a green suite. Where there is no runtime, fall back to the
# plain build and tell the GPU-bearing tests to skip explicitly rather than
# letting them fail for the wrong reason.
# The path comes from the ENVIRONMENT. It used to carry a default pointing at
# one machine's disk, which is both a private detail in a public repo and a
# value that is wrong everywhere else — on any other machine the default did
# not exist, so the suite silently took the no-GPU branch and reported green
# without ever covering the operator's real path.
#
# Set it where vecdb is configured: `ort_dylib_path` in config.toml is applied
# to ORT_DYLIB_PATH at startup, or export it directly. See docs/GPU_LEGACY.md.
ORT_LIB="${ORT_DYLIB_PATH:-}"

# Fall back to the machine's configured runtime when the variable is unset.
#
# THE USER SETS NO ENVIRONMENT VARIABLE. `ort_dylib_path` in config.toml is the
# whole integration: vecdb applies it to ORT_DYLIB_PATH itself at startup
# (vecdb-core/src/lib.rs) so shells, hooks and the MCP server need nothing.
# `vecdb config show` reports the resolved path and its origin.
#
# This read is a BOOTSTRAP, and the only place one is justified: the feature
# flag for T1.1 has to be chosen before any binary exists to ask. Everything
# after this point goes through vecdb. Do not grow it into a second config
# resolver — if more than this one path is ever needed here, build first and
# ask `vecdb config show --json`.
#
# Without it a correctly configured GPU machine silently took the no-GPU branch
# and reported green with every GPU case SKIPPED — the same defect class as the
# hardcoded default removed above, reached by a different route.
#
# Deliberately the USER's config, not $VECDB_CONFIG: the location of this
# machine's libonnxruntime.so is a property of the machine, not of the test
# fixture, and the fixture must stay machine-independent. This reads exactly
# one path used to pick a build feature — it does not make tests run against
# the operator's config.
if [ -z "$ORT_LIB" ]; then
    USER_CONFIG="${XDG_CONFIG_HOME:-$HOME/.config}/vecdb/config.toml"
    if [ -f "$USER_CONFIG" ]; then
        ORT_LIB="$(python3 - "$USER_CONFIG" <<'PY' 2>/dev/null || true
import sys, tomllib
try:
    with open(sys.argv[1], "rb") as fh:
        print(tomllib.load(fh).get("ort_dylib_path", "") or "")
except Exception:
    pass
PY
)"
        [ -n "$ORT_LIB" ] && log "  ORT_DYLIB_PATH unset — using ort_dylib_path from $USER_CONFIG"
    fi
fi

if [ -n "$ORT_LIB" ] && [ -f "$ORT_LIB" ]; then
    export ORT_DYLIB_PATH="$ORT_LIB"
    log "  ONNX Runtime found — building with --features cuda-dynamic (GPU paths covered)"
    count; run_test "T1.1" cargo build --bin vecdb --bin vecdb-server --features cuda-dynamic --quiet; passed
    count; run_test "T1.1b" cargo build --bin vecq --quiet; passed

    # A later plain `cargo build --bin vecdb` REPLACES this binary with a
    # default-feature build at the same path. GPU cases then fail with
    # CUBLAS_STATUS_ARCH_MISMATCH on pre-Ampere cards — which is correct,
    # documented behaviour for the default build (docs/GPU_LEGACY.md), not a
    # defect to re-diagnose. If that appears, check what built the binary
    # before anything else.
    #
    # Do NOT "fix" it by copying provider libraries beside the binary. For a
    # cuda-dynamic build they are resolved next to the runtime ORT_DYLIB_PATH
    # names and copies here are inert; for a default build a foreign provider
    # is explicitly unsupported. Either way it hides which build is under test.
else
    export VECDB_ALLOW_NO_GPU=1
    if [ -z "$ORT_LIB" ]; then
        log "  ORT_DYLIB_PATH unset — plain build; GPU cases will report as SKIPPED"
        log "  (set it, or ort_dylib_path in config.toml, to cover the GPU path)"
    else
        log "  No ONNX Runtime at $ORT_LIB — plain build; GPU cases will report as SKIPPED"
    fi
    count; run_test "T1.1" cargo build --bin vecdb --bin vecdb-server --bin vecq --quiet; passed
fi
# T1.1c pins WHAT is under test, immediately after building it.
#
# Re-run at every tier boundary below. A green suite is only a claim about the
# binary it actually executed, and this suite had four different answers to
# which binary that was — see tests/tier0_binary_provenance.py.
count; run_test "T1.1c" python3 tests/tier0_binary_provenance.py "after build"; passed

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

count; run_test "T1.6c" python3 tests/tier0_binary_provenance.py "after tier 1"; passed

log ""
log "  ┌──────────────────────────────────────────────┐"
log "  │ GATE: Tier 1 PASSED → Proceeding to Tier 2   │"
log "  │   Proven: individual components work          │"
log "  │   Proven: the binary under test is this tree  │"
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
# T2.5 verifies docs/CONFIG.md is REGENERATED, not merely non-empty. Checking
# that each field name appears somewhere in the file passes a wrong type, a
# wrong default, and a description that contradicts the code.
count; run_test "T2.5" cargo run -q -p xtask -- gen-config-docs --check; passed
# T2.5b keeps docs/GPU_LEGACY.md honest: the BYO-onnxruntime instructions pin
# an ORT version and a cargo feature; an ort bump or feature rename without a
# doc update costs a user an hours-long onnxruntime build against the wrong
# contract. The doc carries a machine-readable block this test verifies.
count; run_test "T2.5b" python3 tests/tier2_gpu_docs.py; passed
# T2.5c asserts the ONNX Runtime artifact we actually linked, not the variable
# we set: which libcudart major it demands, and which sm_NN kernels it carries.
# ort-sys chooses between the cu12 and cu13 prebuilts by sniffing the BUILD
# MACHINE's nvcc/CUDA_HOME when ORT_CUDA_VERSION is unset, so the same commit
# could ship different runtime requirements from different workstations with
# nothing recording which — we shipped cu13 for months by accident that way.
# The SM list is also the published GPU support matrix in docs/GPU.md; it is
# pyke's to change, so it must be read out of the binary rather than trusted.
count; run_test "T2.5c" python3 tests/tier2_ort_distribution.py; passed
count; run_test "T2.6" python3 tests/tier2_facets.py; passed
count; run_test "T2.6b" python3 tests/tier2_vecdbrc_routing.py; passed
count; run_test "T2.6c" python3 tests/tier2_list_only_vecdb.py; passed
# `cargo test` above rebuilds the `vecdb` binary with DEFAULT features, at the
# same path T1.1 wrote the cuda-dynamic one to. The GPU cases below then run on
# the default build, which cannot drive pre-Ampere cards — CUBLAS_STATUS_ARCH_
# MISMATCH, exactly as docs/GPU_LEGACY.md describes. That is not a GPU
# regression and must not be "fixed" by copying provider libraries around.
#
# Re-assert the supported build here, immediately before the tests that need it.
if [ -n "${ORT_DYLIB_PATH:-}" ]; then
    log "  Re-asserting cuda-dynamic binary (cargo test clobbers it with a default build)"
    cargo build --bin vecdb --features cuda-dynamic --quiet
fi

count; run_test "T2.6d" python3 tests/tier2_routing_and_delete_endpoint.py; passed
count; run_test "T2.6e" python3 tests/tier2_cancel_exit_code.py; passed
# T2.6f — delete must not act on, or lie about, an endpoint it never reached.
#
# Two defects (2026-257). The `--all` locality guard was evaluated in cli.rs
# against one resolution while delete.rs deleted against another, so
# `delete --all --url http://<remote>` passed a guard that had read localhost.
# And `collection_exists` flattened every transport error to `false`, so an
# unreachable store answered "not found — nothing deleted", exit 0.
#
# Needs no Qdrant: the guard bails before connecting, and the unreachable case
# is unreachable by construction (192.0.2.1, TEST-NET-1).
count; run_test "T2.6f" python3 tests/tier2_delete_safety.py; passed
count; run_test "T2.7b" python3 tests/tier2_stale_chunk_purge.py; passed
count; run_test "T2.7d" python3 tests/tier2_pack_granularity.py; passed
count; run_test "T2.7e" python3 tests/tier2_bare_ingest_collection_config.py; passed
count; run_test "T2.7c" python3 tests/tier2_dry_run_no_state.py; passed
count; run_test "T2.7" python3 tests/tier2_path_parsing.py; passed
count; run_test "T2.8" python3 tests/tier2_parsers_all.py; passed
count; run_test "T2.9" python3 tests/tier2_compile.py; passed
# T2.10 — a shipped file may only name a path a clone actually has.
#
# `.gitignore` excludes every docs/ subdirectory except vecq and specs, so a
# working tree legitimately carries private notes that no clone receives.
# Thirteen tracked files linked into them, README.md among them; `install.sh`
# ran a script from a directory that cannot exist. Both are invisible to a
# reader on the machine where the files happen to be present, which is why this
# asks git rather than the filesystem.
count; run_test "T2.10" python3 tests/tier2_tracked_paths.py; passed

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

# T3.1 is `cargo test -p vecdb-cli`, which REBUILDS the `vecdb` binary with
# DEFAULT features at the path T1.1 wrote the cuda-dynamic build to — the same
# clobber the note above T2.6d describes. That note's re-assert guarded four
# tests; everything from here to the end of Tier 4 ran on a default build,
# including the tiers named "Reality" and "Agent Reality". Re-assert again.
if [ -n "${ORT_DYLIB_PATH:-}" ]; then
    log "  Re-asserting cuda-dynamic binary (T3.1 clobbered it)"
    cargo build --bin vecdb --features cuda-dynamic --quiet
fi
count; run_test "T3.1b" python3 tests/tier0_binary_provenance.py "after tier 3 rebuild"; passed
count; run_test "T3.2" python3 tests/tier3_mcp_e2e.py; passed
count; run_test "T3.3" python3 tests/tier3_mcp_history.py; passed
count; run_test "T3.4" python3 tests/tier3_mcp_resources.py; passed
count; run_test "T3.5" python3 tests/tier3_mcp_server.py; passed
count; run_test "T3.6" python3 tests/tier3_quantization.py; passed
# T3.7 pins CLI/MCP parity. `Core::ingest` — one caller, the MCP handler —
# filled in `pack_target_bytes: None`, so the two entry points recorded
# different granularity for the same collection; once the chunking guard began
# comparing, MCP ingest was refused outright. Both paths now go through
# `ingest_with_options` and this compares the corpora they emit.
count; run_test "T3.7" python3 tests/tier3_mcp_cli_chunking_parity.py; passed

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

count; run_test "P.0" python3 tests/tier0_binary_provenance.py "end of run"; passed
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
