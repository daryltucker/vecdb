#!/usr/bin/env python3
"""
Tier 1: a unit is its own chunk, whatever the file's size.

A UNIT is a Markdown section or a named code unit (function, method, class,
struct, interface, trait). Two units never share a chunk: a small file with two
functions is two chunks, and a small Markdown file with three sections is three
— each named for the unit it holds, not for the first unit of the file.

Size only caps a unit; it never merges units. The whole-file-as-one-chunk rule
stays for files with NO units (flat prose, a small JSON document), where there
is nothing to keep apart.

Runs the real `vecdb ingest --dry-run --show-chunks --json` (test config,
nothing written) at a pack target far above every fixture, so the only thing
that can separate units is the unit boundary itself.

Why this exists (Ecosystem Experts on `b79f78e`): `pack()` returned the whole
file as one chunk whenever it was under `pack_target_bytes`, before any unit
boundary was consulted. A 2-function `.py`/`.rs` came out as one chunk labelled
with the first function (the whole-file-as-one-chunk defect), and a small multi-section
Markdown file as one chunk labelled with its first section. Every existing test
of unit boundaries padded its fixture over the target to step around the rule.
"""

import os
import paths  # noqa: F401  (pins the test Qdrant — see paths.py)
import sys

TESTS = os.path.dirname(os.path.abspath(__file__))
sys.path.insert(0, TESTS)
from lib_json_coverage import body, dump  # noqa: E402  (shared dry-run runner)

REPO = os.path.dirname(TESTS)
FIXTURES = os.path.join(TESTS, "fixtures", "chunking", "units")
# Far above every fixture: under the old rule each file was ONE chunk.
TARGET, PACK = 4000, 8000

# fixture -> one (marker, trail) per unit, in order. The marker appears only in
# that unit's own text; the trail is what the chunk must be named by.
UNITS = {
    "two_functions.rs": [("x + 1", "alpha"), ("x * 2", "beta")],
    "two_functions.py": [("x + 1", "alpha"), ("x * 2", "beta")],
    "small_sections.md": [
        ("Intro to the guide.", "Guide"),
        ("Run the installer.", "Guide::Install"),
        ("Edit the config file.", "Guide::Configure"),
    ],
}
# Files with no units stay whole.
WHOLE = ["flat_prose.md", "small_object.json"]


def log(msg):
    print(f"[TEST] {msg}", file=sys.stderr)


def main():
    failures = []
    for name, units in UNITS.items():
        chunks = dump([os.path.join(FIXTURES, name)], target=TARGET, pack=PACK)
        if len(chunks) != len(units):
            failures.append(f"{name}: {len(units)} units, {len(chunks)} chunks")
        for marker, trail in units:
            holding = [c for c in chunks if marker in body(c)]
            if len(holding) != 1:
                failures.append(f"{name}: {marker!r} in {len(holding)} chunks")
                continue
            chunk = holding[0]
            others = [m for m, _ in units if m != marker and m in body(chunk)]
            if others:
                failures.append(f"{name}: {marker!r} shares a chunk with {others}")
            got = chunk.get("crumbtrail", "")
            if got != trail and not got.endswith("::" + trail):
                failures.append(f"{name}: {marker!r} chunk named {got!r}, not {trail!r}")
        if not any(f.startswith(name) for f in failures):
            log(f"  ok: {name} ({len(chunks)} chunks, one per unit)")
    for name in WHOLE:
        chunks = dump([os.path.join(FIXTURES, name)], target=TARGET, pack=PACK)
        if len(chunks) != 1:
            failures.append(f"{name}: no units, yet {len(chunks)} chunks")
        else:
            log(f"  ok: {name} (whole file, 1 chunk)")
    if failures:
        for f in failures:
            log(f"FAILURE: {f}")
        sys.exit(1)
    log(f"SUCCESS: {len(UNITS)} files split by unit, {len(WHOLE)} unit-less files whole.")


if __name__ == "__main__":
    main()
