#!/usr/bin/env python3
"""
T2.8 — `--dry-run` must not write .vecdb/state.toml.

WHY THIS EXISTS
    The ID-resolution block in ingestion/mod.rs was already guarded with
    `if !options.dry_run`, and its comment says plainly that a dry run must not
    mutate state. Two *other* save sites were not guarded, so a dry run recorded
    every file it previewed. The next real ingest then read that state, decided
    nothing had changed, and skipped the entire corpus — reporting
    "Processed 0, Skipped 5239" for a collection that had never been written.

    Observed 2026-08-26 on a 5,240-file tree.
"""
import os
import subprocess
import sys
import tempfile
from pathlib import Path

sys.path.insert(0, str(Path(__file__).parent))
from paths import find_bin  # noqa: E402

COLLECTION = "test_dry_run_no_state"


def main() -> int:
    vecdb = find_bin("vecdb")
    env = dict(os.environ)
    if "VECDB_CONFIG" not in env:
        print("VECDB_CONFIG unset — refusing to run", file=sys.stderr)
        return 1

    with tempfile.TemporaryDirectory() as tmp:
        root = Path(tmp)
        (root / "sample.md").write_text("# Title\n\nSome prose worth chunking.\n")

        r = subprocess.run(
            [vecdb, "ingest", str(root), "-c", COLLECTION, "--dry-run"],
            env=env, capture_output=True, text=True, timeout=300,
        )
        if r.returncode != 0:
            print(f"dry-run failed: {r.stderr}", file=sys.stderr)
            return 1

        state = root / ".vecdb" / "state.toml"
        if state.exists():
            print(f"FAIL: --dry-run wrote {state}\n{state.read_text()}", file=sys.stderr)
            return 1

    print("PASS: --dry-run left no ingestion state behind")
    return 0


if __name__ == "__main__":
    sys.exit(main())
