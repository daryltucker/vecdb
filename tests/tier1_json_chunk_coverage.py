#!/usr/bin/env python3
"""
Tier 1: every piece of a JSON file is embedded exactly once — at every size.

Runs the real `vecdb ingest --dry-run --show-chunks --json` (test config,
nothing written) over `tests/fixtures/chunking/json/` and vecq's own JSON
fixture, at ceilings from 120 bytes to 24 KB, and compares structure
(`tests/lib_json_coverage.py`): every scalar, every key, every empty
container, every JSONC comment.

The contract:

  LOSSLESS at every ceiling. Nothing in the source may be absent from the text
  the model receives, however small the ceiling.

  EXACTLY ONCE wherever a chunk's label fits in half the ceiling — every real
  configuration (the smallest plausible derived ceiling is ~1.5 KB). Below
  that the label cannot be repeated into each split part, so it is split along
  with the body and appears once per chunk cut from the record; that is the
  only duplication allowed, and only there.

Why this exists: the JSON chunker had been "fixed" twice with tests that asserted chosen cases. Measuring
input against output found four more defects in an afternoon — a container
emitted whole lost its key, an object of short fields vanished when descended,
re-serialisation dropped every JSONC comment and reordered keys, and a dry run
over a routed `.vecdbrc` created collections.
"""

import glob
import paths  # noqa: F401  (pins the test Qdrant — see paths.py)
import os
import sys

TESTS = os.path.dirname(os.path.abspath(__file__))
sys.path.insert(0, TESTS)
from lib_json_coverage import measure  # noqa: E402

REPO = os.path.dirname(TESTS)
FILES = sorted(glob.glob(os.path.join(TESTS, "fixtures", "chunking", "json", "*.json*"))) + [
    os.path.join(REPO, "vecq", "tests", "fixtures", "json", "structures.json"),
]
# (target_chunk_size, pack_target_bytes); ceiling = 6 x target.
FULL_CONTRACT = [(50, 100), (100, 300), (200, 600), (256, 600), (400, 1000), (1200, 2048), (4000, 8000)]
LOSSLESS_ONLY = [(20, 40), (35, 80)]


def log(msg):
    print(f"[TEST] {msg}", file=sys.stderr)


def main():
    failures = []
    for sizes, keys in ((FULL_CONTRACT, ("missing", "extra")), (LOSSLESS_ONLY, ("missing",))):
        for target, pack in sizes:
            for path in FILES:
                m = measure(path, target=target, pack=pack)
                faults = {k: dict(v) for k, v in m.items() if k.startswith(keys) and v}
                name = os.path.relpath(path, REPO)
                if faults:
                    failures.append(f"ceiling {target * 6} pack {pack} {name}: {faults}")
                else:
                    log(f"  ok: ceiling {target * 6:5} pack {pack:4} {name} ({m['chunks']} chunks)")
    if failures:
        for f in failures:
            log(f"FAILURE: {f[:600]}")
        sys.exit(1)
    log(f"SUCCESS: {len(FILES)} JSON files, {len(FULL_CONTRACT) + len(LOSSLESS_ONLY)} sizings — "
        "nothing lost, nothing duplicated.")


if __name__ == "__main__":
    main()
