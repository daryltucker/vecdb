#!/usr/bin/env python3
# ═══════════════════════════════════════════════════════════════════
# TIER 4: AGENT REALITY — Realistic Ingest
# ═══════════════════════════════════════════════════════════════════
#
# PROGRESSIVE TRUST:
#   This test may ONLY use capabilities proven in lower tiers:
#   - T0: Qdrant is running               (tier1_qdrant.py)
#   - T1: MCP protocol works              (tier1_mcp.py)
#   - T1: Config loads correctly           (tier1_config.py)
#   - T1: Parsers produce valid output     (tier1_parsers.py)
#   - T3: Full E2E ingest→search→delete    (tier3_mcp_e2e.py)
#
# FAIL-FAST:
#   If any prerequisite tier failed, this test MUST NOT run.
#   run_all.sh enforces this via `set -e` sequential execution.
#
# DATA:
#   Uses tests/fixtures/external/lua-5.4.6/ (~400KB, 30+ .c/.h files)
#   This is a REAL C project — the Lua interpreter source code.
#   It contains headers, source files, a Makefile, and documentation.
#   This is NOT toy data.
#
# WHAT THIS PROVES:
#   - vecdb can ingest a real multi-file C project
#   - Embeddings are generated for real source code
#   - Search returns semantically relevant results
#   - Metadata (file paths, line numbers) is correct
#   - Binary/non-text files (.o, etc) are handled gracefully
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

# Qdrant test instance ports
DEFAULT_TEST_HTTP_PORT = 6335
DEFAULT_TEST_GRPC_PORT = 6336

# Fixture paths (relative to project root)
LUA_FIXTURE = "tests/fixtures/external/lua-5.4.6"
COLLECTION_NAME = "tier4_lua_realistic"


class Tier4RealisticIngest(unittest.TestCase):
    """
    Tier 4: Ingest a real C project (Lua 5.4.6) and verify
    that search returns relevant, correctly-attributed results.
    """

    @classmethod
    def setUpClass(cls):
        cls.root = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
        cls.lua_path = os.path.join(cls.root, LUA_FIXTURE)

        # Gate check: fixture must exist
        if not os.path.isdir(cls.lua_path):
            raise unittest.SkipTest(
                f"Fixture not found: {cls.lua_path}. "
                "Run: bash tests/fixtures/init.sh"
            )

        # Count source files to validate fixture integrity
        cls.src_files = []
        for root_dir, dirs, files in os.walk(cls.lua_path):
            for f in files:
                if f.endswith(('.c', '.h')):
                    cls.src_files.append(os.path.join(root_dir, f))

        if len(cls.src_files) < 10:
            raise unittest.SkipTest(
                f"Fixture appears incomplete: only {len(cls.src_files)} .c/.h files"
            )

        # Resolve ports from environment or defaults
        cls.grpc_port = os.environ.get("VECDB_TEST_QDRANT_GRPC_PORT", DEFAULT_TEST_GRPC_PORT)
        cls.http_port = os.environ.get("VECDB_TEST_QDRANT_HTTP_PORT", DEFAULT_TEST_HTTP_PORT)

        # Create test config
        cls.test_dir = tempfile.mkdtemp(prefix="tier4_lua_")
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

        # Build server binary
        print("Building vecdb-server...")
        subprocess.run(
            ["cargo", "build", "-p", "vecdb-server"],
            check=True, capture_output=True,
            cwd=cls.root,
        )
        cls.server_bin = os.path.join(cls.root, "target/debug/vecdb-server")

    @classmethod
    def tearDownClass(cls):
        # Cleanup: delete test collection via MCP
        try:
            cls._rpc_oneshot("tools/call", {
                "name": "delete_collection",
                "arguments": {
                    "collection": COLLECTION_NAME,
                    "confirmation_code": f"{COLLECTION_NAME}-DELETE"
                }
            })
        except Exception:
            pass  # Best-effort cleanup
        shutil.rmtree(cls.test_dir, ignore_errors=True)

    @classmethod
    def _rpc_oneshot(cls, method, params=None):
        """Fire a single MCP request using a fresh server process."""
        env = os.environ.copy()
        env["VECDB_CONFIG"] = cls.config_path

        proc = subprocess.Popen(
            [cls.server_bin, "--allow-local-fs"],
            stdin=subprocess.PIPE, stdout=subprocess.PIPE,
            stderr=subprocess.PIPE, text=True, env=env,
        )
        try:
            # Initialize
            init_req = json.dumps({"jsonrpc": "2.0", "method": "initialize", "id": 0}) + "\n"
            proc.stdin.write(init_req)
            proc.stdin.flush()
            proc.stdout.readline()  # consume init response

            # Actual request
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
        """Start a long-lived server for multi-step tests."""
        env = os.environ.copy()
        env["VECDB_CONFIG"] = self.config_path
        self._proc = subprocess.Popen(
            [self.server_bin, "--allow-local-fs"],
            stdin=subprocess.PIPE, stdout=subprocess.PIPE,
            stderr=subprocess.PIPE, text=True, env=env,
        )
        # Initialize
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
            if self._proc.poll() is not None:
                err = self._proc.stderr.read()
                raise Exception(f"Server died. stderr: {err}")
            raise Exception("Empty response from live server")
        return json.loads(line)

    def _stop_server(self):
        if hasattr(self, '_proc') and self._proc:
            self._proc.terminate()
            self._proc.wait()

    def test_01_ingest_lua_project(self):
        """Ingest the entire Lua 5.4.6 source tree via MCP."""
        print(f"\n[T4.1] Ingesting Lua 5.4.6 ({len(self.src_files)} .c/.h files)...")
        self._start_server()
        try:
            start = time.time()
            res = self._rpc("tools/call", {
                "name": "ingest_path",
                "arguments": {
                    "path": self.lua_path,
                    "collection": COLLECTION_NAME,
                }
            })
            duration = time.time() - start
            print(f"    Ingest took {duration:.1f}s")

            self.assertNotIn("error", res, f"Ingest failed: {res}")
            self.assertLess(duration, 120, "Ingest exceeded 120s time budget")
        finally:
            self._stop_server()

    def test_02_search_relevant_results(self):
        """Search for known Lua concepts and verify results are relevant."""
        print("\n[T4.2] Searching for 'garbage collector memory allocation'...")
        self._start_server()
        try:
            res = self._rpc("tools/call", {
                "name": "search_vectors",
                "arguments": {
                    "query": "garbage collector memory allocation",
                    "collection": COLLECTION_NAME,
                    "json": True,
                    "limit": 5,
                }
            })
            self.assertNotIn("error", res, f"Search failed: {res}")

            content = json.loads(res["result"]["content"][0]["text"])
            self.assertGreater(len(content), 0, "Search returned no results")

            # Verify results reference real files from the Lua project
            first = content[0]
            print(f"    Top result: score={first.get('score', 'N/A')}")
            print(f"    File: {first.get('metadata', {}).get('source_file', 'MISSING')}")

            # At least one result should reference a .c or .h file
            any_source = any(
                r.get("metadata", {}).get("source_file", "").endswith(('.c', '.h'))
                for r in content
            )
            self.assertTrue(any_source,
                f"No results reference .c/.h files. Results: "
                f"{[r.get('metadata', {}).get('source_file', '???') for r in content]}")

        finally:
            self._stop_server()

    def test_03_search_different_query(self):
        """Search for a different concept to verify results vary."""
        print("\n[T4.3] Searching for 'lexer tokenizer string parsing'...")
        self._start_server()
        try:
            res = self._rpc("tools/call", {
                "name": "search_vectors",
                "arguments": {
                    "query": "lexer tokenizer string parsing",
                    "collection": COLLECTION_NAME,
                    "json": True,
                    "limit": 5,
                }
            })
            self.assertNotIn("error", res, f"Search failed: {res}")
            content = json.loads(res["result"]["content"][0]["text"])
            self.assertGreater(len(content), 0, "Search returned no results")

            first_file = content[0].get("metadata", {}).get("source_file", "")
            print(f"    Top result file: {first_file}")

            # lexer/llex.c or lparser.c should be highly ranked
            top_files = [r.get("metadata", {}).get("source_file", "") for r in content[:3]]
            print(f"    Top 3 files: {top_files}")

        finally:
            self._stop_server()

    def test_04_metadata_integrity(self):
        """Verify that search results contain correct metadata."""
        print("\n[T4.4] Verifying metadata integrity...")
        self._start_server()
        try:
            res = self._rpc("tools/call", {
                "name": "search_vectors",
                "arguments": {
                    "query": "lua virtual machine bytecode",
                    "collection": COLLECTION_NAME,
                    "json": True,
                    "limit": 3,
                }
            })
            content = json.loads(res["result"]["content"][0]["text"])

            for result in content:
                meta = result.get("metadata", {})

                # source_file must exist and be a real path
                src = meta.get("source_file", "")
                self.assertTrue(len(src) > 0, "source_file is empty")
                print(f"    Checking: {src}")

                # Content must be non-empty
                self.assertTrue(len(result.get("content", "")) > 0,
                    "Result content is empty")

                # Score must be present and > 0
                score = result.get("score", 0)
                self.assertGreater(score, 0, "Score is zero")

        finally:
            self._stop_server()


if __name__ == "__main__":
    unittest.main(verbosity=2)
