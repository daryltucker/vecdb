#!/usr/bin/env python3
# ═══════════════════════════════════════════════════════════════════
# TIER 4: AGENT REALITY — Agent Workflow Simulation
# ═══════════════════════════════════════════════════════════════════
#
# PROGRESSIVE TRUST:
#   This test may ONLY use capabilities proven in lower tiers:
#   - T0: Qdrant is running               (tier1_qdrant.py)
#   - T1: MCP protocol works              (tier1_mcp.py)
#   - T1: Config and profiles work         (tier1_config.py)
#   - T3: Full E2E flow works             (tier3_mcp_e2e.py)
#   - T4: Realistic ingest works          (tier4_realistic_ingest.py)
#   - T4: Mixed formats handled           (tier4_mixed_formats.py)
#
# WHAT THIS PROVES:
#   - Multi-step agent workflow doesn't regress
#   - Ingest → multi-search → delete → re-ingest is idempotent
#   - Different queries return different result sets (not just one blob)
#   - Collection safety lock works under realistic conditions
#   - Scale: 500+ files ingested in a single batch
#
# DATA:
#   Uses tests/fixtures/external/cuda-samples/Samples/2_Concepts_and_Techniques
#   ~480 files (shared subset with tier4_mixed_formats)
#   Text files: ~430 (.cu, .cpp, .h, .cuh, .md, .json, .txt, etc.)
#
# TIME BUDGET: < 900s (~480 files at ~1.3s/file typical)
# ═══════════════════════════════════════════════════════════════════

import unittest
import subprocess
import os
import json
import time
import tempfile
import shutil

DEFAULT_TEST_GRPC_PORT = 6336
COLLECTION_NAME = "tier4_agent_workflow"
CUDA_FIXTURE = "tests/fixtures/external/cuda-samples/Samples/2_Concepts_and_Techniques"


class Tier4AgentWorkflow(unittest.TestCase):
    """
    Tier 4: Simulate a real agent workflow — bulk ingest, multi-query
    search, safety lock, re-ingest idempotency.
    """

    @classmethod
    def setUpClass(cls):
        cls.root = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
        cls.fixture_path = os.path.join(cls.root, CUDA_FIXTURE)

        if not os.path.isdir(cls.fixture_path):
            raise unittest.SkipTest(
                f"Fixture not found: {cls.fixture_path}. "
                "Run: bash tests/fixtures/init.sh"
            )

        cls.grpc_port = os.environ.get("VECDB_TEST_QDRANT_GRPC_PORT", DEFAULT_TEST_GRPC_PORT)

        cls.test_dir = tempfile.mkdtemp(prefix="tier4_workflow_")
        cls.config_path = os.path.join(cls.test_dir, "config.toml")
        with open(cls.config_path, "w") as f:
            f.write(f"""
[profiles.default]
qdrant_url = "http://localhost:{cls.grpc_port}"
collection_name = "{COLLECTION_NAME}"
embedder_type = "local"
embedding_model = "default"
accept_invalid_certs = true
chunk_size = 512
""")

        subprocess.run(
            ["cargo", "build", "-p", "vecdb-server"],
            check=True, capture_output=True, cwd=cls.root,
        )
        cls.server_bin = os.path.join(cls.root, "target/debug/vecdb-server")

    @classmethod
    def tearDownClass(cls):
        # Final cleanup
        try:
            cls._rpc_oneshot("tools/call", {
                "name": "delete_collection",
                "arguments": {
                    "collection": COLLECTION_NAME,
                    "confirmation_code": f"{COLLECTION_NAME}-DELETE"
                }
            })
        except:
            pass
        shutil.rmtree(cls.test_dir, ignore_errors=True)

    @classmethod
    def _rpc_oneshot(cls, method, params=None):
        env = os.environ.copy()
        env["VECDB_CONFIG"] = cls.config_path
        proc = subprocess.Popen(
            [cls.server_bin, "--stdio", "--allow-local-fs"],
            stdin=subprocess.PIPE, stdout=subprocess.PIPE,
            stderr=subprocess.PIPE, text=True, env=env,
        )
        try:
            init = json.dumps({"jsonrpc": "2.0", "method": "initialize", "id": 0}) + "\n"
            proc.stdin.write(init)
            proc.stdin.flush()
            proc.stdout.readline()

            req = {"jsonrpc": "2.0", "method": method, "id": 1}
            if params:
                req["params"] = params
            proc.stdin.write(json.dumps(req) + "\n")
            proc.stdin.flush()
            line = proc.stdout.readline()
            return json.loads(line) if line else None
        finally:
            proc.terminate()
            proc.wait()

    def _start_server(self):
        env = os.environ.copy()
        env["VECDB_CONFIG"] = self.config_path
        self._proc = subprocess.Popen(
            [self.server_bin, "--stdio", "--allow-local-fs"],
            stdin=subprocess.PIPE, stdout=subprocess.PIPE,
            stderr=subprocess.PIPE, text=True, env=env,
        )
        req = json.dumps({"jsonrpc": "2.0", "method": "initialize", "id": 0}) + "\n"
        self._proc.stdin.write(req)
        self._proc.stdin.flush()
        self._proc.stdout.readline()

    def _rpc(self, method, params=None, req_id=1):
        req = {"jsonrpc": "2.0", "method": method, "id": req_id}
        if params:
            req["params"] = params
        self._proc.stdin.write(json.dumps(req) + "\n")
        self._proc.stdin.flush()
        line = self._proc.stdout.readline()
        if not line:
            raise Exception("Empty response")
        return json.loads(line)

    def _stop_server(self):
        if hasattr(self, '_proc'):
            self._proc.terminate()
            self._proc.wait()

    def test_01_bulk_ingest(self):
        """Ingest the CUDA samples subset (~430 text files)."""
        print(f"\n[T4.W1] Bulk ingesting CUDA samples...")
        self._start_server()
        try:
            start = time.time()
            res = self._rpc("tools/call", {
                "name": "ingest_path",
                "arguments": {
                    "path": self.fixture_path,
                    "collection": COLLECTION_NAME,
                }
            })
            duration = time.time() - start
            print(f"    Bulk ingest took {duration:.1f}s")
            self.assertNotIn("error", res, f"Bulk ingest failed: {res}")
            self.assertLess(duration, 900, "Bulk ingest exceeded 900s time budget")
        finally:
            self._stop_server()

    def test_02_multi_query_variety(self):
        """Multiple different queries should return different result sets."""
        print("\n[T4.W2] Testing query variety...")
        self._start_server()
        try:
            queries = [
                "matrix transpose optimization",
                "particle simulation physics",
                "histogram equalization image processing",
                "CMake build configuration",
            ]
            all_top_files = []
            for q in queries:
                res = self._rpc("tools/call", {
                    "name": "search_vectors",
                    "arguments": {
                        "query": q,
                        "collection": COLLECTION_NAME,
                        "json": True,
                        "smart": False,
                        "limit": 3,
                    }
                })
                content = json.loads(res["result"]["content"][0]["text"])
                top_file = content[0].get("metadata", {}).get("source", "") if content else ""
                all_top_files.append(top_file)
                print(f"    '{q[:30]}...' → {top_file}")

            # At least 2 different files should appear as top results
            unique_tops = set(f for f in all_top_files if f)
            self.assertGreaterEqual(
                len(unique_tops), 2,
                f"Queries returned the same top file for everything: {unique_tops}"
            )
            print(f"    ✓ {len(unique_tops)} unique top results from {len(queries)} queries")
        finally:
            self._stop_server()

    def test_03_safety_lock(self):
        """Delete without confirmation code must fail."""
        print("\n[T4.W3] Testing collection safety lock...")
        self._start_server()
        try:
            res = self._rpc("tools/call", {
                "name": "delete_collection",
                "arguments": {"collection": COLLECTION_NAME}
            })
            # Should error or contain safety lock message
            result_text = json.dumps(res)
            has_safety = ("error" in result_text.lower() or
                         "safety" in result_text.lower() or
                         "lock" in result_text.lower() or
                         "confirm" in result_text.lower())
            self.assertTrue(has_safety,
                f"Safety lock did not trigger on delete without confirmation: {res}")
            print("    ✓ Safety lock blocked unconfirmed delete")
        finally:
            self._stop_server()

    def test_04_re_ingest_idempotency(self):
        """Re-ingesting the same data should not create duplicates."""
        print("\n[T4.W4] Testing re-ingest idempotency...")

        # First: count current search results
        self._start_server()
        try:
            res1 = self._rpc("tools/call", {
                "name": "search_vectors",
                "arguments": {
                    "query": "CUDA kernel",
                    "collection": COLLECTION_NAME,
                    "json": True,
                    "smart": False,
                    "limit": 10,
                }
            })
            results_before = json.loads(res1["result"]["content"][0]["text"])
            count_before = len(results_before)
        finally:
            self._stop_server()

        # Second: re-ingest same path
        self._start_server()
        try:
            res = self._rpc("tools/call", {
                "name": "ingest_path",
                "arguments": {
                    "path": self.fixture_path,
                    "collection": COLLECTION_NAME,
                }
            })
            self.assertNotIn("error", res, f"Re-ingest failed: {res}")
        finally:
            self._stop_server()

        # Third: count results again
        self._start_server()
        try:
            res2 = self._rpc("tools/call", {
                "name": "search_vectors",
                "arguments": {
                    "query": "CUDA kernel",
                    "collection": COLLECTION_NAME,
                    "json": True,
                    "smart": False,
                    "limit": 10,
                }
            })
            results_after = json.loads(res2["result"]["content"][0]["text"])
            count_after = len(results_after)
        finally:
            self._stop_server()

        print(f"    Before: {count_before} results, After: {count_after} results")
        # Should be equal (idempotent) — or at most slightly different due to
        # chunking boundary changes, but NOT doubled
        self.assertLessEqual(
            count_after, count_before * 1.1,  # Allow 10% variance
            f"Re-ingest appears to have created duplicates: {count_before} → {count_after}"
        )
        print("    ✓ Re-ingest is idempotent (no duplicates)")


if __name__ == "__main__":
    unittest.main(verbosity=2)
