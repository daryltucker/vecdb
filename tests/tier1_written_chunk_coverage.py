#!/usr/bin/env python3
"""
Tier 1: every token of a YAML or HTML file is embedded exactly once, as
written — at every size.

Runs the real `vecdb ingest --dry-run --show-chunks --json` (test config,
nothing written) over vecq's YAML and HTML fixtures at several pack targets and
compares the source's tokens with the tokens of the chunk bodies. Labels (the
container line a piece was cut from — `services:`, `<div class="nav">` — which
`pack` writes into the chunk text) count toward coverage, never toward
duplication. The small sizings force the descend path even on small files;
`yaml/fleet.yaml` and `html/guide.html` exceed one chunk at every sizing.

Tokens are whitespace-separated, and for HTML also cut between `>` and `<`:
`<head><title>` is two tags, and a chunk boundary between two elements written
on one line is not a loss.

Why this exists (2026-314): YAML and HTML took the code walk — a container's
frame, then each member — so a YAML file was embedded re-assembled from
fragments (indentation, `- ` markers, its header comment and `---` gone,
`["a", "b"]` as `[]` + `"a"` + `"b"`: 29-34% of tokens lost) and HTML
`<head><title>` as `<head></head>` + `<title>` (22% duplicated). Nothing
compared input with output for these types. Fixed by `flatten_written`
(vecdb-core vecq_adapter).
"""

import collections
import paths  # noqa: F401  (pins the test Qdrant — see paths.py)
import os
import re
import sys

TESTS = os.path.dirname(os.path.abspath(__file__))
sys.path.insert(0, TESTS)
from lib_json_coverage import dump, reassemble  # noqa: E402  (shared dry-run runner)

REPO = os.path.dirname(TESTS)
FIXTURES = os.path.join(REPO, "vecq", "tests", "fixtures")
# (target_chunk_size, pack_target_bytes); ceiling = 6 x target.
SIZINGS = [(100, 200), (400, 600), (1200, 2048)]
TAG_BOUNDARY = re.compile(r"(?<=>)(?=<)")


def tokens(text, html):
    out = collections.Counter()
    for tok in text.split():
        out.update(TAG_BOUNDARY.split(tok) if html else [tok])
    return out


def log(msg):
    print(f"[TEST] {msg}", file=sys.stderr)


def main():
    files = []
    for sub, ext in (("yaml", (".yaml", ".yml")), ("html", (".html", ".htm"))):
        d = os.path.join(FIXTURES, sub)
        files += sorted(os.path.join(d, f) for f in os.listdir(d) if f.endswith(ext))
    failures = []
    for target, pack in SIZINGS:
        for path in files:
            html = path.endswith((".html", ".htm"))
            with open(path, encoding="utf-8") as f:
                src = tokens(f.read(), html)
            chunks = dump([path], target=target, pack=pack)
            out, labels = collections.Counter(), collections.Counter()
            # Split parts rejoined first: a token longer than the ceiling is
            # hard-cut across parts, not lost.
            for text, label in reassemble(chunks):
                out.update(tokens(text, html))
                labels.update(tokens(label, html))
            missing = src - out - labels
            extra = out - src
            name = os.path.relpath(path, REPO)
            if missing or extra:
                failures.append(f"pack {pack} {name}: missing {dict(missing)} extra {dict(extra)}")
            else:
                log(f"  ok: pack {pack:5} {name} ({len(chunks)} chunks)")
    if failures:
        for f in failures:
            log(f"FAILURE: {f[:600]}")
        sys.exit(1)
    log(f"SUCCESS: {len(files)} YAML/HTML files, {len(SIZINGS)} sizings — every token once.")


if __name__ == "__main__":
    main()
