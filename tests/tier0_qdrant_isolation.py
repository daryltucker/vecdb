#!/usr/bin/env python3
"""
Tier 0: Qdrant Isolation Guard

Purpose: Prove that the test suite will NEVER touch production Qdrant (port 6334/6333).
         This test runs BEFORE any test that may connect to Qdrant.

Checks:
  1. VECDB_CONFIG is set to the test fixture config, not a user config.
  2. All qdrant_url entries in the active config point to test ports (6335/6336 only).
  3. Production Qdrant ports (6333/6334) do NOT appear anywhere in the active test config.
  4. No test Python file hardcodes a production Qdrant URL.

Failure here is a hard gate — no Qdrant-touching tests will run.
"""

import os
import sys
import re

try:
    import tomllib  # Python 3.11+
except ImportError:
    try:
        import tomli as tomllib
    except ImportError:
        print("ERROR: Need Python 3.11+ or 'pip install tomli'", file=sys.stderr)
        sys.exit(1)

PROD_QDRANT_PORTS = {"6333", "6334"}
TEST_QDRANT_PORTS = {"6335", "6336"}
PROD_URL_PATTERN = re.compile(r"localhost:(6333|6334)")

# The ONE authorized test config. No alternatives accepted.
REQUIRED_CONFIG = "tests/fixtures/config.toml"

def log(msg, status="INFO"):
    prefix = {"PASS": "[PASS]", "FAIL": "[FAIL]", "WARN": "[WARN]"}.get(status, "[INFO]")
    print(f"{prefix} {msg}", file=sys.stderr)


def check_config_env():
    """
    ALL TESTS MUST ALWAYS USE THE TESTING CONFIGURATION.
    NEVER HIT PRODUCTION QDRANT (ports 6333/6334).

    VECDB_CONFIG must be set to exactly tests/fixtures/config.toml.
    No other config is accepted. This is non-negotiable.
    """
    config_path = os.environ.get("VECDB_CONFIG")
    if not config_path:
        log("VECDB_CONFIG is NOT set.", "FAIL")
        log("ALL TESTS MUST ALWAYS USE THE TESTING CONFIGURATION.", "FAIL")
        log("Run via: VECDB_CONFIG=tests/fixtures/config.toml python3 tests/<test>.py", "FAIL")
        log("Or use: make tests  (which enforces this automatically)", "FAIL")
        return None

    # Normalize paths for comparison
    normalized = os.path.normpath(config_path)
    required = os.path.normpath(REQUIRED_CONFIG)

    if normalized != required and not normalized.endswith(os.path.normpath(REQUIRED_CONFIG)):
        log(f"VECDB_CONFIG = '{config_path}' — NOT the test fixture!", "FAIL")
        log(f"Required:    '{REQUIRED_CONFIG}'", "FAIL")
        log("ALL TESTS MUST ALWAYS USE THE TESTING CONFIGURATION.", "FAIL")
        log("Do NOT point VECDB_CONFIG at your user config or any other file.", "FAIL")
        return None

    if not os.path.exists(config_path):
        log(f"VECDB_CONFIG points to non-existent file: {config_path}", "FAIL")
        return None

    log(f"VECDB_CONFIG = {config_path} (correct test fixture)", "PASS")
    return config_path


def check_config_urls(config_path):
    """Verify all qdrant_url values in config use test ports only."""
    with open(config_path, "rb") as f:
        config = tomllib.load(f)

    failures = []

    def check_url(url, location):
        if not url:
            return
        for port in PROD_QDRANT_PORTS:
            if f":{port}" in url:
                failures.append(f"{location}: '{url}' uses production port {port}")

    # Top-level qdrant_url
    check_url(config.get("qdrant_url"), "root.qdrant_url")

    # Profile-level qdrant_urls
    for profile_name, profile in config.get("profiles", {}).items():
        check_url(profile.get("qdrant_url"), f"profiles.{profile_name}.qdrant_url")

    # Collection-level qdrant_url overrides
    for coll_name, coll in config.get("collections", {}).items():
        check_url(coll.get("qdrant_url"), f"collections.{coll_name}.qdrant_url")

    if failures:
        for f in failures:
            log(f"Config uses PRODUCTION Qdrant: {f}", "FAIL")
        return False

    log("All qdrant_url entries in test config use test ports (6335/6336).", "PASS")
    return True


def check_test_files_for_hardcoded_prod():
    """Scan test Python files for hardcoded production Qdrant URLs."""
    tests_dir = os.path.join(os.path.dirname(__file__))
    violations = []

    for fname in os.listdir(tests_dir):
        if not fname.endswith(".py"):
            continue
        fpath = os.path.join(tests_dir, fname)
        with open(fpath, "r", encoding="utf-8", errors="replace") as f:
            for lineno, line in enumerate(f, 1):
                if PROD_URL_PATTERN.search(line):
                    # Allow comments and this file itself
                    stripped = line.strip()
                    if stripped.startswith("#") or fname == "tier0_qdrant_isolation.py":
                        continue
                    violations.append(f"{fname}:{lineno}: {stripped[:80]}")

    if violations:
        log("Test files contain hardcoded production Qdrant URLs:", "FAIL")
        for v in violations:
            log(f"  {v}", "FAIL")
        return False

    log("No hardcoded production Qdrant URLs found in test files.", "PASS")
    return True


def check_rust_test_url():
    """Verify VECDB_TEST_QDRANT_URL is set to a test port for Rust integration tests."""
    url = os.environ.get("VECDB_TEST_QDRANT_URL", "")
    if not url:
        log("VECDB_TEST_QDRANT_URL not set — Rust tier2_qdrant tests will be skipped.", "WARN")
        return True  # Warning only, not a hard failure (Rust tests skip themselves)

    for port in PROD_QDRANT_PORTS:
        if f":{port}" in url:
            log(f"VECDB_TEST_QDRANT_URL='{url}' uses PRODUCTION port {port}!", "FAIL")
            log("ALL TESTS MUST ALWAYS USE THE TESTING CONFIGURATION.", "FAIL")
            return False

    log(f"VECDB_TEST_QDRANT_URL = {url} (test port)", "PASS")
    return True


def main():
    log("=== Tier 0: Qdrant Isolation Guard ===")
    log("ALL TESTS MUST ALWAYS USE TESTING CONFIGURATION — NEVER PRODUCTION QDRANT (6333/6334).")

    ok = True

    config_path = check_config_env()
    if config_path is None:
        ok = False
    else:
        if not check_config_urls(config_path):
            ok = False

    if not check_rust_test_url():
        ok = False

    if not check_test_files_for_hardcoded_prod():
        ok = False

    if ok:
        log("=== Isolation guard PASSED. Safe to run Qdrant tests. ===", "PASS")
    else:
        log("=== Isolation guard FAILED. Aborting to protect production data. ===", "FAIL")

    return ok


if __name__ == "__main__":
    sys.exit(0 if main() else 1)
