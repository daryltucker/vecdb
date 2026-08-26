# ═══════════════════════════════════════════════════════════
# VECDB-MCP MAKEFILE
# ═══════════════════════════════════════════════════════════
#
# `make test` MUST run the COMPLETE test suite.
# Partial test runs are a release blocker.
# See: docs/planning/TESTING.md §4 (Tiered Testing Framework)
# See: docs/planning/V1_AUDIT.md §8 (Test Manifest)
#

PROJECT_NAME := vecdb
IMAGE_NAME   := daryltucker/vecdb-mcp
TAG          := latest
DEBIAN_VER   := trixie

# Colors
YELLOW := \033[1;33m
GREEN  := \033[1;32m
RED    := \033[1;31m
RESET  := \033[0m

.PHONY: all check guard-paths guard-workspace test tests test-rust test-perf test-full doc build install clean help run-stdio run

all: check tests build

help:
	@echo "$(YELLOW)VecDb MCP Automation$(RESET)"
	@echo "  check      - Run cargo check & clippy"
	@echo "  tests      - Run COMPLETE test suite (all tiers)"
	@echo "  test-rust  - Run Rust-only tests (unit + integration)"
	@echo "  test-perf  - Run wall-clock performance assertions (serial)"
	@echo "  doc        - Generate internal docs"
	@echo "  build      - Build Docker image"
	@echo "  install    - Install vecdb binary locally"
	@echo "  run-stdio  - Run docker container (stdio)"
	@echo "  run        - Run docker in interactive mode with volume mount"

# ═══════════════════════════════════════════════════════════
# Dev Workflow
# ═══════════════════════════════════════════════════════════

# --all-targets on BOTH commands is load-bearing.
#
# `cargo check --workspace` does not compile test targets, and
# `cargo clippy --workspace` does not lint them or `#[cfg(test)]` modules inside
# src/. Without it this target was blind to everything under tests/, which is
# how four clippy errors sat in vecdb-core's tests behind a green `make check`.
# Do not remove --all-targets from either line.
check: guard-paths guard-workspace
	@echo "$(YELLOW)Checking...$(RESET)"
	cargo fmt --all -- --check
	cargo check --workspace --all-targets
	cargo clippy --workspace --all-targets -- -D warnings

# Absolute home paths in shipped sources are a bug before they are a leak: they
# resolve on exactly one machine. Three shipped in release binaries once, one of
# them forking a doomed `git` on every MCP handshake.
#
# Scoped to non-test sources deliberately — tests/ legitimately holds absolute
# fixture paths. `git grep` exits 0 when it MATCHES, so a match is the failure.
guard-paths:
	@echo "$(YELLOW)Guard: no hardcoded home paths in shipped sources...$(RESET)"
	@if git grep -nE "/home/[a-z]+" -- '*.rs' ':!*/tests/*' ':!tests/*'; then \
		echo "$(RED)FAIL: absolute home path in a non-test source (see above).$(RESET)"; \
		echo "These break on every other machine. Resolve at build time or from config."; \
		exit 1; \
	else \
		echo "$(GREEN)ok$(RESET)"; \
	fi

# Every tracked Cargo.toml must be a workspace member or an explicit exclude.
# A crate that is neither builds locally and fails in a clean clone.
guard-workspace:
	@echo "$(YELLOW)Guard: no tracked-but-unlisted crates...$(RESET)"
	@git ls-files '*Cargo.toml' | python3 -c '\
import sys, tomllib, pathlib; \
ws = tomllib.load(open("Cargo.toml","rb"))["workspace"]; \
known = set(ws.get("members", [])) | set(ws.get("exclude", [])); \
found = {str(pathlib.Path(l.strip()).parent) for l in sys.stdin if l.strip() != "Cargo.toml"}; \
missing = sorted(found - known); \
sys.exit(0) if not missing else (print("FAIL: tracked crates in neither members nor exclude: " + ", ".join(missing)), sys.exit(1))'
	@echo "$(GREEN)ok$(RESET)"

# ───────────────────────────────────────────────────────────
# tests: The COMPLETE test suite. All tiers. No exceptions.
#
# ANTI-CHEAT MANDATE:
#   This target delegates to tests/run_all.sh which is the
#   single source of truth for which tests must pass.
#   Agents MUST NOT bypass this by running individual tests.
#   A release requires `make tests` to pass in its entirety.
# ───────────────────────────────────────────────────────────
tests:
	@echo "$(YELLOW)═══════════════════════════════════════════════$(RESET)"
	@echo "$(YELLOW)  COMPLETE TEST SUITE (All Tiers)$(RESET)"
	@echo "$(YELLOW)═══════════════════════════════════════════════$(RESET)"
	@echo ""
	@echo "$(RED)⚠  Running ALL tests. Partial runs are a release blocker.$(RESET)"
	@echo ""
	bash tests/run_all.sh

# Backward-compat alias
test: tests

# Convenience: Rust-only tests (fast, no Python/Bash)
#
# VECDB_CONFIG is forced here for the same reason run_all.sh forces it: a Rust
# test that reaches Qdrant resolves the URL from config, so an unset
# VECDB_CONFIG resolves to the user's real config and aims the suite at
# production (6333/6334). This target is a documented entry point, so it cannot
# rely on the caller's environment being right.
test-rust:
	@echo "$(YELLOW)Rust Tests Only (Unit + Integration)$(RESET)"
	VECDB_CONFIG="$(CURDIR)/tests/fixtures/config.toml" \
	VECDB_TEST_QDRANT_URL="http://localhost:6336" \
	VECDB_TEST_QDRANT_HTTP_URL="http://localhost:6335" \
	cargo test --workspace

# Wall-clock performance assertions.
#
# Split out of `make tests` deliberately. The gate runs test binaries
# concurrently, so an absolute duration measured there reports machine load as
# much as ingestion speed — a 44-byte fixture timed at 580ms alone and 10.6s
# under the full suite. The ingestion path still runs in the gate; only the
# clock is judged here, serially, with VECDB_PERF_ASSERT=1.
test-perf:
	@echo "$(YELLOW)Performance Assertions (serial)$(RESET)"
	VECDB_CONFIG="$(CURDIR)/tests/fixtures/config.toml" \
	VECDB_TEST_QDRANT_URL="http://localhost:6336" \
	VECDB_TEST_QDRANT_HTTP_URL="http://localhost:6335" \
	VECDB_PERF_ASSERT=1 \
	cargo test -p vecdb-core --test perf_ingestion -- --test-threads=1 --nocapture
	VECDB_CONFIG="$(CURDIR)/tests/fixtures/config.toml" \
	VECDB_TEST_QDRANT_URL="http://localhost:6336" \
	VECDB_TEST_QDRANT_HTTP_URL="http://localhost:6335" \
	VECDB_PERF_ASSERT=1 \
	cargo test -p vecdb-core --test regression_performance -- --test-threads=1 --nocapture
	VECDB_CONFIG="$(CURDIR)/tests/fixtures/config.toml" \
	VECDB_TEST_QDRANT_URL="http://localhost:6336" \
	VECDB_TEST_QDRANT_HTTP_URL="http://localhost:6335" \
	VECDB_PERF_ASSERT=1 \
	cargo test -p vecdb-core --test regression_performance -- --test-threads=1 --nocapture

doc:
	@echo "$(YELLOW)Generating Docs...$(RESET)"
	cargo doc --no-deps --open

# ═══════════════════════════════════════════════════════════
# Docker Workflow
# ═══════════════════════════════════════════════════════════

build:
	@echo "$(YELLOW)Building Docker Image...$(RESET)"
	docker build --build-arg DEBIAN_VERSION=$(DEBIAN_VER) \
		-t $(IMAGE_NAME):$(TAG) \
		-t $(IMAGE_NAME):$(TAG)-$(DEBIAN_VER) .

run-stdio:
	docker run -i --rm \
		-v "$(HOME)/.config/vecdb:/vecdb/config" \
		-v "$(HOME)/.local/share/vecdb:/vecdb/data" \
		-e RUST_LOG=debug \
		$(IMAGE_NAME):$(TAG) start --stdio

run:
	docker run -it --rm \
		-v "$(HOME)/.config/vecdb:/vecdb/config" \
		-v "$(HOME)/.local/share/vecdb:/vecdb/data" \
		-e RUST_LOG=info \
		$(IMAGE_NAME):$(TAG)

# ═══════════════════════════════════════════════════════════
# Local Installation
# ═══════════════════════════════════════════════════════════

# Install destination.
#
# Pinned explicitly because cargo APPENDS "/bin" to CARGO_INSTALL_ROOT. An
# environment with CARGO_INSTALL_ROOT=~/.cargo/bin — which reads as correct —
# therefore installs into ~/.cargo/bin/bin, a directory that is not on PATH.
# `make install` then reports success while the binaries on PATH stay untouched:
# this machine was running vecdb v0.0.9 from January, installed from a git URL,
# through every `make install` since.
#
# Override with `make install INSTALL_ROOT=/some/prefix` (binaries land in
# $(INSTALL_ROOT)/bin).
INSTALL_ROOT ?= $(HOME)/.cargo

install:
	@echo "$(YELLOW)Installing to $(INSTALL_ROOT)/bin (locked)...$(RESET)"
	CARGO_INSTALL_ROOT="$(INSTALL_ROOT)" cargo install --path vecdb-cli --locked --force
	CARGO_INSTALL_ROOT="$(INSTALL_ROOT)" cargo install --path vecdb-server --locked --force
	CARGO_INSTALL_ROOT="$(INSTALL_ROOT)" cargo install --path vecq --locked --force
	@echo ""
	@echo "$(GREEN)Installed:$(RESET)"
	@for b in vecdb vecdb-server vecq; do \
		printf '  %-14s %s\n' "$$b" "$$(command -v $$b || echo 'NOT ON PATH')"; \
	done
	@echo ""
	@echo "$(YELLOW)Verify the binary on PATH is the one just built:$(RESET)"
	@vecdb --version 2>/dev/null || true