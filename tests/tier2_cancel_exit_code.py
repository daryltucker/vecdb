#!/usr/bin/env python3
"""
Tier 2: cancelling an ingest exits 130, not 0.

> [!CRITICAL]
> **TEST ISOLATION MANDATE** — test Qdrant only, ports 6335/6336, `test_`
> prefixed collections. NEVER production (6333/6334).

Uses the shared fixture `tests/fixtures/config.toml` (profile `route_cpu`).

Found 2026-256. Every `ctrl_c` arm in `vecdb-cli/src/commands/ingest.rs` printed
"Cancelled by user." and then called `std::process::exit(0)` — reporting
SUCCESS for work that did not happen. The consequence was not cosmetic:

    for DIR in .../*/; do
        cd "$DIR" && vecdb ingest ./ || echo "FAILED $DIR"
    done

Bash was told each cancelled directory had ingested cleanly, so the loop
advanced. Ctrl-C killed one file and the run marched on — thirty-five times in
one sitting for the operator, who had to kill the script by PID. The `||` guard
never even fired, because there was no failure to catch.

130 is 128 + SIGINT, the shell convention, and it is what lets a caller tell
"the user stopped this" from "this finished". Same family as `delete` printing
"Done" after a no-op: **an operation that did not happen must not report
success.**
"""
import json
import os
import signal
import subprocess
import sys
import time
import urllib.request

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
from paths import bin_path

VECDB_BIN = bin_path("vecdb")
REPO = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
FIXTURE = os.path.join(REPO, "tests", "fixtures", "config.toml")
COLLECTION = "test_cancel_exit"
CONTAINER_NAME = "qdrant-test"
HTTP = "http://localhost:6335"
# No default: a hardcoded path is one machine's disk layout, wrong everywhere
# else and private in a public repo. run_all.sh exports ORT_DYLIB_PATH when the
# runtime exists; a cuda-dynamic binary refuses to embed without it, loudly.
DEFAULT_ORT = os.environ.get("ORT_DYLIB_PATH", "")
# Big enough that the run is still going when the signal lands.
TARGET = os.path.join(REPO, "vecdb-core", "src")


def ensure_test_qdrant():
    try:
        res = subprocess.run(
            ["docker", "ps", "--filter", f"name={CONTAINER_NAME}", "--format", "{{.ID}}"],
            capture_output=True, text=True, check=True)
        if res.stdout.strip():
            return
        res = subprocess.run(
            ["docker", "ps", "-a", "--filter", f"name={CONTAINER_NAME}", "--format", "{{.ID}}"],
            capture_output=True, text=True, check=True)
        if res.stdout.strip():
            subprocess.run(["docker", "start", CONTAINER_NAME], check=True)
        else:
            subprocess.run(["docker", "run", "-d", "-p", "6335:6333", "-p", "6336:6334",
                            "--name", CONTAINER_NAME, "qdrant/qdrant"], check=True)
        time.sleep(5)
    except subprocess.CalledProcessError as e:
        print(f"CRITICAL FAIL: could not manage test container: {e}")
        sys.exit(1)


def drop():
    try:
        req = urllib.request.Request(f"{HTTP}/collections/{COLLECTION}", method="DELETE")
        urllib.request.urlopen(req, timeout=30)
    except Exception:
        pass


def main():
    ensure_test_qdrant()
    os.environ["VECDB_CONFIG"] = FIXTURE
    # A cuda-dynamic binary refuses to embed at all without this, GPU or not.
    if os.path.exists(os.environ.get("ORT_DYLIB_PATH", DEFAULT_ORT)):
        os.environ.setdefault("ORT_DYLIB_PATH", DEFAULT_ORT)
    failures = []

    drop()
    proc = subprocess.Popen(
        [VECDB_BIN, "--profile", "route_cpu", "ingest", TARGET, "-c", COLLECTION],
        stdout=subprocess.PIPE, stderr=subprocess.STDOUT, text=True)

    # Let it get past model load and actually start embedding, then interrupt.
    time.sleep(12)
    if proc.poll() is not None:
        failures.append("ingest finished before it could be cancelled — "
                        f"pick a larger target than {TARGET}")
    else:
        proc.send_signal(signal.SIGINT)
        try:
            out, _ = proc.communicate(timeout=60)
        except subprocess.TimeoutExpired:
            proc.kill()
            failures.append("ingest did not exit within 60s of SIGINT")
            out = ""

        rc = proc.returncode
        if rc == 0:
            failures.append(
                "cancelled ingest exited 0 — it reports SUCCESS for work that did not "
                "happen, so a shell loop over directories advances instead of stopping. "
                "Expected 130 (128 + SIGINT).")
        elif rc != 130:
            failures.append(f"cancelled ingest exited {rc}, expected 130 (128 + SIGINT)")
        else:
            print("✓ cancelled ingest exits 130")

        if "Cancelled by user" not in out:
            failures.append("cancellation was not announced to the operator")
        else:
            print("✓ cancellation is announced")

    drop()

    if failures:
        print("\nFAILED:")
        for f in failures:
            print(f"  - {f}")
        sys.exit(1)
    print("\nPASS: cancellation exits 130, so callers can tell stopped from finished")


if __name__ == "__main__":
    main()
