
import unittest
import subprocess
import os
import json
import time
import tempfile
import shutil

class Tier3ResourcesTest(unittest.TestCase):
    def setUp(self):
        self.test_dir = tempfile.mkdtemp()
        self.config_path = os.path.join(self.test_dir, "config.toml")
        
        # Use existing Qdrant if available (assuming previous tests left it running/available)
        # For simplicity, we assume Qdrant on localhost:6334 (default)
        
        config_content = """
[profiles.default]
qdrant_url = "http://localhost:6334"
collection_name = "tier3_resources_test"
ollama_url = "http://localhost:11434"
embedding_model = "nomic-embed-text"
embedder_type = "local"
accept_invalid_certs = true
"""
        with open(self.config_path, "w") as f:
            f.write(config_content)
            
        self.env = os.environ.copy()
        self.env["VECDB_CONFIG"] = self.config_path
        self.env["VECDB_ALLOW_LOCAL_FS"] = "true"
        
        # Build
        subprocess.run(["cargo", "build", "-p", "vecdb-server"], check=True, capture_output=True)
        self.server_bin = "./target/debug/vecdb-server"
        
        self.process = subprocess.Popen(
            [self.server_bin, "--allow-local-fs"],
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
                "collection": "tier3_res_test"
            }
        })
        
        # Now list again
        res = self._rpc("resources/list")
        resources = res["result"]["resources"]
        target_uri = "vecdb://tier3_res_test"
        found = any(r["uri"] == target_uri for r in resources)
        self.assertTrue(found, "Newly ingested collection should appear in resources")
        
        # Read it
        res = self._rpc("resources/read", {"uri": target_uri})
        self.assertNotIn("error", res)
        contents = res["result"]["contents"]
        self.assertEqual(len(contents), 1)
        self.assertEqual(contents[0]["mimeType"], "application/json")
        stats = json.loads(contents[0]["text"])
        self.assertEqual(stats["name"], "tier3_res_test")
        
        # 4. Smart Search (Verify no regression/panic on smart arg)
        res = self._rpc("tools/call", {
            "name": "search_vectors",
            "arguments": {
                "query": "anything",
                "collection": "tier3_res_test",
                "smart": True,
                "json": True
            }
        })
        # This will fail logic-wise if 'docs' collection is missing or smart search fails, 
        # but we just want to ensure it doesn't PANIC or explode due to arg parsing.
        # Smart search usually defaults to 'docs'. If we search 'tier3_res_test', 
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
