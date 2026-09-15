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
  5. No `.vecdbrc` sits at or above the repo root. Routing is a SECOND axis of
     isolation: rc discovery walks upward, so a stray routing file silently
     redirects every test that relies on a default collection, while the ingest
     still exits 0 and reports success.

Failure here is a hard gate — no Qdrant-touching tests will run.
"""

import json
import os
import sys
import urllib.request
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

    # MUST be absolute.
    #
    # A relative VECDB_CONFIG is resolved against each process's own CWD, and
    # `cargo test -p <crate>` runs test binaries with CWD at the CRATE root.
    # "tests/fixtures/config.toml" therefore meant vecdb-cli/tests/fixtures/
    # config.toml for every Rust tier-2 test — a different file, whose default
    # collection was the unprefixed production name "docs". The gate cannot
    # enforce "the ONE authorized config" while the path is ambiguous.
    if not os.path.isabs(config_path):
        log(f"VECDB_CONFIG = '{config_path}' is RELATIVE.", "FAIL")
        log("It resolves differently per process CWD; cargo test runs at the crate root.", "FAIL")
        log("Set it to an absolute path. `make tests` does this for you.", "FAIL")
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


def _test_sources():
    """Every shipped test file, Python and Rust.

    Rust was a blind spot. This walked `tests/*.py` only, so
    `vecdb-server/tests/tier2_mcp_integration.rs` carried a literal
    `http://localhost:6334` — the PRODUCTION gRPC port — in a shipped test,
    directly beneath a comment recording the bug that came from doing exactly
    that. Python discipline does not generalise to a language the scanner never
    opened.
    """
    here = os.path.dirname(os.path.abspath(__file__))
    repo = os.path.dirname(here)

    for fname in sorted(os.listdir(here)):
        if fname.endswith(".py"):
            yield os.path.join(here, fname), fname

    for crate in sorted(os.listdir(repo)):
        crate_tests = os.path.join(repo, crate, "tests")
        if not os.path.isdir(crate_tests):
            continue
        for root, _dirs, files in os.walk(crate_tests):
            for fname in sorted(files):
                if fname.endswith(".rs"):
                    path = os.path.join(root, fname)
                    yield path, os.path.relpath(path, repo)


def check_test_files_for_hardcoded_prod():
    """Scan every test source for hardcoded production Qdrant URLs."""
    violations = []
    scanned = 0

    for fpath, label in _test_sources():
        scanned += 1
        with open(fpath, "r", encoding="utf-8", errors="replace") as f:
            for lineno, line in enumerate(f, 1):
                if PROD_URL_PATTERN.search(line):
                    stripped = line.strip()
                    # Comments may name the prod ports — explaining why they are
                    # forbidden is the opposite of using them. `//` covers Rust.
                    if (
                        stripped.startswith("#")
                        or stripped.startswith("//")
                        or label == "tier0_qdrant_isolation.py"
                    ):
                        continue
                    violations.append(f"{label}:{lineno}: {stripped[:80]}")

    if violations:
        log("Test files contain hardcoded production Qdrant URLs:", "FAIL")
        for v in violations:
            log(f"  {v}", "FAIL")
        return False

    log(
        f"No hardcoded production Qdrant URLs in {scanned} test files (.py and .rs).",
        "PASS",
    )
    return True


def check_collection_names_are_test_prefixed():
    """
    Every collection a test creates must be named `test_*`.

    Two distinct failures this prevents:

      1. A test that omits `-c` inherits the profile default. When that default
         was named after a real collection ("docs"), the tests wrote to a
         production *name*, and only the port kept them off production data.
      2. Un-prefixed names are indistinguishable from real ones when reviewing
         the instance, so nobody can safely purge leftovers — and leftovers are
         how one run's state leaks into the next. `git_test`, `history_v1`,
         `inc_test` and `tier1_lua` all accumulated this way.

    Scans for collection names passed on the command line or in MCP arguments.
    """
    tests_dir = os.path.dirname(__file__)
    # `--collection X`, `--collection=X`, `-c X`, and JSON `"collection": "X"`.
    patterns = [
        re.compile(r"--collection[= ]+[\"']?([A-Za-z0-9_\-]+)"),
        re.compile(r"\"collection\"\s*:\s*\"([A-Za-z0-9_\-]+)\""),
        # Assignment form. Added after `TEST_COLLECTION = "tier1_embedder_test"`
        # slipped through a literal-only scan: the flag is built from the
        # variable further down, so nothing matched at the call site.
        re.compile(r"^\s*[A-Za-z_]*COLLECTION[A-Za-z_]*\s*=\s*\"([A-Za-z0-9_\-]+)\"", re.M),
    ]
    # Values that are not literal collection names.
    placeholders = {"collection", "name", "None", "null"}

    violations = []
    for fname in sorted(os.listdir(tests_dir)):
        if not fname.endswith(".py") or fname == os.path.basename(__file__):
            continue
        fpath = os.path.join(tests_dir, fname)
        with open(fpath, "r", encoding="utf-8", errors="replace") as f:
            for lineno, line in enumerate(f, 1):
                if line.strip().startswith("#"):
                    continue
                for pat in patterns:
                    for name in pat.findall(line):
                        # Interpolated names (f-strings, variables) resolve at
                        # runtime; the literal check cannot judge them.
                        if name in placeholders or "{" in name:
                            continue
                        if not name.startswith("test_"):
                            violations.append(f"{fname}:{lineno}: collection '{name}'")

    if violations:
        log("Test collections must be named test_*:", "FAIL")
        for v in violations:
            log(f"  {v}", "FAIL")
        log("Rename them. An un-prefixed collection cannot be safely purged and", "FAIL")
        log("will leak state from one test run into the next.", "FAIL")
        return False

    log("All literal test collection names are test_-prefixed.", "PASS")
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


def check_no_ancestor_vecdbrc():
    """No `.vecdbrc` may sit above the test tree.

    `.vecdbrc` discovery walks UP from the ingest path (`vecdbrc.rs:61`), so a
    routing file anywhere at or above the repo root silently captures every test
    that relies on a profile's `default_collection_name`. The ingest still exits
    0 and still reports "Processed N" — the points simply land somewhere else.

    A developer's own routing file at the repo root is enough to do it, and the
    resulting gate failure names neither the cause nor the file. The ports guard
    above keeps that off production; routing is a second axis of isolation and
    needs its own check.
    """
    repo = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
    found = []
    d = repo
    while True:
        rc = os.path.join(d, ".vecdbrc")
        if os.path.exists(rc):
            found.append(rc)
        parent = os.path.dirname(d)
        if parent == d:
            break
        d = parent

    if not found:
        log("No .vecdbrc above the test tree — default-collection routing is clean.", "PASS")
        return True

    for rc in found:
        log(f".vecdbrc found at {rc}", "FAIL")
        try:
            with open(rc) as f:
                for line in f:
                    if "collection" in line and not line.strip().startswith("#"):
                        log(f"    routes to: {line.strip()}", "FAIL")
        except OSError:
            pass
    log("This silently redirects every test that uses a default collection.", "FAIL")
    log("Move it for the duration of the run, then put it back:", "FAIL")
    log(f"    mv {found[0]} {found[0]}.off  &&  <run tests>  &&  mv {found[0]}.off {found[0]}", "FAIL")
    return False


def check_production_has_no_test_collections():
    """Production must hold no `test_`-prefixed collection.

    Every other check here is preventive and inspects files. This one is
    DETECTIVE and inspects reality, because the preventive checks have a blind
    spot they cannot close: a test that generates its own config at runtime is
    invisible to a source scan.

    That blind spot was not hypothetical. A tier-3 test wrote a config into a
    temp HOME and patched the endpoint with a regex; when `init` changed shape
    the regex matched nothing, the patch silently did nothing, and the ingest
    landed a `test_` collection in production. Nothing failed. The test failed
    later, for an unrelated-looking reason.

    A `test_`-prefixed collection in production has exactly one cause, so
    finding one is proof a test escaped its sandbox. Reported, never deleted —
    naming the target is the operator's call to act on.
    """
    prod = os.environ.get("VECDB_PROD_QDRANT_HTTP_URL", "http://localhost:63" + "33")
    try:
        with urllib.request.urlopen(f"{prod}/collections", timeout=5) as r:
            names = [c["name"] for c in json.load(r)["result"]["collections"]]
    except Exception:
        # Not running, not reachable, not our business. Absence of production is
        # not evidence of leakage.
        log("Production Qdrant not reachable — leak check skipped.", "PASS")
        return True

    leaked = sorted(n for n in names if n.startswith("test_"))
    if leaked:
        log("PRODUCTION CONTAINS TEST COLLECTIONS — a test escaped its sandbox:", "FAIL")
        for n in leaked:
            log(f"    {n}", "FAIL")
        log("Find the test that generates its own config and does not pin the", "FAIL")
        log("endpoint. Then remove these by name.", "FAIL")
        return False

    log("Production holds no test_ collections — no test has escaped.", "PASS")
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

    if not check_collection_names_are_test_prefixed():
        ok = False

    if not check_no_ancestor_vecdbrc():
        ok = False

    if not check_production_has_no_test_collections():
        ok = False

    if ok:
        log("=== Isolation guard PASSED. Safe to run Qdrant tests. ===", "PASS")
    else:
        log("=== Isolation guard FAILED. Aborting to protect production data. ===", "FAIL")

    return ok


if __name__ == "__main__":
    sys.exit(0 if main() else 1)
