
import unittest
import subprocess
import os
import json
import time
import tempfile
import shutil

class InstalledBinaryTest(unittest.TestCase):
    def setUp(self):
        self.test_dir = tempfile.mkdtemp()
        self.config_path = os.path.join(self.test_dir, "config.toml")
        
        # Point to mocked qdrant just so it starts
        config_content = """
[profiles.default]
qdrant_url = "http://localhost:6334"
collection_name = "install_verify_test"
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
        
        # TARGET THE INSTALLED BINARY
        self.server_bin = os.path.expanduser("~/.cargo/bin/vecdb-server")
        
        if not os.path.exists(self.server_bin):
            raise Exception(f"Binary not found at {self.server_bin}. install.sh failed?")
            
        print(f"Testing binary at: {self.server_bin}")
        
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

    def test_resources_list(self):
        # 1. Initialize
        self._rpc("initialize")
        
        # 2. List Resources
        print("Calling resources/list...")
        res = self._rpc("resources/list")
        if "error" in res:
             self.fail(f"Method not found in INSTALLED binary! {res['error']}")
        
        resources = res["result"]["resources"]
        print(f"Resources found: {len(resources)}")
        
        # Check for Manual
        manual_res = next((r for r in resources if r["uri"] == "vecdb://manual"), None)
        self.assertIsNotNone(manual_res, "Manual resource missing from INSTALLED binary")
        print("vecdb://manual found! Binary is updated. ✅")

if __name__ == "__main__":
    unittest.main()
