#!/usr/bin/env python3
"""
Tier 1: on_oversize policy — `split` and `skip`.

The invariant both policies preserve:

    Never store a chunk whose metadata claims more than its content contains.

`split` keeps the content and labels the parts; `skip` keeps it out. Neither
truncates and NEITHER ABORTS THE RUN.

Regressions pinned here, both found by running the real binary:

  1. `skip` used to abort. When every chunk in a batch was oversized the batch
     emptied, and Qdrant rejects an empty upsert ("Empty update request") — so
     the gentler policy was the one that killed the ingest.
  2. The end-of-run summary named a document UUID instead of the file path,
     which is useless to whoever has to go and look at it.

Also asserts the summary appears under BOTH policies. Reporting only on `skip`
would hide the case where content was silently reshaped rather than dropped.
"""

import os
import shutil
import subprocess
import sys
import tempfile

import sys, os as _os
sys.path.insert(0, _os.path.dirname(_os.path.abspath(__file__)))
from paths import bin_path

REPO = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
BINARY = bin_path("vecdb")
BASE_CONFIG = os.path.join(REPO, "tests", "fixtures", "config.toml")


def log(msg):
    print(f"[TEST] {msg}", file=sys.stderr)


def make_config(tmp, policy):
    """Copy the sanctioned test config and append only the ingestion overrides.

    Copied rather than written from scratch so the test instance URLs come from
    the one authorized fixture — this cannot drift onto production.
    """
    path = os.path.join(tmp, f"config_{policy}.toml")
    with open(BASE_CONFIG) as f:
        lines = f.read().split("\n")

    # Insert into the existing [ingestion] table rather than appending a second
    # one — TOML rejects a duplicate table header.
    out, injected = [], False
    for line in lines:
        out.append(line)
        if line.strip() == "[ingestion]" and not injected:
            out.append("target_chunk_size = 4000")
            out.append("max_chunk_bytes = 2000")
            out.append(f'on_oversize = "{policy}"')
            injected = True
    assert injected, "fixture config has no [ingestion] table to extend"

    with open(path, "w") as f:
        f.write("\n".join(out))
    return path


def run(policy, data_dir, tmp, suffix=""):
    collection = f"test_oversize_{policy}{suffix}"
    subprocess.run([BINARY, "delete", collection, "--yes"],
                   capture_output=True, env={**os.environ, "VECDB_CONFIG": BASE_CONFIG})
    shutil.rmtree(os.path.join(data_dir, ".vecdb"), ignore_errors=True)

    env = {**os.environ, "VECDB_CONFIG": make_config(tmp, policy)}
    proc = subprocess.run([BINARY, "ingest", data_dir, "-c", collection],
                          capture_output=True, text=True, env=env)
    return proc, collection


def main():
    tmp = tempfile.mkdtemp()
    data = os.path.join(tmp, "data")
    os.makedirs(data)

    # One file whose single chunk blows the ceiling, and one that does not.
    with open(os.path.join(data, "big.txt"), "w") as f:
        f.write("The quick brown fox jumps over the lazy dog. " * 400 + "\n")
    with open(os.path.join(data, "small.txt"), "w") as f:
        f.write("tiny file\n")

    # A directory whose every chunk is oversized — see the empty-batch case below.
    lone = os.path.join(tmp, "lone")
    os.makedirs(lone)
    with open(os.path.join(lone, "big.txt"), "w") as f:
        f.write("The quick brown fox jumps over the lazy dog. " * 400 + "\n")

    failures = []
    try:
        for policy in ("split", "skip"):
            log(f"--- on_oversize = {policy} ---")
            proc, collection = run(policy, data, tmp)

            # Regression 1: neither policy may abort.
            if proc.returncode != 0:
                failures.append(
                    f"{policy}: ingest exited {proc.returncode} — a policy for "
                    f"oversized chunks must never abort the run.\n{proc.stderr[-600:]}"
                )
                continue

            # The summary must appear under BOTH policies.
            if "exceeded max_chunk_bytes" not in proc.stderr:
                failures.append(
                    f"{policy}: no oversize summary. The count must be reported "
                    f"regardless of policy.\n{proc.stderr[-400:]}"
                )
                continue

            # Regression 2: it must name the file, not a UUID.
            if "big.txt" not in proc.stderr:
                failures.append(
                    f"{policy}: summary does not name the offending file.\n{proc.stderr[-400:]}"
                )

            expected = "split into labelled parts" if policy == "split" else "NOT indexed"
            if expected not in proc.stderr:
                failures.append(
                    f"{policy}: summary must say what actually happened "
                    f"(expected '{expected}').\n{proc.stderr[-400:]}"
                )

            log(f"  ok: reported, named the file, and did not abort")

            subprocess.run([BINARY, "delete", collection, "--yes"],
                           capture_output=True,
                           env={**os.environ, "VECDB_CONFIG": BASE_CONFIG})

        # The empty-batch case, isolated so it cannot depend on walk order.
        #
        # With a companion file in the same batch there is always something left
        # to upsert, so the bug hides. A directory holding ONLY the oversized
        # file guarantees `skip` empties the batch — which is exactly when Qdrant
        # used to reject the empty upsert and take the run down with it.
        log("--- skip, nothing left to write ---")
        proc, collection = run("skip", lone, tmp, suffix="_lone")
        if proc.returncode != 0:
            failures.append(
                "skip: aborted when every chunk in the batch was skipped. "
                "Nothing to write is a success, not an error.\n"
                + proc.stderr[-600:]
            )
        else:
            log("  ok: completed with nothing to write")
            subprocess.run([BINARY, "delete", collection, "--yes"],
                           capture_output=True,
                           env={**os.environ, "VECDB_CONFIG": BASE_CONFIG})
    finally:
        shutil.rmtree(tmp, ignore_errors=True)

    if failures:
        for f in failures:
            print(f"FAILURE: {f}", file=sys.stderr)
        sys.exit(1)

    log("SUCCESS: both oversize policies report, name files, and complete.")


if __name__ == "__main__":
    main()
