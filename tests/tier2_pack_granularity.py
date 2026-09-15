#!/usr/bin/env python3
"""
Tier 2: configured chunk granularity must reach the chunks.

> [!CRITICAL]
> **TEST ISOLATION MANDATE** — test Qdrant only, ports 6335/6336, `test_`
> prefixed collections. NEVER production (6333/6334).

Uses the shared fixture `tests/fixtures/config.toml`
(`test_pack_small` / `test_pack_large` — identical but for `pack_target_bytes`).

**This asserts the artifact, not the configuration.** Two tests already covered
this area and both proved nothing:

  - one asserted `chunking_for(c).target_chunk_size == 384` — the resolver's
    return value. A HashMap lookup. No chunk was ever measured.
  - the other compared point counts, but only with `tokenizer = "bytes"` on
    `.txt` input, which its own comment described as "the one configuration
    where the effect is directly observable". It exercised the generic chunker
    and skipped the AST path the corpus is actually made of.

Both stayed green while the configured granularity had no effect whatsoever on
code, markdown, JSON or YAML. What was missing was a test that ingests real
source into two destinations differing ONLY in granularity and compares the
sizes that come out.

So this fixture is Rust and Markdown, and the assertion is on the emitted chunk
length distribution. If granularity stops reaching the packer again, the medians
converge and this fails.
"""
import json
import os
import shutil
import statistics
import subprocess
import sys
import tempfile
import urllib.request

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
from paths import bin_path

VECDB = bin_path("vecdb")
REPO = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
FIXTURE = os.path.join(REPO, "tests", "fixtures", "config.toml")
HTTP = "http://localhost:6335"
SMALL, LARGE = "test_pack_small", "test_pack_large"
# No default: a hardcoded path is one machine's disk layout, wrong everywhere
# else and private in a public repo. run_all.sh exports ORT_DYLIB_PATH when the
# runtime exists; a cuda-dynamic binary refuses to embed without it, loudly.
DEFAULT_ORT = os.environ.get("ORT_DYLIB_PATH", "")


def drop(c):
    try:
        urllib.request.urlopen(
            urllib.request.Request(f"{HTTP}/collections/{c}", method="DELETE"), timeout=30)
    except Exception:
        pass


def chunk_lengths(c):
    """Content lengths of every real chunk, genesis excluded."""
    out, off = [], None
    while True:
        body = {"limit": 1000, "with_payload": ["content"], "with_vector": False}
        if off:
            body["offset"] = off
        r = json.load(urllib.request.urlopen(urllib.request.Request(
            f"{HTTP}/collections/{c}/points/scroll", data=json.dumps(body).encode(),
            headers={"Content-Type": "application/json"}, method="POST"), timeout=120))["result"]
        for p in r["points"]:
            # Genesis carries no content; it is the nil-UUID marker point.
            if str(p["id"]) == "00000000-0000-0000-0000-000000000000":
                continue
            out.append(len(p["payload"].get("content", "")))
        off = r.get("next_page_offset")
        if not off:
            return sorted(out)


def write_corpus(d):
    """Real source, big enough that granularity has room to express itself.

    Distinct function and section bodies rather than repeated text: identical
    content would collapse to identical chunk IDs and hide a packing difference
    behind deduplication.
    """
    os.makedirs(d, exist_ok=True)
    rs = ["//! Module for the granularity fixture.\n"]
    for i in range(24):
        rs.append(
            f"/// Operation {i} of the fixture.\n"
            f"pub fn operation_{i}(input: &str) -> String {{\n"
            f"    // Body {i} is written out so each function differs from its neighbours.\n"
            f"    let prepared = input.trim().to_lowercase();\n"
            f"    let marker = \"stage-{i}\";\n"
            f"    format!(\"{{}}::{{}}::{i}\", prepared, marker)\n"
            f"}}\n"
        )
    with open(os.path.join(d, "lib.rs"), "w") as f:
        f.write("\n".join(rs))

    md = ["# Granularity fixture\n"]
    for i in range(24):
        md.append(
            f"## Section {i}\n\n"
            f"Section {i} exists so the packer has a real document to work on. It carries "
            f"enough prose to be packed with its neighbours at a loose target and to stand "
            f"apart at a tight one, which is the whole distinction under test here.\n"
        )
    with open(os.path.join(d, "notes.md"), "w") as f:
        f.write("\n".join(md))


def main():
    os.environ["VECDB_CONFIG"] = FIXTURE
    if os.path.exists(os.environ.get("ORT_DYLIB_PATH", DEFAULT_ORT)):
        os.environ.setdefault("ORT_DYLIB_PATH", DEFAULT_ORT)

    tmp = tempfile.mkdtemp()
    data = os.path.join(tmp, "src")
    write_corpus(data)
    failures = []
    try:
        for c in (SMALL, LARGE):
            drop(c)
            r = subprocess.run([VECDB, "ingest", data, "-c", c],
                               capture_output=True, text=True)
            if r.returncode != 0:
                print(f"FAIL: ingest into {c} exited {r.returncode}\n{(r.stdout + r.stderr)[-1200:]}")
                sys.exit(1)

        small, large = chunk_lengths(SMALL), chunk_lengths(LARGE)
        if not small or not large:
            print(f"FAIL: no chunks — small={len(small)} large={len(large)}")
            sys.exit(1)

        ms, ml = statistics.median(small), statistics.median(large)
        print(f"  pack_target 400  -> {len(small):3} chunks, median {ms:.0f} chars")
        print(f"  pack_target 4000 -> {len(large):3} chunks, median {ml:.0f} chars")

        # The configured 10x spread need not appear exactly — AST boundaries are
        # real and a chunk never straddles two declarations. A clear separation
        # is the honest assertion; equality is the defect.
        if ml <= ms * 2:
            failures.append(
                f"granularity did not reach the chunks: median {ms:.0f} vs {ml:.0f} chars for "
                f"pack_target_bytes 400 vs 4000. Equal-ish medians mean the configured value "
                f"is being ignored and some global default is chunking everything.")
        if len(small) <= len(large):
            failures.append(
                f"a tighter target must yield MORE chunks: {len(small)} vs {len(large)}")

        # Genesis must record what actually cut the chunks, or nothing can tell
        # later which granularity produced a corpus.
        for c, want in ((SMALL, 400), (LARGE, 4000)):
            # Genesis lives at the nil UUID. Fetched by id rather than filtered
            # on `__meta_vecdb`, whose value is a version string, not a boolean —
            # a match filter on `true` silently returns nothing and the check
            # passes for the wrong reason.
            g = json.load(urllib.request.urlopen(urllib.request.Request(
                f"{HTTP}/collections/{c}/points",
                data=json.dumps({"ids": ["00000000-0000-0000-0000-000000000000"],
                                 "with_payload": True}).encode(),
                headers={"Content-Type": "application/json"}, method="POST"),
                timeout=60))["result"]
            got = g[0]["payload"].get("__meta_pack_target_bytes") if g else None
            if got != want:
                failures.append(
                    f"{c}: genesis records __meta_pack_target_bytes={got}, expected {want}. "
                    f"Without it the collection cannot say which granularity produced it.")

        # ── The routed case, and the one that actually exercises the plumbing ──
        #
        # Above, each destination got its own `vecdb ingest` invocation, so the
        # run-level resolution already differed and a global parser would pass.
        # Granularity is a property of the DESTINATION: one run fanning across
        # collections must chunk each as configured. That only holds if the
        # parse site asks for the destination's value per file.
        for c in (SMALL, LARGE):
            drop(c)
        routed = os.path.join(tmp, "routed")
        write_corpus(os.path.join(routed, "tight"))
        write_corpus(os.path.join(routed, "loose"))
        with open(os.path.join(routed, ".vecdbrc"), "w") as f:
            f.write(f'[default]\ncollection = "{SMALL}"\n\n'
                    f'[[routes]]\nglob = "loose/**"\ncollection = "{LARGE}"\n')

        r = subprocess.run([VECDB, "ingest", routed], capture_output=True, text=True)
        if r.returncode != 0:
            print(f"FAIL: routed ingest exited {r.returncode}\n{(r.stdout + r.stderr)[-1200:]}")
            sys.exit(1)

        rs, rl = chunk_lengths(SMALL), chunk_lengths(LARGE)
        if not rs or not rl:
            failures.append(f"routed run populated neither side: {len(rs)}/{len(rl)}")
        else:
            mrs, mrl = statistics.median(rs), statistics.median(rl)
            print(f"  routed -> {SMALL}: {len(rs):3} chunks, median {mrs:.0f} chars")
            print(f"  routed -> {LARGE}: {len(rl):3} chunks, median {mrl:.0f} chars")
            if mrl <= mrs * 2:
                failures.append(
                    f"ONE run, two destinations, granularity did not follow the route: "
                    f"median {mrs:.0f} vs {mrl:.0f} chars for pack_target_bytes 400 vs 4000. "
                    f"A run-level parser chunks every destination identically.")

        if failures:
            print("\nFAILED:")
            for f in failures:
                print(f"  - {f}")
            sys.exit(1)
        print("\nPASS: configured granularity reaches the chunks and is recorded in genesis")
    finally:
        for c in (SMALL, LARGE):
            drop(c)
        shutil.rmtree(tmp, ignore_errors=True)


if __name__ == "__main__":
    main()
