#!/usr/bin/env python3
"""
Tier 2: a bare `vecdb ingest` must honour its destination's own config, and a
collection must refuse chunks cut differently from the ones it already holds.

> [!CRITICAL]
> **TEST ISOLATION MANDATE** — test Qdrant only, ports 6335/6336, `test_`
> prefixed collections. NEVER production (6333/6334).

Uses the shared fixture `tests/fixtures/config.toml`
(`bare_default` / `test_bare_default` / `test_bare_ref`).

Two defects, one root:

1. `resolve_with` looked `[collections.*]` up from the *requested* collection
   name and gave up when there was none — then fell back to the profile's
   `default_collection_name` afterwards, to decide where to WRITE. So a bare
   `vecdb ingest ./` put its data in a collection whose own configuration had
   never been read, and chunked it at the profile's granularity instead.

2. Nothing compared the chunking a collection records in genesis against the
   chunking a later run would use. The embedding-space guard cannot see it —
   two granularities produce perfectly valid vectors in the same space — so the
   collection quietly became a mixture of two corpora with no error anywhere.

Why no existing test caught either: every other test names its collection with
`-c` or a `.vecdbrc`, which are the two paths where the lookup works. A test has
to be deterministic about where it writes, and that discipline is exactly what
kept the profile-default path — the one real ingests use constantly —
unexercised.

**This asserts the artifact, not the resolver.** `tier1_route_chunking.rs`
asserts a resolver return value and stayed green throughout. The assertions here
are on emitted chunk sizes and on the exit code of a second ingest.
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
BARE, REF = "test_bare_default", "test_bare_ref"
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


def genesis(c):
    g = json.load(urllib.request.urlopen(urllib.request.Request(
        f"{HTTP}/collections/{c}/points",
        data=json.dumps({"ids": ["00000000-0000-0000-0000-000000000000"],
                         "with_payload": True}).encode(),
        headers={"Content-Type": "application/json"}, method="POST"),
        timeout=60))["result"]
    return g[0]["payload"] if g else {}


def write_corpus(d):
    """Real source, big enough that granularity has room to express itself.

    Distinct bodies rather than repeated text: identical content collapses to
    identical chunk IDs and would hide a packing difference behind dedup.
    """
    os.makedirs(d, exist_ok=True)
    rs = ["//! Module for the bare-ingest fixture.\n"]
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

    md = ["# Bare ingest fixture\n"]
    for i in range(24):
        md.append(
            f"## Section {i}\n\n"
            f"Section {i} exists so the packer has a real document to work on. It carries "
            f"enough prose to be packed with its neighbours at a loose target and to stand "
            f"apart at a tight one, which is the whole distinction under test here.\n"
        )
    with open(os.path.join(d, "notes.md"), "w") as f:
        f.write("\n".join(md))


def ingest(args):
    return subprocess.run([VECDB, "--profile", "bare_default", "ingest"] + args,
                          capture_output=True, text=True)


def main():
    os.environ["VECDB_CONFIG"] = FIXTURE
    if os.path.exists(os.environ.get("ORT_DYLIB_PATH", DEFAULT_ORT)):
        os.environ.setdefault("ORT_DYLIB_PATH", DEFAULT_ORT)

    tmp = tempfile.mkdtemp()
    data = os.path.join(tmp, "src")
    write_corpus(data)
    failures = []
    try:
        # `.vecdbrc` discovery walks UP from the ingest path. A stray one above
        # the temp dir would supply a `[default] collection` and silently turn
        # this into the already-covered routed case, passing for the wrong
        # reason. Tier 0 guards the repo tree; this guards the temp tree.
        probe = os.path.abspath(data)
        while True:
            if os.path.exists(os.path.join(probe, ".vecdbrc")):
                print(f"FAIL: a .vecdbrc at {probe} would defeat the bare path")
                sys.exit(1)
            parent = os.path.dirname(probe)
            if parent == probe:
                break
            probe = parent

        for c in (BARE, REF):
            drop(c)

        # ── 1. Bare: no -c, no .vecdbrc. The collection comes from the
        #       profile's default_collection_name, so its config must govern.
        r = ingest([data])
        if r.returncode != 0:
            print(f"FAIL: bare ingest exited {r.returncode}\n{(r.stdout + r.stderr)[-1500:]}")
            sys.exit(1)

        # ── 2. Control: same files, same profile, loose granularity.
        r = ingest([data, "-c", REF])
        if r.returncode != 0:
            print(f"FAIL: control ingest exited {r.returncode}\n{(r.stdout + r.stderr)[-1500:]}")
            sys.exit(1)

        bare, ref = chunk_lengths(BARE), chunk_lengths(REF)
        if not bare or not ref:
            print(f"FAIL: no chunks — bare={len(bare)} ref={len(ref)}")
            sys.exit(1)

        mb, mr = statistics.median(bare), statistics.median(ref)
        print(f"  bare    -> {BARE}: {len(bare):3} chunks, median {mb:.0f} chars "
              f"(collection says pack_target_bytes 400)")
        print(f"  -c      -> {REF}: {len(ref):3} chunks, median {mr:.0f} chars "
              f"(profile says 4000)")

        # The configured 10x spread need not appear exactly — AST boundaries are
        # real and a chunk never straddles two declarations. Clear separation is
        # the honest assertion; convergence is the defect.
        if mr <= mb * 2:
            failures.append(
                f"the profile's default collection did not get its own config: median "
                f"{mb:.0f} vs {mr:.0f} chars. A bare ingest chunked at the PROFILE's "
                f"pack_target_bytes 4000 while writing into a collection configured for 400.")
        if len(bare) <= len(ref):
            failures.append(
                f"a tighter target must yield MORE chunks: {len(bare)} vs {len(ref)}")

        # Genesis must record what actually cut the chunks. If the resolver skips
        # the collection entry, this records the profile's value too, and the
        # collection then misdescribes its own contents forever.
        got = genesis(BARE).get("__meta_pack_target_bytes")
        if got != 400:
            failures.append(
                f"{BARE}: genesis records __meta_pack_target_bytes={got}, expected 400 "
                f"from [collections.{BARE}].")

        # ── 3. The guard: a granularity change must be REFUSED, not absorbed.
        #
        # `--overlap` is a granularity field, so re-ingesting with a different
        # one would leave this collection holding two corpora cut two different
        # ways. Nothing compared them before; the space guard is blind to
        # chunking because both sides are valid vectors in the same space.
        #
        # Overlap rather than `--target-chunk-size` because the latter trips
        # `check_chunk_fit` against num_ctx first and never reaches this guard —
        # a real guard, but the wrong one to be testing here.
        r = ingest([data, "--overlap", "7"])
        combined = r.stdout + r.stderr
        if r.returncode == 0:
            failures.append(
                "re-ingesting at a different chunk_overlap was ACCEPTED. The collection "
                "now holds two chunkings and no later read can tell them apart.")
        elif "chunking mismatch" not in combined:
            failures.append(
                f"the ingest was refused, but not for the chunking mismatch — the message "
                f"must say what moved. Got: {combined[-400:]}")
        else:
            print("  re-ingest at a different chunk_overlap -> refused, as it must be")

        # ── 4. And the guard must not cry wolf: unchanged config re-ingests.
        #      Without this, "refuses everything" would pass step 3.
        r = ingest([data])
        if r.returncode != 0:
            failures.append(
                f"an UNCHANGED re-ingest was refused (exit {r.returncode}). The guard must "
                f"catch a real change, not block routine incremental ingests.\n"
                f"      {(r.stdout + r.stderr)[-400:]}")
        else:
            print("  re-ingest at the recorded granularity -> accepted, as it must be")

        if failures:
            print("\nFAILED:")
            for f in failures:
                print(f"  - {f}")
            sys.exit(1)
        print("\nPASS: the profile's default collection governs its own chunks, "
              "and a granularity change is refused")
    finally:
        for c in (BARE, REF):
            drop(c)
        shutil.rmtree(tmp, ignore_errors=True)


if __name__ == "__main__":
    main()
