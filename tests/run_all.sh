set -e
set -o pipefail

# Tier 1 Test Runner
# Builds and runs all core verification scripts.

PROJECT_ROOT=$(dirname "$0")/..
cd "$PROJECT_ROOT"

# ==========================================
# RESOURCE MANAGEMENT (The "Sane Default")
# ==========================================
# Detect CPU count
TOTAL_CORES=$(nproc)
# Default to half cores, minimum 1
HALF_CORES=$((TOTAL_CORES / 2))
if [ "$HALF_CORES" -lt 1 ]; then HALF_CORES=1; fi

# Allow user override via JOBS env var
JOBS=${JOBS:-$HALF_CORES}

export CARGO_BUILD_JOBS=$JOBS
export RAYON_NUM_THREADS=$JOBS

echo "=== Resource Configuration ==="
echo "Total Cores: $TOTAL_CORES"
echo "Build Jobs:  $CARGO_BUILD_JOBS"
echo "Rayon Threads: $RAYON_NUM_THREADS"
echo "=============================="

# Create logs directory
mkdir -p logs
TIMESTAMP=$(date +%Y%m%d_%H%M%S)

echo "=== Tier 1 Test Suite ===" | tee "logs/tier1_${TIMESTAMP}.log"

# 0. Initialize Fixtures
echo "[0/5] Initializing High-Fidelity Fixtures..." | tee -a "logs/tier1_${TIMESTAMP}.log"
./tests/fixtures/init.sh 2>&1 | tee -a "logs/tier1_${TIMESTAMP}.log"

# 0.5. Ensure Test Qdrant is Running
echo "[0.5/5] Starting Test Qdrant Instance..." | tee -a "logs/tier1_${TIMESTAMP}.log"
python3 tests/tier1_qdrant.py 2>&1 | tee -a "logs/tier1_${TIMESTAMP}.log"

# 1. Build Server
echo "[1/4] Building vecdb-server..." | tee -a "logs/tier1_${TIMESTAMP}.log"
cargo build --bin vecdb-server --quiet 2>&1 | tee -a "logs/tier1_${TIMESTAMP}.log"

# 2. Run Parity Test (Contract)
echo "[2/4] Running Contract Test (Parity)..." | tee -a "logs/tier1_${TIMESTAMP}.log"
python3 tests/tier1_parity.py 2>&1 | tee -a "logs/tier1_${TIMESTAMP}.log"

# 3. Run Security Test
echo "[3/4] Running Security Test (API-Only Mode)..." | tee -a "logs/tier1_${TIMESTAMP}.log"
python3 tests/tier1_security.py 2>&1 | tee -a "logs/tier1_${TIMESTAMP}.log"

# 4. Run Functional Test (MCP)
echo "[4/4] Running Functional Test (MCP Flow)..." | tee -a "logs/tier1_${TIMESTAMP}.log"
python3 tests/tier1_mcp.py 2>&1 | tee -a "logs/tier1_${TIMESTAMP}.log"

# 5. Run Parser Integration Test
echo "[5/5] Running Parser Integration Test (Tier 1)..." | tee -a "logs/tier1_${TIMESTAMP}.log"
./tests/tier1_parsers.sh 2>&1 | tee -a "logs/tier1_${TIMESTAMP}.log"

echo "=== Core Unit Tests ===" | tee "logs/unit_${TIMESTAMP}.log"
# 5.5. Run Vecq Unit & Integration Tests (The Spine: Property & Roundtrip)
echo "[Core] Running vecq Tests (Spine)..." | tee -a "logs/unit_${TIMESTAMP}.log"
cargo test -p vecq 2>&1 | tee -a "logs/unit_${TIMESTAMP}.log"

echo "[Core] Running vecdb-asm Tests..." | tee -a "logs/unit_${TIMESTAMP}.log"
cargo test -p vecdb-asm 2>&1 | tee -a "logs/unit_${TIMESTAMP}.log"

echo "=== Tier 2 Test Suite (Integration) ===" | tee "logs/tier2_${TIMESTAMP}.log"
# 6. Run Tier 2 Integration Tests
echo "[Tier 2] Running Integration Tests..." | tee -a "logs/tier2_${TIMESTAMP}.log"
cargo test --test 'tier2_*' --quiet 2>&1 | tee -a "logs/tier2_${TIMESTAMP}.log"

echo "=== Tier 3 Test Suite (Reality) ===" | tee "logs/tier3_${TIMESTAMP}.log"
# 7. Run Tier 3 Fresh Install Journey (Now in Rust!)
echo "[Tier 3] Running Fresh Install Journey (Rust Integration)..." | tee -a "logs/tier3_${TIMESTAMP}.log"
# Replaces python3 tests/tier3_fresh_install.py
cargo test -p vecdb-cli --test cli_integration 2>&1 | tee -a "logs/tier3_${TIMESTAMP}.log"

# 8. Post Test Tests (Maintenance & Audit)
echo "" | tee -a "logs/tier3_${TIMESTAMP}.log"
echo "=== Post Test Tests (Maintenance & Audit) ===" | tee -a "logs/tier3_${TIMESTAMP}.log"
echo "NOTE: These tests are run LAST to ensure high visibility of warnings." | tee -a "logs/tier3_${TIMESTAMP}.log"
echo "------------------------------------------------------------" | tee -a "logs/tier3_${TIMESTAMP}.log"

echo "[Post-1] Auditing Untracked Files..." | tee -a "logs/tier3_${TIMESTAMP}.log"
python3 tests/tier3_audit_files.py 2>&1 | tee -a "logs/tier3_${TIMESTAMP}.log"

echo "" | tee -a "logs/tier3_${TIMESTAMP}.log"
echo "[Post-2] Auditing Cargo Dependencies..." | tee -a "logs/tier3_${TIMESTAMP}.log"
python3 tests/tier3_audit_cargo.py 2>&1 | tee -a "logs/tier3_${TIMESTAMP}.log"

echo "" | tee -a "logs/tier3_${TIMESTAMP}.log"
echo "=== ALL SYSTEMS GREEN (Tier 1, 2, 3 Passed) ===" | tee -a "logs/tier3_${TIMESTAMP}.log"
