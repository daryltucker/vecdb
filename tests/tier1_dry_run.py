#!/usr/bin/env python3
"""
Tier 1: `ingest --dry-run` answers a useful question, and writes nothing.

Two properties, and the second is why the first is safe to add:

  1. It reports the CHUNK COUNT, not just the file list. "Would ingest: <file>"
     answers a question nobody has — the shell already listed the files. What is
     not knowable without doing the work is how many chunks come out, and
     whether any will trip the oversize ceiling. Parsing and chunking are local;
     embedding and upserting are the expensive parts and neither happens.

  2. It creates NO collection and writes NO points. A dry run that has side
     effects is worse than no dry run, because it is the one command people
     reach for against an unfamiliar target.
"""

import json
import os
import shutil
import subprocess
import sys
import tempfile
import urllib.request

import sys, os as _os
sys.path.insert(0, _os.path.dirname(_os.path.abspath(__file__)))
from paths import bin_path

REPO = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
BINARY = bin_path("vecdb")
CONFIG = os.path.join(REPO, "tests", "fixtures", "config.toml")
COLLECTION = "test_dry_run_must_not_exist"


def log(msg):
    print(f"[TEST] {msg}", file=sys.stderr)


def collections():
    http = os.environ.get("VECDB_TEST_QDRANT_HTTP_URL", "http://localhost:6335")
    with urllib.request.urlopen(f"{http}/collections", timeout=20) as r:
        return {c["name"] for c in json.load(r)["result"]["collections"]}


def main():
    tmp = tempfile.mkdtemp()
    try:
        # Big enough to chunk into more than one piece, so a count is meaningful.
        with open(os.path.join(tmp, "doc.txt"), "w") as f:
            f.write("The quick brown fox jumps over the lazy dog. " * 500 + "\n")

        before = collections()
        assert COLLECTION not in before, "test collection leaked from a previous run"

        env = {**os.environ, "VECDB_CONFIG": CONFIG}
        proc = subprocess.run(
            [BINARY, "ingest", tmp, "-c", COLLECTION, "--dry-run"],
            capture_output=True, text=True, env=env,
        )

        if proc.returncode != 0:
            log(f"FAILURE: dry run exited {proc.returncode}\n{proc.stderr[-600:]}")
            sys.exit(1)

        out = proc.stdout + proc.stderr

        if "chunk(s)" not in out:
            log("FAILURE: dry run reported no chunk count.")
            log("A file listing is not an estimate — the point is to answer "
                "'how much will this produce' before committing to a run.")
            log(f"Output: {out[-400:]}")
            sys.exit(1)
        log("  ok: reports a chunk count")

        # The whole point of a dry run.
        after = collections()
        if COLLECTION in after:
            log(f"FAILURE: dry run CREATED collection '{COLLECTION}'.")
            sys.exit(1)
        if after != before:
            log(f"FAILURE: dry run changed the instance: {after ^ before}")
            sys.exit(1)
        log("  ok: created nothing")
    finally:
        shutil.rmtree(tmp, ignore_errors=True)

    routed_dry_run()
    log("SUCCESS: dry run estimates chunks and writes nothing.")


ROUTED = "test_dry_run_routed_must_not_exist"


def routed_dry_run():
    """A dry run over a `.vecdbrc` that routes files elsewhere creates nothing.

    2026-300: the run flushed its (always empty) buffer to each routed
    collection on every routing switch, and the embedding worker's first act
    for a new collection is to create it with a genesis point. The case above
    has no `.vecdbrc`, so it could not see this.
    """
    tmp = tempfile.mkdtemp()
    try:
        with open(os.path.join(tmp, ".vecdbrc"), "w") as f:
            f.write(f'[default]\ncollection = "{COLLECTION}"\n\n'
                    f'[[routes]]\nglob = "*.json"\ncollection = "{ROUTED}"\n')
        # Alternate the destinations so the run switches collections.
        for i in range(3):
            with open(os.path.join(tmp, f"a{i}.md"), "w") as f:
                f.write(f"# Doc {i}\n\nSome prose for document {i}.\n")
            with open(os.path.join(tmp, f"b{i}.json"), "w") as f:
                f.write(json.dumps({"id": i, "name": f"record {i}"}) + "\n")

        before = collections()
        for name in (COLLECTION, ROUTED):
            assert name not in before, f"test collection {name} leaked from a previous run"

        env = {**os.environ, "VECDB_CONFIG": CONFIG}
        for extra in ([], ["--show-chunks"]):
            proc = subprocess.run(
                [BINARY, "ingest", tmp, "--dry-run", *extra],
                capture_output=True, text=True, env=env,
            )
            if proc.returncode != 0:
                log(f"FAILURE: routed dry run {extra} exited {proc.returncode}\n{proc.stderr[-600:]}")
                sys.exit(1)

        created = collections() - before
        if created:
            log(f"FAILURE: routed dry run CREATED {sorted(created)}.")
            sys.exit(1)
        log("  ok: routed dry run created nothing")
    finally:
        shutil.rmtree(tmp, ignore_errors=True)


if __name__ == "__main__":
    main()
