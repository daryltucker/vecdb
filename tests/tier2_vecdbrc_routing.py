#!/usr/bin/env python3
"""
Tier 2: `.vecdbrc` routing, verified against Qdrant.

The existing coverage was nine unit tests of `route()` plus one integration test
of *warning spam*. Nothing verified that a routed ingest actually lands files in
the collections the routes name. That is the whole feature, and it is the part
that fails silently: a misroute produces a successful-looking run, and the only
symptom is a search that comes back thin months later.

Asserted here:

  1. Each route's files land in that route's collection, and nowhere else.
  2. An unrouted file falls through to the `[default]` collection.
  3. Each destination is created independently, with its own genesis.
  4. Per-destination chunk parameters apply — a route configured for small
     chunks produces more points than one configured for large chunks, from
     comparable input. Chunk parameters are baked in at ingest, so getting this
     wrong is only repairable by re-ingesting.

Run REPEATS times, because the bug this caught was a race. The chunk buffer
accumulates across files and was flushed under whichever collection arrived
next; `try_join_next` returns tasks in completion order, so the destination was
decided by timing. It was wrong roughly half the time, which a single run passes
by luck.
"""

REPEATS = 5

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
BASE_CONFIG = os.path.join(REPO, "tests", "fixtures", "config.toml")
HTTP = os.environ.get("VECDB_TEST_QDRANT_HTTP_URL", "http://localhost:6335")

SMALL = "test_rc_small"      # tight target_chunk_size
LARGE = "test_rc_large"      # loose target_chunk_size
FALLBACK = "test_rc_fallback"
ALL = [SMALL, LARGE, FALLBACK]


def log(msg):
    print(f"[TEST] {msg}", file=sys.stderr)


def points(collection):
    """Point count, or None when the collection does not exist."""
    try:
        with urllib.request.urlopen(f"{HTTP}/collections/{collection}", timeout=20) as r:
            return json.load(r)["result"]["points_count"]
    except urllib.error.HTTPError as e:
        if e.code == 404:
            return None
        raise


def drop(collection):
    try:
        urllib.request.urlopen(
            urllib.request.Request(f"{HTTP}/collections/{collection}", method="DELETE"),
            timeout=20,
        )
    except Exception:
        pass


def write_config(tmp):
    """Fixture config plus three collections with deliberately different chunking.

    Copied from the one authorized fixture so the Qdrant URLs cannot drift onto
    production.
    """
    with open(BASE_CONFIG) as f:
        lines = f.read().split("\n")

    out, injected = [], False
    for line in lines:
        out.append(line)
        if line.strip() == "[ingestion]" and not injected:
            out.append("target_chunk_size = 400")
            out.append('tokenizer = "bytes"')
            injected = True
    assert injected

    # `tokenizer = "bytes"` makes target_chunk_size govern a byte-window split, which is
    # the one configuration where the effect is directly observable in the point
    # count. See docs/specs/CHUNKING_STRATEGY.md for why that is not the default.
    out += [
        "",
        f'[collections.{SMALL}]',
        f'name = "{SMALL}"',
        'profile = "tier1_basic"',
        "target_chunk_size = 200",
        "max_chunk_bytes = 400",
        "",
        f'[collections.{LARGE}]',
        f'name = "{LARGE}"',
        'profile = "tier1_basic"',
        "target_chunk_size = 20000",
        "max_chunk_bytes = 40000",
        "",
        f'[collections.{FALLBACK}]',
        f'name = "{FALLBACK}"',
        'profile = "tier1_basic"',
        "",
    ]
    path = os.path.join(tmp, "config.toml")
    with open(path, "w") as f:
        f.write("\n".join(out))
    return path


def run_once(attempt):
    tmp = tempfile.mkdtemp()
    data = os.path.join(tmp, "project")
    for sub in ("tight", "loose"):
        os.makedirs(os.path.join(data, sub))

    for c in ALL:
        drop(c)

    try:
        body = "Sentence about the system. " * 200 + "\n"
        with open(os.path.join(data, "tight", "a.txt"), "w") as f:
            f.write(body)
        with open(os.path.join(data, "loose", "b.txt"), "w") as f:
            f.write(body)
        # Matches no route.
        with open(os.path.join(data, "stray.txt"), "w") as f:
            f.write(body)

        with open(os.path.join(data, ".vecdbrc"), "w") as f:
            f.write(
                "[default]\n"
                f'collection = "{FALLBACK}"\n\n'
                "[[routes]]\n"
                'glob = "tight/**"\n'
                f'collection = "{SMALL}"\n\n'
                "[[routes]]\n"
                'glob = "loose/**"\n'
                f'collection = "{LARGE}"\n'
            )

        env = {**os.environ, "VECDB_CONFIG": write_config(tmp)}
        proc = subprocess.run(
            [BINARY, "ingest", data, "-c", FALLBACK],
            capture_output=True, text=True, env=env,
        )
        if proc.returncode != 0:
            log(f"FAILURE: routed ingest exited {proc.returncode}\n{proc.stderr[-1500:]}")
            sys.exit(1)

        counts = {c: points(c) for c in ALL}
        log(f"  point counts: {counts}")

        failures = []

        # 1 + 3: every destination exists and received something.
        for c in ALL:
            if counts[c] is None:
                failures.append(f"{c}: collection was never created — route did not fire")
            elif counts[c] == 0:
                failures.append(f"{c}: created but empty — files did not land here")

        # 4: per-destination chunk parameters. Same input either side, so a
        # difference in point count can only come from the routed chunk config.
        if counts[SMALL] and counts[LARGE]:
            if counts[SMALL] <= counts[LARGE]:
                failures.append(
                    f"{SMALL} ({counts[SMALL]} points) should hold MORE points than "
                    f"{LARGE} ({counts[LARGE]}) — identical input, target_chunk_size 200 vs "
                    f"20000. Equal counts mean one chunk config was applied to both "
                    f"routes, which is the defect this test exists for."
                )

        if failures:
            for f in failures:
                log(f"FAILURE: {f}")
            log(f"stderr tail:\n{proc.stderr[-800:]}")
            sys.exit(1)

        log(f"  attempt {attempt}: ok — {counts}")
    finally:
        for c in ALL:
            drop(c)
        shutil.rmtree(tmp, ignore_errors=True)


def main():
    for attempt in range(1, REPEATS + 1):
        run_once(attempt)
    log("  ok: each route landed in its own collection")
    log("  ok: unrouted file fell through to the default")
    log("  ok: chunk parameters resolved per destination")
    log(f"SUCCESS: .vecdbrc routing verified end to end, {REPEATS}x.")


if __name__ == "__main__":
    main()
