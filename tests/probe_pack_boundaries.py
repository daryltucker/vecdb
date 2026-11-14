#!/usr/bin/env python3
"""probe_pack_boundaries — does a packed chunk hold more than one THING?

NOT A TIER TEST — `probe_` prefix, never added to `run_all.sh`.

It lives in `tests/` because that is what the `.gitignore` allowlist tracks,
and alongside the other non-test helpers here (`lib_envelope.py`, `paths.py`).
`run_all.sh` enumerates tiers by name rather than globbing, so nothing picks
this up as a gate.

It reads whatever collection you point it at, **including a production one**,
and asserts nothing. The isolation mandate (test Qdrant only, 6335/6336,
`test_` prefixes) binds tier tests because they WRITE. This only scrolls
points. Do not give it a `tier` prefix without first giving it a test Qdrant
fixture.

WHY IT EXISTS
-------------
`VecqAdapter::pack` breaks a sibling run when `parent_trail` changes:

    let breaks_sibling_run = run.first()
        .map(|f| f.parent_trail != cand.parent_trail)
        .unwrap_or(false);

That is a VERTICAL boundary — it stops a chunk straddling a parent change. It
does not stop a chunk straddling two SIBLINGS, and for several document shapes
the semantic unit IS a sibling:

    markdown glossary   `## Pass` and `## Path` are siblings under one parent,
                        so 709 terms form ONE run broken only by byte budget
    flat modules        two small `fn`s are siblings; neither reaches
                        `stands_alone`, so they pack into one chunk named
                        after the first
    JSON record arrays  each record is a sibling of the next

`absorb_undersized` then deliberately glues undersized runs to their
neighbours — correct for deeply-nested JSON, actively harmful for a file whose
every element is a small, independently-addressable unit.

Observed 2026-10-14 on `leagueoflegends-lts`: a chunk titled `Path` opened with
the body of `Pass` ("Event passes that links with event tokens") and was the
0.598 top hit for "jungle pathing and tempo".

WHAT IT REPORTS
---------------
  straddle   markdown chunks where content precedes the first heading, i.e.
             the chunk begins mid-section. The chunk's name and its opening
             text belong to different sections.
  multi-unit chunks holding more than one named declaration or heading
  share      per-file share of the index — a file holding half the collection
             dominates top-k by mass regardless of relevance

    tests/probe_pack_boundaries.py -c <collection> --qdrant http://host:6333
    tests/probe_pack_boundaries.py -c <collection> --limit 500 --verbose
"""

from __future__ import annotations
import paths  # noqa: F401  (pins the test Qdrant — see paths.py)

import argparse
import json
import re
import sys
import urllib.error
import urllib.request
from collections import Counter

# A named declaration opening a line. Deliberately shallow: this counts units a
# reader would name the chunk by, not every syntactic form.
DECL = re.compile(
    r"^[ \t]*(?:pub(?:\([^)]*\))?\s+)?(?:async\s+)?"
    r"(fn|struct|enum|trait|impl|class|def|type|const|interface)\s+(\w+)",
    re.M,
)
HEADING = re.compile(r"^(#{1,6})\s+(.+)$", re.M)


def scroll(qdrant: str, collection: str, limit: int) -> list[dict]:
    """Page through points. Read-only; no client library required."""
    url = f"{qdrant.rstrip('/')}/collections/{collection}/points/scroll"
    out: list[dict] = []
    offset = None
    while len(out) < limit:
        body = {"limit": min(256, limit - len(out)), "with_payload": True,
                "with_vector": False}
        if offset is not None:
            body["offset"] = offset
        req = urllib.request.Request(
            url, data=json.dumps(body).encode(),
            headers={"Content-Type": "application/json"},
        )
        try:
            with urllib.request.urlopen(req, timeout=60) as r:
                res = json.load(r)["result"]
        except urllib.error.URLError as exc:
            print(f"cannot reach {url}: {exc}", file=sys.stderr)
            raise SystemExit(2)
        out.extend(res["points"])
        offset = res.get("next_page_offset")
        if offset is None:
            break
    return out


def payload_of(point: dict) -> tuple[str, dict]:
    pl = point.get("payload") or {}
    md = pl.get("metadata") or pl
    return (md.get("content") or pl.get("content") or ""), md


def main() -> int:
    ap = argparse.ArgumentParser(
        description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("-c", "--collection", required=True)
    ap.add_argument("--qdrant", default="http://localhost:6335",
                    help="Qdrant HTTP endpoint (note: HTTP port, not gRPC)")
    ap.add_argument("--limit", type=int, default=2000)
    ap.add_argument("--verbose", action="store_true")
    a = ap.parse_args()

    points = scroll(a.qdrant, a.collection, a.limit)
    if not points:
        print("collection is empty")
        return 0

    per_file = Counter()
    straddles: list[tuple] = []
    multi: list[tuple] = []
    packed = 0

    for p in points:
        content, md = payload_of(p)
        path = md.get("path") or md.get("source") or "?"
        per_file[path] += 1
        n = md.get("packed_elements") or 1
        if n > 1:
            packed += 1

        heads = list(HEADING.finditer(content))
        decls = DECL.findall(content)

        # Straddle: a heading exists, but real content precedes it. The chunk
        # is named for a section it does not begin.
        if heads:
            prefix = content[: heads[0].start()].strip()
            if len(prefix) > 24:
                straddles.append((path, heads[0].group(2)[:40],
                                  prefix.replace("\n", " ")[:64], n))

        units = len(heads) + len({d[1] for d in decls})
        if units > 1:
            names = [h.group(2)[:24] for h in heads] + [d[1] for d in decls]
            multi.append((units, path, names[:6], n))

    total = len(points)
    print(f"collection : {a.collection}   points sampled: {total}")
    print(f"packed (>1 element) chunks : {packed}/{total} ({packed / total:.0%})")
    print(f"straddling chunks          : {len(straddles)}/{total} "
          f"({len(straddles) / total:.0%})  <- named for a section they do not begin")
    print(f"multi-unit chunks          : {len(multi)}/{total} ({len(multi) / total:.0%})")

    print("\nindex share by file (top 8):")
    for path, n in per_file.most_common(8):
        flag = "  <- dominates top-k by mass" if n / total > 0.25 else ""
        print(f"  {n:>5} {n / total:>5.0%}  {path.split('/')[-1]}{flag}")

    if straddles:
        print(f"\nstraddles ({'all' if a.verbose else 'first 8'}):")
        for path, head, prefix, n in (straddles if a.verbose else straddles[:8]):
            print(f"  {path.split('/')[-1]:<34} named '{head}'")
            print(f"       opens with: {prefix!r}  ({n} elements packed)")

    if multi and a.verbose:
        print("\nmulti-unit chunks:")
        for units, path, names, n in sorted(multi, reverse=True)[:12]:
            print(f"  {units} units ({n} elements)  {path.split('/')[-1]:<28} "
                  f"{', '.join(names)}")

    # A straddle is the unambiguous defect: nothing legitimate produces a chunk
    # whose name and opening text come from different sections.
    return 1 if straddles else 0


if __name__ == "__main__":
    sys.exit(main())
