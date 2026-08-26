#!/usr/bin/env python3
"""
Tier 0: Test Qdrant Reset

Purpose: Guarantee every suite run starts from an EMPTY test Qdrant, so a green
         run cannot be inherited from a previous one.

Why this exists:
    Per-test cleanup is necessary but not sufficient. A test that fails partway
    leaves its collection behind, and a test that forgets its teardown leaks one
    on every green run. The next run then reads state it did not create. That
    surfaces as a bug in whatever code touches the leftover first — the space
    guard rejecting a collection built by an older default model, for instance,
    which looks like a guard defect and is not one.

    The instance is not containerized per-run, so this script is what makes it
    behave as if it were: functionally ephemeral, without docker lifecycle
    management.

Safety:
    This script DELETES EVERY COLLECTION on its target. It therefore refuses to
    run against anything that is not demonstrably the test instance:
      - target host must be loopback
      - target port must be a test port (6335/6336), never production (6333/6334)
    Production lives on a remote host and on ports 6333/6334. Neither is
    reachable from here by construction.

    (The production hostname was named here literally until day 238. This repo
    is public, so an internal DNS name in a docstring is a disclosure with no
    corresponding benefit — the guard checks loopback and port, never a name.)

Runs BEFORE any Qdrant-touching test, immediately after the isolation gate.
"""

import os
import sys
import json
import urllib.error
import urllib.request
from urllib.parse import urlparse

PROD_QDRANT_PORTS = {"6333", "6334"}
TEST_QDRANT_PORTS = {"6335", "6336"}
LOOPBACK_HOSTS = {"localhost", "127.0.0.1", "::1"}

DEFAULT_HTTP_URL = "http://localhost:6335"


def log(msg, status="INFO"):
    prefix = {"PASS": "[PASS]", "FAIL": "[FAIL]", "WARN": "[WARN]"}.get(status, "[INFO]")
    print(f"{prefix} {msg}", file=sys.stderr)


def assert_is_test_instance(url):
    """
    Refuse to proceed unless the target is unambiguously the test instance.

    Deliberately allowlist-based: an unrecognized host or port is a refusal, not
    a warning. The cost of a false refusal is a failed test run; the cost of a
    false accept is someone's production corpus.
    """
    parsed = urlparse(url)
    host = (parsed.hostname or "").lower()
    port = str(parsed.port) if parsed.port else ""

    if host not in LOOPBACK_HOSTS:
        log(f"Refusing to reset '{url}' — host '{host}' is not loopback.", "FAIL")
        log("This script only ever resets the local test instance.", "FAIL")
        return False

    if port in PROD_QDRANT_PORTS:
        log(f"Refusing to reset '{url}' — port {port} is a PRODUCTION port.", "FAIL")
        return False

    if port not in TEST_QDRANT_PORTS:
        log(f"Refusing to reset '{url}' — port '{port}' is not a test port.", "FAIL")
        log(f"Expected one of: {sorted(TEST_QDRANT_PORTS)}", "FAIL")
        return False

    return True


def http_json(url, method="GET"):
    req = urllib.request.Request(url, method=method)
    with urllib.request.urlopen(req, timeout=10) as resp:
        return json.loads(resp.read().decode())


def main():
    url = os.environ.get("VECDB_TEST_QDRANT_HTTP_URL", DEFAULT_HTTP_URL).rstrip("/")

    if not assert_is_test_instance(url):
        return 1

    try:
        listing = http_json(f"{url}/collections")
    except urllib.error.URLError as e:
        log(f"Test Qdrant unreachable at {url}: {e}", "FAIL")
        log("Start the test instance before running the suite.", "FAIL")
        return 1

    names = [c["name"] for c in listing["result"]["collections"]]

    if not names:
        log(f"Test Qdrant at {url} is already empty.", "PASS")
        return 0

    log(f"Resetting {len(names)} collection(s) on {url}", "INFO")
    failed = []
    for name in sorted(names):
        try:
            http_json(f"{url}/collections/{name}", method="DELETE")
            log(f"  dropped {name}")
        except urllib.error.URLError as e:
            log(f"  FAILED to drop {name}: {e}", "FAIL")
            failed.append(name)

    if failed:
        log(f"Could not drop: {', '.join(failed)}", "FAIL")
        return 1

    remaining = http_json(f"{url}/collections")["result"]["collections"]
    if remaining:
        log(f"Reset incomplete — still present: {[c['name'] for c in remaining]}", "FAIL")
        return 1

    log(f"Test Qdrant reset — {len(names)} collection(s) dropped, instance empty.", "PASS")
    return 0


if __name__ == "__main__":
    sys.exit(main())
