#!/usr/bin/env python3
"""
Tier 1: every word of a Markdown file is embedded exactly once — at every size.

Runs the real `vecdb ingest --dry-run --show-chunks --json` (test config,
nothing written) over vecq's Markdown fixtures at several pack targets, and
compares the source's whitespace-separated tokens with the tokens of the chunk
bodies. Heading lines may appear in labels (each chunk opens with its section's
heading path), so label tokens count toward coverage, never toward duplication.

The one exception, and its reason: a THEMATIC BREAK (`---`, `***`, `___` alone
on a line) is not embedded. It carries no words for search to match; it is used
as a break OPPORTUNITY — where the chunker prefers to cut an overflowing run
(vecq_adapter `Candidate::break_before`). Front-matter fences and Setext heading
underlines are not thematic breaks and ARE embedded with their block.

Why this exists (2026-300): Markdown sections were packed together across
headings, paragraphs lost their backticks/emphasis/link URLs, HTML blocks,
front matter and link reference definitions were silently dropped — none of
which any test compared input against output to see.
"""

import collections
import paths  # noqa: F401  (pins the test Qdrant — see paths.py)
import json
import os
import re
import subprocess
import sys

TESTS = os.path.dirname(os.path.abspath(__file__))
sys.path.insert(0, TESTS)
from lib_json_coverage import dump, reassemble  # noqa: E402  (shared dry-run runner)

REPO = os.path.dirname(TESTS)
FIXTURES = os.path.join(REPO, "vecq", "tests", "fixtures", "markdown")
# (target_chunk_size, pack_target_bytes); ceiling = 6 x target.
SIZINGS = [(300, 200), (400, 600), (1200, 2048), (4000, 8000)]
THEMATIC_BREAK = re.compile(r"^ {0,3}([-*_])( *\1){2,} *$")


def source_tokens(text):
    """Tokens of the source, thematic-break lines excluded (see module doc).
    A `---` directly under a text line is a Setext underline, not a break."""
    out = collections.Counter()
    lines = text.split("\n")
    in_front_matter = lines and lines[0].strip() in ("---", "+++")
    for i, line in enumerate(lines):
        if in_front_matter:
            out.update(line.split())
            if i > 0 and line.strip() in ("---", "+++"):
                in_front_matter = False
            continue
        prev = lines[i - 1].strip() if i > 0 else ""
        if THEMATIC_BREAK.match(line) and not prev:
            continue
        out.update(line.split())
    return out


def log(msg):
    print(f"[TEST] {msg}", file=sys.stderr)


def main():
    files = sorted(os.path.join(FIXTURES, f) for f in os.listdir(FIXTURES) if f.endswith(".md"))
    # A larger document, so packing, splitting and section boundaries are exercised.
    files.append(os.path.join(TESTS, "fixtures", "perf_3kb.md"))
    failures = []
    for target, pack in SIZINGS:
        for path in files:
            with open(path, encoding="utf-8") as f:
                src = source_tokens(f.read())
            chunks = dump([path], target=target, pack=pack)
            out, labels = collections.Counter(), collections.Counter()
            # Split parts rejoined first: a token longer than the ceiling (no
            # whitespace to cut at) is hard-cut across parts, not lost.
            for text, label in reassemble(chunks):
                out.update(text.split())
                labels.update(label.split())
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
    log(f"SUCCESS: {len(files)} Markdown files, {len(SIZINGS)} sizings — every word once.")


if __name__ == "__main__":
    main()
