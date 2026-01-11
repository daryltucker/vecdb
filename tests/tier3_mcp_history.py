
import unittest
import subprocess
import os
import json
import time
import tempfile
import shutil

class Tier3HistoryTest(unittest.TestCase):
    def setUp(self):
        self.test_dir = tempfile.mkdtemp()
        self.config_path = os.path.join(self.test_dir, "config.toml")
        self.repo_dir = os.path.join(self.test_dir, "test_repo")
        
        # 1. Setup Git Repo
        os.makedirs(self.repo_dir)
        subprocess.run(["git", "init"], cwd=self.repo_dir, check=True, stdout=subprocess.DEVNULL)
        subprocess.run(["git", "config", "user.email", "test@example.com"], cwd=self.repo_dir, check=True)
        subprocess.run(["git", "config", "user.name", "Test User"], cwd=self.repo_dir, check=True)
        
        # Commit v1
        self.v1_file = os.path.join(self.repo_dir, "logic.txt")
        with open(self.v1_file, "w") as f:
            f.write("This is version ONE of the logic.\nLegacy code here.")
        subprocess.run(["git", "add", "."], cwd=self.repo_dir, check=True)
        subprocess.run(["git", "commit", "-m", "Commit v1"], cwd=self.repo_dir, check=True, stdout=subprocess.DEVNULL)
        subprocess.run(["git", "tag", "v1.0.0"], cwd=self.repo_dir, check=True)
        
        # Commit v2
        with open(self.v1_file, "w") as f:
            f.write("This is version TWO of the logic.\nRefactored modern code.")
        subprocess.run(["git", "add", "."], cwd=self.repo_dir, check=True)
        subprocess.run(["git", "commit", "-m", "Commit v2"], cwd=self.repo_dir, check=True, stdout=subprocess.DEVNULL)
        subprocess.run(["git", "tag", "v2.0.0"], cwd=self.repo_dir, check=True)
        
        # Config
        config_content = """
[profiles.default]
qdrant_url = "http://localhost:6334"
collection_name = "tier3_history_test"
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

    def test_historic_ingestion(self):
        self._rpc("initialize")

        # 0. List Tools (Step 0)
        print("listing tools...")
        res = self._rpc("tools/list")
        self.assertNotIn("error", res)
        tools = res["result"]["tools"]
        tool_names = [t["name"] for t in tools]
        print(f"Tools found: {tool_names}")
        self.assertIn("ingest_historic_version", tool_names, "ingest_historic_version tool missing!")
        
        # 1. Ingest v1
        print("Ingesting v1.0.0...")
        res = self._rpc("tools/call", {
            "name": "ingest_historic_version",
            "arguments": {
                "repo_path": self.repo_dir,
                "git_ref": "v1.0.0",
                "collection": "history_v1"
            }
        })
        self.assertNotIn("error", res)
        
        # 2. Ingest v2
        print("Ingesting v2.0.0...")
        res = self._rpc("tools/call", {
            "name": "ingest_historic_version",
            "arguments": {
                "repo_path": self.repo_dir,
                "git_ref": "v2.0.0",
                "collection": "history_v2"
            }
        })
        self.assertNotIn("error", res)
        
        # 3. Search v1 -> Expect "Legacy"
        print("Searching v1...")
        res = self._rpc("tools/call", {
            "name": "search_vectors",
            "arguments": {
                "query": "logic version",
                "collection": "history_v1",
                "smart": False,
                "json": True
            }
        })
        content = json.loads(res["result"]["content"][0]["text"])
        self.assertTrue(any("version ONE" in c["content"] for c in content), "Should find v1 content in v1 collection")
        self.assertFalse(any("version TWO" in c["content"] for c in content), "Should NOT find v2 content in v1 collection")

        # 4. Search v2 -> Expect "Refactored"
        print("Searching v2...")
        res = self._rpc("tools/call", {
            "name": "search_vectors",
            "arguments": {
                "query": "logic version",
                "collection": "history_v2",
                "smart": False,
                "json": True
            }
        })
        content = json.loads(res["result"]["content"][0]["text"])
        self.assertTrue(any("version TWO" in c["content"] for c in content), "Should find v2 content in v2 collection")
        self.assertFalse(any("version ONE" in c["content"] for c in content), "Should NOT find v1 content in v2 collection")
        
        print("Time Travel Verified. ✅")

if __name__ == "__main__":
    unittest.main()
