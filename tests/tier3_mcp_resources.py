
import unittest
import subprocess
import os
import json
import urllib.request
import time
import tempfile
import shutil

import sys, os as _os
sys.path.insert(0, _os.path.dirname(_os.path.abspath(__file__)))
from paths import bin_path

class Tier3ResourcesTest(unittest.TestCase):
    def setUp(self):
        self.test_dir = tempfile.mkdtemp()
        self.config_path = os.path.join(self.test_dir, "config.toml")
        
        # Use existing Qdrant if available (assuming previous tests left it running/available)
        # ALL TESTS MUST USE TEST QDRANT — NEVER PRODUCTION (6333/6334)

        config_content = """
[backend.local]
kind = "fastembed"

[embedder.default]
backend = "local"
model = "all-minilm-l6-v2"

[profiles.default]
embedder = "default"
qdrant_url = "http://localhost:6336"
collection_name = "tier3_resources_test"
accept_invalid_certs = true
"""
        with open(self.config_path, "w") as f:
            f.write(config_content)
            
        self.env = os.environ.copy()
        self.env["VECDB_CONFIG"] = self.config_path
        self.env["VECDB_ALLOW_LOCAL_FS"] = "true"
        
        # Build
        subprocess.run(["cargo", "build", "-p", "vecdb-server"], check=True, capture_output=True)
        self.server_bin = bin_path("vecdb-server")
        
        self.process = subprocess.Popen(
            [self.server_bin, "--stdio", "--allow-local-fs"],
            stdin=subprocess.PIPE,
            stdout=subprocess.PIPE,
            stderr=subprocess.PIPE,
            text=True,
            env=self.env
        )
        time.sleep(1)

    def tearDown(self):
        if self.process:
            self.process.terminate()
            try:
                self.process.communicate(timeout=1)
            except:
                self.process.kill()
        shutil.rmtree(self.test_dir)

    def _rpc(self, method, params=None):
        req = {
            "jsonrpc": "2.0",
            "method": method,
            "id": 1
        }
        if params:
            req["params"] = params
        
        self.process.stdin.write(json.dumps(req) + "\n")
        self.process.stdin.flush()
        line = self.process.stdout.readline()
        if not line:
             err = self.process.stderr.read()
             raise Exception(f"Server died: {err}")
        return json.loads(line)

    def test_resources_flow(self):
        # 1. Initialize & Check Capabilities
        res = self._rpc("initialize")
        caps = res["result"]["capabilities"]
        self.assertIn("resources", caps, "Server should declare resources capability")

        # 2. List Resources
        res = self._rpc("resources/list")
        if "error" in res:
             self.fail(f"List resources failed: {res['error']}")
        
        resources = res["result"]["resources"]
        # We assume at least 'tier3_resources_test' (default) or 'docs' exists
        self.assertIsInstance(resources, list)
        
        # Check for Manual
        manual_res = next((r for r in resources if r["uri"] == "vecdb://manual"), None)
        self.assertIsNotNone(manual_res, "Manual resource 'vecdb://manual' not found in list")
        self.assertEqual(manual_res["mimeType"], "text/markdown")
        
        # Read Manual
        res = self._rpc("resources/read", {"uri": "vecdb://manual"})
        self.assertNotIn("error", res)
        self.assertIn("# AGENT INTERFACE SPECIFICATION", res["result"]["contents"][0]["text"])

        # 3. Read Resource (Read stats of 'docs' if it exists, or create one)
        # Let's ingest something to ensure a collection exists
        self._rpc("tools/call", {
            "name": "ingest_path",
            "arguments": {
                "path": "README.md", # Assume valid path in repo
                "collection": "test_tier3_res"
            }
        })
        
        # Now list again
        res = self._rpc("resources/list")
        resources = res["result"]["resources"]
        target_uri = "vecdb://collections/test_tier3_res"
        found = any(r["uri"] == target_uri for r in resources)
        self.assertTrue(found, "Newly ingested collection should appear in resources")

        # Read it
        res = self._rpc("resources/read", {"uri": target_uri})
        self.assertNotIn("error", res)
        contents = res["result"]["contents"]
        self.assertEqual(len(contents), 1)
        self.assertEqual(contents[0]["mimeType"], "application/json")
        stats = json.loads(contents[0]["text"])
        self.assertEqual(stats["name"], "test_tier3_res")

        # `is_compatible` used to be hardcoded `true`.
        #
        # This resource is what an agent reads to decide whether it may use a
        # collection, so a constant `true` is not a missing feature — it is an
        # answer that is wrong exactly when it matters. A collection we just
        # ingested with this very embedder must report compatible AND report the
        # model it was written with; a hardcoded value cannot do the second.
        self.assertTrue(
            stats.get("is_vecdb"),
            f"a collection vecdb just wrote must be recognised as ours: {stats}",
        )
        self.assertTrue(
            stats.get("is_compatible"),
            f"a collection written by this embedder must be compatible with it: {stats}",
        )
        self.assertTrue(
            stats.get("model"),
            f"compatibility is a claim about a model, so name it: {stats}",
        )


        # The discriminating case: a collection that is NOT ours.
        #
        # Asserting only on a collection we just wrote cannot catch a hardcoded
        # `true` — for that collection `true` is the correct answer. A bare
        # collection with no genesis point is what "not a vecdb collection"
        # means, and it must come back false on both counts.
        foreign = "test_tier3_foreign"
        http = os.environ.get("VECDB_TEST_QDRANT_HTTP_URL", "http://localhost:6335")
        try:
            urllib.request.urlopen(
                urllib.request.Request(
                    f"{http}/collections/{foreign}",
                    data=json.dumps({"vectors": {"size": 384, "distance": "Cosine"}}).encode(),
                    headers={"Content-Type": "application/json"},
                    method="PUT",
                ),
                timeout=20,
            )

            res = self._rpc("resources/read", {"uri": f"vecdb://collections/{foreign}"})
            self.assertNotIn("error", res)
            stats = json.loads(res["result"]["contents"][0]["text"])

            self.assertFalse(
                stats.get("is_vecdb"),
                f"a collection with no genesis point is not ours: {stats}",
            )
            self.assertFalse(
                stats.get("is_compatible"),
                "a foreign collection must NOT report compatible — this field is "
                f"what an agent trusts before writing: {stats}",
            )
        finally:
            try:
                urllib.request.urlopen(
                    urllib.request.Request(
                        f"{http}/collections/{foreign}", method="DELETE"
                    ),
                    timeout=20,
                )
            except Exception:
                pass

        # 4. Smart Search (Verify no regression/panic on smart arg)
        res = self._rpc("tools/call", {
            "name": "search_vectors",
            "arguments": {
                "query": "anything",
                "collection": "test_tier3_res",
                "smart": True,
                "json": True
            }
        })
        # This will fail logic-wise if 'docs' collection is missing or smart search fails, 
        # but we just want to ensure it doesn't PANIC or explode due to arg parsing.
        # Smart search usually defaults to 'docs'. If we search 'test_tier3_res', 
        # using 'smart' might ignore collection? 
        # Code: if args.smart { core.search_smart(...) }
        # core.search_smart hardcodes "docs"? Or uses config?
        # Let's check result provided no error.
        
        # Wait, if smart search fails (e.g. no 'docs' collection), it sends an error.
        # That is Acceptable. We just want to ensure routing works.
        
        if "error" in res:
             print(f"Smart search error (expected if docs missing): {res['error']}")
        else:
             print("Smart search success")

if __name__ == "__main__":
    unittest.main()
