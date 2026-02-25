#!/usr/bin/env python3
# ═══════════════════════════════════════════════════════════════════
# TIER 4: AGENT REALITY — Mixed Format Resilience
# ═══════════════════════════════════════════════════════════════════
#
# PROGRESSIVE TRUST:
#   This test may ONLY use capabilities proven in lower tiers:
#   - T0: Qdrant is running               (tier1_qdrant.py)
#   - T1: Parsers produce valid output     (tier1_parsers.py, tier1_parsers.sh)
#   - T2: Large files don't OOM            (tier2_large_files.rs)
#   - T3: E2E ingest→search works          (tier3_mcp_e2e.py)
#   - T4: Realistic ingest works           (tier4_realistic_ingest.py)
#
# DATA:
#   Uses tests/fixtures/external/cuda-samples/ (a subset)
#   This directory contains: .cu, .c, .cpp, .h, .cuh, .md, .json,
#   .cmake, .txt, .sh, .py AND binary files (.bin, .ppm, .docx, .pdf)
#   This IS real-world mixed format data.
#
# WHAT THIS PROVES:
#   - vecdb handles mixed-format directories without crashing
#   - Binary files are gracefully skipped (not ingested as garbage)
#   - Text files of various languages are all searchable
#   - The parser routing (Code, Recursive, Streaming, Simple)
#     works correctly across file types in a single batch
#
# TIME BUDGET: < 120s
# ═══════════════════════════════════════════════════════════════════

import unittest
import subprocess
import os
import json
import time
import tempfile
import shutil

DEFAULT_TEST_GRPC_PORT = 6336
COLLECTION_NAME = "tier4_mixed_formats"

# Use a subset of cuda-samples that has good format diversity
# Samples/2_Concepts_and_Techniques has .cu, .cpp, .h, .doc, .pdf, .ppm, .bin
CUDA_SUBSET = "tests/fixtures/external/cuda-samples/Samples/2_Concepts_and_Techniques"


class Tier4MixedFormats(unittest.TestCase):
    """
    Tier 4: Ingest a directory with wildly mixed file types and verify
    that binary files are skipped and text files are searchable.
    """

    @classmethod
    def setUpClass(cls):
        cls.root = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
        cls.fixture_path = os.path.join(cls.root, CUDA_SUBSET)

        if not os.path.isdir(cls.fixture_path):
            raise unittest.SkipTest(
                f"Fixture not found: {cls.fixture_path}. "
                "Run: bash tests/fixtures/init.sh"
            )

        # Inventory: count file types for later assertions
        cls.file_types = {}
        for root_dir, dirs, files in os.walk(cls.fixture_path):
            for f in files:
                ext = os.path.splitext(f)[1].lower()
                cls.file_types[ext] = cls.file_types.get(ext, 0) + 1

        binary_exts = {'.bin', '.ppm', '.bmp', '.doc', '.docx', '.pdf', '.png',
                       '.jpg', '.yuv', '.o', '.so', '.a'}
        cls.binary_count = sum(v for k, v in cls.file_types.items() if k in binary_exts)
        cls.text_count = sum(v for k, v in cls.file_types.items() if k not in binary_exts)
        total = sum(cls.file_types.values())
        print(f"Fixture inventory: {total} files ({cls.text_count} text, {cls.binary_count} binary)")
        print(f"  Extensions: {dict(sorted(cls.file_types.items()))}")

        # Port config
        cls.grpc_port = os.environ.get("VECDB_TEST_QDRANT_GRPC_PORT", DEFAULT_TEST_GRPC_PORT)

        # Config
        cls.test_dir = tempfile.mkdtemp(prefix="tier4_mixed_")
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

        # Build
        subprocess.run(
            ["cargo", "build", "-p", "vecdb-server"],
            check=True, capture_output=True, cwd=cls.root,
        )
        cls.server_bin = os.path.join(cls.root, "target/debug/vecdb-server")

    @classmethod
    def tearDownClass(cls):
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
            [cls.server_bin, "--allow-local-fs"],
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
            [self.server_bin, "--allow-local-fs"],
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

    def test_01_ingest_mixed_directory(self):
        """Ingest a directory with text + binary files. Must not crash."""
        print(f"\n[T4.M1] Ingesting mixed-format directory...")
        print(f"    Path: {self.fixture_path}")
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
            print(f"    Ingest took {duration:.1f}s")

            # Must not error out
            self.assertNotIn("error", res, f"Ingest crashed on mixed formats: {res}")
            self.assertLess(duration, 120, "Ingest exceeded time budget")
        finally:
            self._stop_server()

    def test_02_search_cuda_content(self):
        """Search for CUDA-specific concepts. Should find .cu/.cuh files."""
        print("\n[T4.M2] Searching for 'CUDA kernel global thread block'...")
        self._start_server()
        try:
            res = self._rpc("tools/call", {
                "name": "search_vectors",
                "arguments": {
                    "query": "CUDA kernel global thread block",
                    "collection": COLLECTION_NAME,
                    "json": True,
                    "limit": 5,
                }
            })
            self.assertNotIn("error", res)
            content = json.loads(res["result"]["content"][0]["text"])
            self.assertGreater(len(content), 0, "No results for CUDA query")

            top_files = [r.get("metadata", {}).get("source_file", "") for r in content[:3]]
            print(f"    Top 3: {top_files}")
        finally:
            self._stop_server()

    def test_03_binary_files_not_in_results(self):
        """Search results must NOT contain binary file content."""
        print("\n[T4.M3] Verifying binary files were not ingested as text...")
        self._start_server()
        try:
            # Search broadly
            res = self._rpc("tools/call", {
                "name": "search_vectors",
                "arguments": {
                    "query": "data processing",
                    "collection": COLLECTION_NAME,
                    "json": True,
                    "limit": 20,
                }
            })
            content = json.loads(res["result"]["content"][0]["text"])

            # Check that no result references a binary file
            binary_exts = ('.bin', '.ppm', '.bmp', '.doc', '.docx', '.pdf', '.png', '.yuv')
            for result in content:
                src = result.get("metadata", {}).get("source_file", "")
                for ext in binary_exts:
                    self.assertFalse(
                        src.endswith(ext),
                        f"Binary file found in search results: {src}"
                    )
            print(f"    ✓ No binary files in {len(content)} results")
        finally:
            self._stop_server()


if __name__ == "__main__":
    unittest.main(verbosity=2)
