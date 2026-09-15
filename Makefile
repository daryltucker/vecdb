# ═══════════════════════════════════════════════════════════
# VECDB-MCP MAKEFILE
# ═══════════════════════════════════════════════════════════
#
# `make test` MUST run the COMPLETE test suite.
# Partial test runs are a release blocker.
# The tier definitions and the manifest live in tests/run_all.sh itself — it is
# the single source of truth, and `tests` below simply runs it. A pointer here
# must name a file a fresh clone actually has; run_all.sh is that file.
#

PROJECT_NAME := vecdb
IMAGE_NAME   := daryltucker/vecdb-mcp
TAG          := latest
DEBIAN_VER   := trixie

# Which CUDA major the prebuilt ONNX Runtime links against.
#
# Left unset, ort-sys GUESSES from the build machine — it checks CUDA_HOME,
# NV_CUDA_CUDART_VERSION and `nvcc --version`, falling back to 12. That makes
# the artifact a property of the workstation rather than of the commit: two
# people building the same tag get binaries needing different libcudart
# sonames, silently. Pin it so builds are reproducible.
#
# 12 rather than 13 because the two flavours carry the IDENTICAL kernel set
# (sm_75/sm_80/sm_90 — measured, see docs/GPU.md), so the only difference is
# which CUDA the user must have installed, and CUDA 12 is more widely deployed
# and needs a lower minimum driver. This is NOT a compatibility knob for old
# GPUs; see docs/GPU_LEGACY.md for those.
#
# Override deliberately: `make install ORT_CUDA_VERSION=13`.
ORT_CUDA_VERSION ?= 12
export ORT_CUDA_VERSION

# Colors
#
# ESC holds a REAL escape byte, produced once by printf, rather than the
# four-character text `\033`. Recipes run under /bin/sh, and whether its `echo`
# expands backslash escapes is implementation-defined — bash's does not. With
# the literal sequence, every `@echo "$(GREEN)Installed:$(RESET)"` in this file
# printed `\033[1;32mInstalled:\033[0m` verbatim; only the handful using printf
# came out coloured. Substituting the byte here makes all 17 call sites correct
# without touching any of them, and works with echo and printf alike.
ESC    := $(shell printf '\033')
YELLOW := $(ESC)[1;33m
GREEN  := $(ESC)[1;32m
RED    := $(ESC)[1;31m
RESET  := $(ESC)[0m

.PHONY: all check guard-paths guard-workspace test tests test-rust test-perf test-full doc build install install-cuda-dynamic clean help run-stdio run

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

# cargo install normally builds in a private temp target dir, which would
# discard the ORT provider-lib symlinks we need to install below. Pin the
# target dir (respecting an externally-set CARGO_TARGET_DIR) so the symlinks
# land somewhere known — and the build cache is reused as a bonus.
EFFECTIVE_TARGET_DIR := $(or $(CARGO_TARGET_DIR),$(CURDIR)/target)

install:
	@echo "$(YELLOW)Installing to $(INSTALL_ROOT)/bin (locked)...$(RESET)"
	CARGO_TARGET_DIR="$(EFFECTIVE_TARGET_DIR)" CARGO_INSTALL_ROOT="$(INSTALL_ROOT)" cargo install --path vecdb-cli --locked --force
	CARGO_TARGET_DIR="$(EFFECTIVE_TARGET_DIR)" CARGO_INSTALL_ROOT="$(INSTALL_ROOT)" cargo install --path vecdb-server --locked --force
	CARGO_INSTALL_ROOT="$(INSTALL_ROOT)" cargo install --path vecq --locked --force
	@# ORT resolves the CUDA provider libraries relative to dirname(argv[0]) of
	@# the RUNNING EXECUTABLE — not ldconfig, not /usr/local/lib. They must also
	@# come from the exact ONNX Runtime build the binary links (a mismatched
	@# pair aborts the process with free():invalid pointer), which is why we
	@# copy the build's own symlinked artifacts (ort's copy-dylibs feature)
	@# instead of a downloaded release tarball. Without these next to the
	@# binary, use_gpu=true cannot register the CUDA EP (BUG-2026-254).
	@for lib in libonnxruntime_providers_shared.so libonnxruntime_providers_cuda.so; do \
		if [ -e "$(EFFECTIVE_TARGET_DIR)/release/$$lib" ]; then \
			cp -fL "$(EFFECTIVE_TARGET_DIR)/release/$$lib" "$(INSTALL_ROOT)/bin/$$lib" && \
			echo "  installed $$lib (CUDA execution provider)"; \
		else \
			echo "  $(YELLOW)note:$(RESET) $$lib not in $(EFFECTIVE_TARGET_DIR)/release — GPU (use_gpu=true) will not work"; \
		fi; \
	done
	@echo ""
	@echo "$(GREEN)Installed:$(RESET)"
	@for b in vecdb vecdb-server vecq; do \
		printf '  %-14s %s\n' "$$b" "$$(command -v $$b || echo 'NOT ON PATH')"; \
	done
	@echo ""
	@echo "$(YELLOW)Verify the binary on PATH is the one just built:$(RESET)"
	@vecdb --version 2>/dev/null || true

# Bring-your-own ONNX Runtime install (docs/GPU_LEGACY.md): for GPUs the
# prebuilt runtime has dropped. The binaries dlopen the libonnxruntime.so
# named by ORT_DYLIB_PATH at runtime — no provider libs are copied next to
# the binary (they resolve next to YOUR libonnxruntime.so instead). The ORT
# you point at is built once per machine per ORT version; vecdb upgrades
# through this target reuse it unchanged.
install-cuda-dynamic:
	@echo "$(YELLOW)Installing cuda-dynamic (BYO ONNX Runtime) to $(INSTALL_ROOT)/bin (locked)...$(RESET)"
	CARGO_TARGET_DIR="$(EFFECTIVE_TARGET_DIR)" CARGO_INSTALL_ROOT="$(INSTALL_ROOT)" cargo install --path vecdb-cli --locked --force --features cuda-dynamic
	CARGO_TARGET_DIR="$(EFFECTIVE_TARGET_DIR)" CARGO_INSTALL_ROOT="$(INSTALL_ROOT)" cargo install --path vecdb-server --locked --force --features cuda-dynamic
	CARGO_INSTALL_ROOT="$(INSTALL_ROOT)" cargo install --path vecq --locked --force
	@printf '\n'
	@# printf, not echo: make runs recipes under /bin/sh, whose `echo` does NOT
	@# expand \033 escapes — the colour codes printed literally as `\033[1;32m`.
	@printf '$(GREEN)Installed:$(RESET)\n'
	@for b in vecdb vecdb-server vecq; do \
		printf '  %-14s %s\n' "$$b" "$$(command -v $$b || echo 'NOT ON PATH')"; \
	done
	@printf '\n'
	@# These binaries load ONNX Runtime at run time and cannot embed without one.
	@# THE USER SETS NO ENVIRONMENT VARIABLE: `ort_dylib_path` in config.toml is
	@# the whole integration — vecdb applies it to ORT_DYLIB_PATH itself at
	@# startup (vecdb-core/src/lib.rs:124), so shells, editors, cron and the MCP
	@# server inherit it without being told. An exported ORT_DYLIB_PATH still
	@# wins, for one-off overrides. This block used to instruct users to "set it
	@# everywhere vecdb runs", which was true before the config key existed and
	@# has been wrong since.
	@printf '$(YELLOW)Runtime check — the ONNX Runtime these binaries will load:$(RESET)\n'
	@# `vecdb config show` picks its format from the TTY, and a make recipe's
	@# stdout is a PIPE — so this gets JSON (`"onnx_runtime": {...}`), never the
	@# human table (`onnx runtime  <path>`). Matching only the table form printed
	@# a "is it on PATH?" error for a command that had just succeeded. Match
	@# either spelling, and carry the following lines so the JSON object's
	@# path/source/exists come through.
	@vecdb config show 2>/dev/null | grep -iA4 'onnx[ _]runtime' || \
		printf '  $(RED)"vecdb config show" reported no ONNX Runtime — is $(INSTALL_ROOT)/bin on PATH?$(RESET)\n'
	@printf '\n'
	@printf '  Missing, or pointing at the wrong build? Set it once, in\n'
	@printf '  ~/.config/vecdb/config.toml, at the TOP LEVEL (not under a profile):\n'
	@printf '    ort_dylib_path = "/path/to/libonnxruntime.so"\n'
	@printf '  Building that runtime: docs/GPU_LEGACY.md\n'
	@printf '\n'
	@vecdb --version 2>/dev/null || true