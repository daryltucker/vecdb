
import unittest
import subprocess
import os
import json
import time
import tempfile
import shutil

# Default ports if running standalone
DEFAULT_TEST_HTTP_PORT = 6433
DEFAULT_TEST_GRPC_PORT = 6434
CONTAINER_NAME = "vecdb-qdrant-test"

class Tier3MCPTest(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        """Start isolated Qdrant container if not provided by environment"""
        # Check if running under test_runner.sh
        if "VECDB_TEST_QDRANT_GRPC_PORT" in os.environ:
            print("Running in MANAGED mode (Shared Infrastructure).")
            cls.managed = False
            cls.grpc_port = os.environ["VECDB_TEST_QDRANT_GRPC_PORT"]
            cls.http_port = os.environ["VECDB_TEST_QDRANT_HTTP_PORT"]
            # We assume infrastructure is healthy, but we can double check
            cls.wait_for_qdrant()
        else:
            print("Running in STANDALONE mode (Managing Docker).")
            cls.managed = True
            cls.grpc_port = DEFAULT_TEST_GRPC_PORT
            cls.http_port = DEFAULT_TEST_HTTP_PORT
            
            # 1. Cleanup old
            subprocess.run(["docker", "rm", "-f", CONTAINER_NAME], stderr=subprocess.DEVNULL)
            
            # 2. Start
            print(f"Starting isolated Qdrant '{CONTAINER_NAME}' on {cls.http_port}/{cls.grpc_port}...")
            subprocess.run([
                "docker", "run", "-d",
                "--name", CONTAINER_NAME,
                "-p", f"{cls.http_port}:6333",
                "-p", f"{cls.grpc_port}:6334",
                "qdrant/qdrant:v1.16.0"
            ], check=True, stdout=subprocess.DEVNULL)
            
            # 3. Wait
            cls.wait_for_qdrant()

    @classmethod
    def tearDownClass(cls):
        """Cleanup only if we started it"""
        if cls.managed:
            print(f"Stopping isolated Qdrant container '{CONTAINER_NAME}'...")
            subprocess.run(["docker", "rm", "-f", CONTAINER_NAME], stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)

    @classmethod
    def wait_for_qdrant(cls):
        """Poll Qdrant health endpoint until ready"""
        url = f"http://localhost:{cls.http_port}/healthz"
        for _ in range(20): # 10 seconds timeout
            try:
                # Use curl to check health
                res = subprocess.run(["curl", "-s", "-f", url], stdout=subprocess.DEVNULL)
                if res.returncode == 0:
                    time.sleep(1) # Give gRPC a moment
                    return
            except:
                pass
            time.sleep(0.5)
        # If managed, don't crash, just warn? No, test relies on it.
        raise RuntimeError(f"Qdrant at {url} not ready.")

    def setUp(self):
        # 1. Create Temp Config
        self.test_dir = tempfile.mkdtemp()
        self.config_path = os.path.join(self.test_dir, "config.toml")
        
        # Create a tiny doc for ingestion
        self.doc_dir = os.path.join(self.test_dir, "docs")
        os.makedirs(self.doc_dir)
        with open(os.path.join(self.doc_dir, "hello.txt"), "w") as f:
            f.write("Integration Testing with Vector Database is fun.")

        # Use Class-level ports
        config_content = f"""
[profiles.default]
qdrant_url = "http://localhost:{self.grpc_port}"
collection_name = "tier3_test"
ollama_url = "http://localhost:11434"
embedding_model = "nomic-embed-text"
embedder_type = "local"
accept_invalid_certs = true
"""
        with open(self.config_path, "w") as f:
            f.write(config_content)
            
        # 2. Compile Server (Usually already built by runner, but safe to check)
        # In managed mode, maybe skip build? 
        # But 'cargo build' is idempotent.
        subprocess.run(["cargo", "build", "-p", "vecdb-server"], check=True, capture_output=True)
        self.server_bin = "./target/debug/vecdb-server"
        
        # 3. Start Server with ISOLATED CONFIG
        self.env = os.environ.copy()
        self.env["VECDB_CONFIG"] = self.config_path
        self.process = subprocess.Popen(
            [self.server_bin, "--allow-local-fs"],
            stdin=subprocess.PIPE,
            stdout=subprocess.PIPE,
            stderr=subprocess.PIPE,
            text=True,
            env=self.env
        )
        time.sleep(1) # Wait for process init

    def tearDown(self):
        if self.process:
            self.process.terminate()
            try:
                self.process.communicate(timeout=2)
            except:
                self.process.kill()
        shutil.rmtree(self.test_dir)

    def _rpc(self, method, params=None, req_id=1):
        req = {
            "jsonrpc": "2.0",
            "method": method,
            "id": req_id
        }
        if params:
            req["params"] = params
            
        json_line = json.dumps(req) + "\n"
        try:
            self.process.stdin.write(json_line)
            self.process.stdin.flush()
        except BrokenPipeError:
            err = self.process.stderr.read()
            raise Exception(f"Server Process Exited with Broken Pipe. Stderr: {err}")
            
        # Read with timeout safety mechanism could be added here
        # For now, blocking read is okay for tests
        response_line = self.process.stdout.readline()
        
        # DEBUG LOGGING (Can be verbose, maybe suppress if clean run desired?)
        # print(f"DEBUG: RPC Response for {method}: {response_line!r}")

        if not response_line:
             # Don't block blindly. Poll.
             if self.process.poll() is not None:
                 err = self.process.stderr.read()
                 raise Exception(f"Server Process Exited. Stderr: {err}")
             else:
                 raise Exception("Server returned empty line but process is alive (Stdout closed?)")
             
        try:
            return json.loads(response_line)
        except json.JSONDecodeError:
            # Avoid blocking read found in previous runs
            raise Exception(f"Failed to parse JSON: {response_line!r}")

    def test_full_flow_ingest_search_delete(self):
        self._rpc("initialize")
        
        # 1. Ingest
        # print("Tier 3: Ingesting...")
        res = self._rpc("tools/call", {
            "name": "ingest_path",
            "arguments": {
                "path": self.doc_dir,
                "collection": "flow_test"
            }
        })
        self.assertNotIn("error", res, f"Ingest failed: {res.get('error')}")
        
        # 2. Search (Assert we find data)
        # print("Tier 3: Searching...")
        res = self._rpc("tools/call", {
            "name": "search_vectors",
            "arguments": {
                "query": "fun",
                "collection": "flow_test",
                "smart": False,
                "json": True
            }
        })
        self.assertNotIn("error", res)
        if "content" not in res["result"]:
             self.fail(f"Search result missing content: {res}")
        content = json.loads(res["result"]["content"][0]["text"])
        self.assertTrue(len(content) > 0, "Should have found the document")
        self.assertIn("Integration Testing", content[0]["content"])
        
        # 3. Delete (Safety Check)
        # print("Tier 3: Deleting...")
        # First call fails (lock)
        res = self._rpc("tools/call", {
            "name": "delete_collection",
            "arguments": {
                "collection": "flow_test"
            }
        })
        self.assertIn("error", res["result"] if "result" in res else res)
        # self.assertIn("SAFETY LOCK ACTIVE", str(res))

        # Second call succeeds
        res = self._rpc("tools/call", {
            "name": "delete_collection",
            "arguments": {
                "collection": "flow_test",
                "confirmation_code": "flow_test-DELETE"
            }
        })
        self.assertNotIn("error", res)
        self.assertEqual(res["result"]["status"], "success")
        
    def test_initialize(self):
        res = self._rpc("initialize")
        self.assertEqual(res["result"]["serverInfo"]["name"], "vecdb-mcp")

    def test_list_tools(self):
        self._rpc("initialize")
        res = self._rpc("tools/list")
        if "result" not in res:
             self.fail(f"Tool list failed: {res}")
        tools = res["result"]["tools"]
        names = [t["name"] for t in tools]
        for t in ["search_vectors", "ingest_path", "delete_collection", "embed"]:
            self.assertIn(t, names)

    def test_list_collections(self):
        self._rpc("initialize")
        res = self._rpc("tools/call", {
            "name": "list_collections",
            "arguments": {}
        })
        self.assertIn("content", res["result"])

if __name__ == "__main__":
    unittest.main()
