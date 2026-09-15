#!/usr/bin/env python3
"""T2.7 — a re-ingested file must not leave its previous chunks behind.

Chunk IDs are a UUIDv5 over the content, so editing a file writes the new
version under a new ID. Nothing removed the point the old version occupied, so
every edit grew the collection and searches returned the pre-edit code next to
the current code with nothing to tell them apart.

This went unnoticed for as long as it did because Python and Go emitted a
constant signature label (`def alpha(...)`) as their chunk content: the content
never changed, so neither did the ID, so nothing accumulated *and* no edit was
ever indexed. Fixing the parsers (see `vecq/tests/tier1_language_fidelity.rs`)
exposed the accumulation immediately — the first probe after that fix produced
two copies of the same function.

The two halves are tested together on purpose. Either one alone can be made to
pass by breaking the other: freeze the content and nothing accumulates; delete
everything and nothing is stale. What must hold is both at once —

    the edit is visible, and only the edit is there.
"""

import json
import os
import subprocess
import sys
import tempfile
import urllib.request
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))
from paths import bin_path  # noqa: E402

REPO = Path(__file__).resolve().parent.parent
QDRANT = "http://localhost:6335"
COLLECTION = "test_stale_purge"

# The packer emits ONE chunk for any file whose total content is under
# `pack_target_bytes` — splitting it cannot improve retrieval, since every piece
# answers the same queries. Undersized neighbours are folded together for the
# same reason.
#
# This test needs alpha and beta in SEPARATE chunks, because the property it
# proves is that editing one leaves the other untouched. So the fixture has to
# be a file that genuinely warrants two chunks: each function clears the floor
# on its own, and the pair clears the pack target. Padding is generated rather
# than typed out so the intent stays legible.
_FILLER = (
    "    It is deliberately verbose so that this function is large enough to be\n"
    "    chunked on its own rather than folded in with its neighbour.\n"
) * 9

_ALPHA_BODY = (
    "    Alpha performs the first half of the worked example used by this test.\n"
    "    It exists to be edited.\n" + _FILLER
)

_BETA_BODY = (
    "    Beta is untouched throughout, and exists to prove that purging the\n"
    "    edited chunk does not take unrelated content with it.\n" + _FILLER
)

ORIGINAL = f'''\
def alpha():
    """Original alpha docstring, mentions marmalade.
{_ALPHA_BODY}"""
    return 1


def beta():
    """Beta is untouched throughout this test.
{_BETA_BODY}"""
    return 2
'''

EDITED = f'''\
def alpha():
    """Rewritten alpha docstring, mentions zeppelins.
{_ALPHA_BODY}"""
    return 999


def beta():
    """Beta is untouched throughout this test.
{_BETA_BODY}"""
    return 2
'''


def fail(msg):
    print(f"FAIL: {msg}", file=sys.stderr)
    sys.exit(1)


def scroll():
    """Every chunk payload in the collection, genesis excluded."""
    req = urllib.request.Request(
        f"{QDRANT}/collections/{COLLECTION}/points/scroll",
        data=json.dumps({"limit": 256, "with_payload": True}).encode(),
        headers={"Content-Type": "application/json"},
    )
    with urllib.request.urlopen(req) as r:
        points = json.load(r)["result"]["points"]
    return [p for p in points if (p["payload"] or {}).get("content")]


def ingest(path, env):
    # `bin_path("vecdb")`, not a bare "vecdb".
    #
    # A bare name resolves through PATH to ~/.cargo/bin/vecdb — whatever was
    # last `make install`ed, which on one measured tree was four hours older
    # than the binary T1.1 had just built. This was the only test in the suite
    # doing it, and nothing caught it: tier0_target_dir_isolation.py scans for
    # the literal "target/debug", which a bare name does not contain.
    proc = subprocess.run(
        [bin_path("vecdb"), "ingest", str(path), "-c", COLLECTION],
        capture_output=True, text=True, env=env,
    )
    if proc.returncode != 0:
        fail(f"ingest failed: {proc.stderr}")
    return proc.stderr


def main():
    config = REPO / "tests" / "fixtures" / "config.toml"
    if not config.exists():
        fail(f"missing test config at {config}")

    env = {**os.environ, "VECDB_CONFIG": str(config)}

    try:
        urllib.request.urlopen(f"{QDRANT}/collections", timeout=5).read()
    except Exception as e:
        fail(f"test Qdrant on :6335 is not reachable ({e}). Start it before running tests.")

    urllib.request.urlopen(
        urllib.request.Request(f"{QDRANT}/collections/{COLLECTION}", method="DELETE")
    ).read()

    with tempfile.TemporaryDirectory() as tmp:
        src = Path(tmp) / "a.py"
        src.write_text(ORIGINAL)

        ingest(tmp, env)
        first = scroll()
        if len(first) != 2:
            fail(f"expected 2 chunks (alpha, beta) on first ingest, got {len(first)}: "
                 f"{[p['payload']['content'][:40] for p in first]}")

        # The parser must be emitting real source, or the rest of this test is
        # vacuous: a signature stub would keep the ID stable and hide everything.
        if not any("marmalade" in p["payload"]["content"] for p in first):
            fail("first ingest did not store the function body — chunk content is "
                 "not verbatim source, so this test cannot detect staleness at all")

        src.write_text(EDITED)
        out = ingest(tmp, env)
        second = scroll()

        contents = [p["payload"]["content"] for p in second]

        # 1. The edit landed.
        if not any("zeppelins" in c for c in contents):
            fail("re-ingest did not index the edited function; the change is invisible "
                 f"to search. Stored: {[c[:50] for c in contents]}")

        # 2. The superseded version is gone.
        if any("marmalade" in c for c in contents):
            fail("the pre-edit version of alpha is still in the collection alongside "
                 "the new one. Searches will return deleted code as if it were current. "
                 f"Stored {len(second)} chunks: {[c[:50] for c in contents]}")

        # 3. Untouched content is untouched — the purge must be scoped to what
        #    actually changed, not a delete-and-rewrite of the whole document.
        if not any("untouched throughout" in c for c in contents):
            fail("beta was removed even though it did not change")

        if len(second) != 2:
            fail(f"expected exactly 2 chunks after re-ingest, got {len(second)}: "
                 f"{[c[:50] for c in contents]}")

        # The run must say what it removed; a silent rewrite of a collection is
        # indistinguishable from a no-op to whoever ran it.
        if "superseded" not in out:
            fail(f"ingest removed a stale chunk but did not report it. stderr was:\n{out}")

    urllib.request.urlopen(
        urllib.request.Request(f"{QDRANT}/collections/{COLLECTION}", method="DELETE")
    ).read()

    print("PASS: re-ingest replaces edited chunks and leaves no stale copies")


if __name__ == "__main__":
    main()
