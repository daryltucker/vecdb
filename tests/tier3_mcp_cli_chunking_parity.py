#!/usr/bin/env python3
"""T3.7 — CLI and MCP must build the same corpus from the same files.

> [!CRITICAL]
> **TEST ISOLATION MANDATE** — test Qdrant only, ports 6335/6336, `test_`
> prefixed collections. NEVER production (6333/6334).

WHAT THIS PROVES
    Ingesting one corpus into one collection produces the same chunks whether it
    goes through `vecdb ingest` or the MCP `ingest_path` tool, and both record
    the same chunking in genesis.

WHY IT EXISTS
    They did not. `Core::ingest` — a convenience wrapper used by exactly one
    caller, the MCP handler — pinned `pack_target_bytes: None`, so
    `chunking_identity()` recorded the 2048 default while the CLI recorded the
    collection's configured value. `pack_target_bytes` is a GRANULARITY field,
    so once `ensure_write_target` started comparing chunking (2026-256) the
    result was:

        CLI creates `X` at pack_target 400  →  genesis records 400
        MCP ingests into `X`                →  computes 2048  →  REFUSED

    Before that guard existed the same gap was silent and worse: one collection
    holding two granularities, every later search ranking them against each
    other, nothing anywhere reporting it.

    The wrapper also hardcoded `strategy` and `tokenizer` past the config. It
    and `ingest_routed` (which had no callers at all) were deleted; both paths
    now go through `ingest_with_options`, where the caller states every
    parameter.

WHY PARITY RATHER THAN A CONSTANT
    Asserting `pack_target_bytes == 400` would prove the one field that was
    wrong. Two entry points into one collection can disagree about any of them —
    overlap, tokenizer, ceiling, strategy. Comparing the emitted corpora catches
    the class, and comparing genesis catches a disagreement that has not yet
    changed the chunks.

    Reverting the fix (restoring `pack_target_bytes: None` on the MCP path) must
    make this fail. It does: the MCP ingest is refused outright.
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
from paths import bin_path, require_bin
from lib_stdio import drain_stderr

REPO = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
FIXTURE = os.path.join(REPO, "tests", "fixtures", "config.toml")
HTTP = os.environ.get("VECDB_TEST_QDRANT_HTTP_URL", "http://localhost:6335")

# Two collections in the shared fixture, identical but for pack_target_bytes.
# Using the tight one: a wrong granularity is most visible where it is smallest.
COLLECTION = "test_pack_small"          # [collections.test_pack_small] pack_target_bytes = 400
EXPECTED_PACK_TARGET = 400


def drop(c):
    try:
        urllib.request.urlopen(
            urllib.request.Request(f"{HTTP}/collections/{c}", method="DELETE"), timeout=30
        )
    except Exception:
        pass


def chunk_lengths(c):
    out, off = [], None
    while True:
        body = {"limit": 1000, "with_payload": ["content"], "with_vector": False}
        if off:
            body["offset"] = off
        r = json.load(
            urllib.request.urlopen(
                urllib.request.Request(
                    f"{HTTP}/collections/{c}/points/scroll",
                    data=json.dumps(body).encode(),
                    headers={"Content-Type": "application/json"},
                    method="POST",
                ),
                timeout=120,
            )
        )["result"]
        for p in r["points"]:
            if str(p["id"]) == "00000000-0000-0000-0000-000000000000":
                continue
            out.append(len(p["payload"].get("content", "")))
        off = r.get("next_page_offset")
        if not off:
            return sorted(out)


def genesis(c):
    g = json.load(
        urllib.request.urlopen(
            urllib.request.Request(
                f"{HTTP}/collections/{c}/points",
                data=json.dumps(
                    {"ids": ["00000000-0000-0000-0000-000000000000"], "with_payload": True}
                ).encode(),
                headers={"Content-Type": "application/json"},
                method="POST",
            ),
            timeout=60,
        )
    )["result"]
    return g[0]["payload"] if g else {}


def write_corpus(d):
    """Real source, distinct bodies so identical content cannot mask a difference."""
    os.makedirs(d, exist_ok=True)
    rs = ["//! Module for the parity fixture.\n"]
    for i in range(24):
        rs.append(
            f"/// Operation {i} of the fixture.\n"
            f"pub fn operation_{i}(input: &str) -> String {{\n"
            f"    // Body {i} differs from its neighbours so packing is observable.\n"
            f"    let prepared = input.trim().to_lowercase();\n"
            f'    let marker = "stage-{i}";\n'
            f'    format!("{{}}::{{}}::{i}", prepared, marker)\n'
            f"}}\n"
        )
    with open(os.path.join(d, "lib.rs"), "w") as f:
        f.write("\n".join(rs))


class Mcp:
    """One MCP stdio session against the INSTALLED-tree server binary."""

    def __init__(self, env):
        self.p = subprocess.Popen(
            [require_bin("vecdb-server"), "--stdio", "--allow-local-fs"],
            stdin=subprocess.PIPE, stdout=subprocess.PIPE, stderr=subprocess.PIPE,
            text=True, env=env,
        )
        # Drain stderr continuously so the server can never block writing to
        # a full stderr pipe while this test blocks reading stdout.
        # See tests/lib_stdio.py for the deadlock this prevents.
        self._stderr = drain_stderr(self.p)
        self.rpc("initialize")

    def rpc(self, method, params=None):
        req = {"jsonrpc": "2.0", "method": method, "id": 1}
        if params:
            req["params"] = params
        self.p.stdin.write(json.dumps(req) + "\n")
        self.p.stdin.flush()
        line = self.p.stdout.readline()
        if not line:
            raise AssertionError(f"server died: {self._stderr()}")
        return json.loads(line)

    def close(self):
        self.p.terminate()
        try:
            self.p.communicate(timeout=5)
        except subprocess.TimeoutExpired:
            self.p.kill()


def main():
    env = {**os.environ, "VECDB_CONFIG": FIXTURE, "VECDB_ALLOW_LOCAL_FS": "true"}
    tmp = tempfile.mkdtemp()
    data = os.path.join(tmp, "src")
    write_corpus(data)
    failures = []

    try:
        # ── 1. CLI ingest. This is the reference corpus.
        drop(COLLECTION)
        r = subprocess.run(
            [bin_path("vecdb"), "ingest", data, "-c", COLLECTION],
            capture_output=True, text=True, env=env,
        )
        if r.returncode != 0:
            print(f"FAIL: CLI ingest exited {r.returncode}\n{(r.stdout + r.stderr)[-1200:]}")
            return 1
        cli_chunks = chunk_lengths(COLLECTION)
        cli_genesis = genesis(COLLECTION)
        if not cli_chunks:
            print("FAIL: CLI ingest produced no chunks")
            return 1

        # ── 2. MCP ingest, same files, same collection.
        #
        # Into the SAME collection deliberately: that is where the two paths
        # actually meet, and where a disagreement is refused by the chunking
        # guard rather than merely different.
        mcp = Mcp(env)
        try:
            res = mcp.rpc("tools/call", {
                "name": "ingest_path",
                "arguments": {"path": data, "collection": COLLECTION},
            })
        finally:
            mcp.close()

        if "error" in res:
            failures.append(
                "MCP ingest into a CLI-created collection was REFUSED:\n"
                f"      {res['error'].get('message', res['error'])}\n"
                "      The two entry points disagree about how to cut this collection."
            )
        else:
            mcp_chunks = chunk_lengths(COLLECTION)
            mcp_genesis = genesis(COLLECTION)

            # Same corpus, re-ingested: content is unchanged, so the chunk set
            # must be identical. Any difference is a granularity difference.
            if mcp_chunks != cli_chunks:
                failures.append(
                    f"CLI and MCP produced different corpora from the same files:\n"
                    f"      CLI: {len(cli_chunks)} chunks, median "
                    f"{statistics.median(cli_chunks):.0f} chars\n"
                    f"      MCP: {len(mcp_chunks)} chunks, median "
                    f"{statistics.median(mcp_chunks):.0f} chars"
                )
            else:
                print(f"  CLI and MCP agree: {len(cli_chunks)} chunks, "
                      f"median {statistics.median(cli_chunks):.0f} chars")

            # Genesis must still describe the collection truthfully.
            for label, g in (("CLI", cli_genesis), ("after MCP", mcp_genesis)):
                got = g.get("__meta_pack_target_bytes")
                if got != EXPECTED_PACK_TARGET:
                    failures.append(
                        f"{label}: genesis records __meta_pack_target_bytes={got}, "
                        f"expected {EXPECTED_PACK_TARGET} from "
                        f"[collections.{COLLECTION}]."
                    )
            if not failures:
                print(f"  genesis records pack_target_bytes={EXPECTED_PACK_TARGET} "
                      f"before and after the MCP ingest")

        if failures:
            print("\nFAILED:")
            for f in failures:
                print(f"  - {f}")
            return 1

        print("\nPASS: CLI and MCP build the same corpus and record the same chunking")
        return 0
    finally:
        drop(COLLECTION)
        shutil.rmtree(tmp, ignore_errors=True)


if __name__ == "__main__":
    sys.exit(main())
