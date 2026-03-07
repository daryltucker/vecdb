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

.PHONY: all check test test-rust test-full doc build install clean help run-stdio run

all: check test build

help:
	@echo "$(YELLOW)VecDb MCP Automation$(RESET)"
	@echo "  check      - Run cargo check & clippy"
	@echo "  test       - Run COMPLETE test suite (all tiers)"
	@echo "  test-rust  - Run Rust-only tests (unit + integration)"
	@echo "  doc        - Generate internal docs"
	@echo "  build      - Build Docker image"
	@echo "  install    - Install vecdb binary locally"
	@echo "  run-stdio  - Run docker container (stdio)"
	@echo "  run        - Run docker in interactive mode with volume mount"

# ═══════════════════════════════════════════════════════════
# Dev Workflow
# ═══════════════════════════════════════════════════════════

check:
	@echo "$(YELLOW)Checking...$(RESET)"
	cargo check --workspace
	cargo clippy --workspace -- -D warnings

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
test-rust:
	@echo "$(YELLOW)Rust Tests Only (Unit + Integration)$(RESET)"
	cargo test --workspace

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

install:
	@echo "$(YELLOW)Installing to ~/.cargo/bin...$(RESET)"
	cargo install --path vecdb-cli --force
	cargo install --path vecdb-server --force