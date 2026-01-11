#!/usr/bin/env python3
"""
Tier 1: Qdrant Test Instance Manager

Purpose: Ensure test Qdrant instance is running before other tests
Scope: Docker container management, health check
"""

import sys
import subprocess
import time
import urllib.request
import json

try:
    import tomllib  # Python 3.11+
except ImportError:
    try:
        import tomli as tomllib
    except ImportError:
        print("ERROR: Need Python 3.11+ or 'pip install tomli'", file=sys.stderr)
        sys.exit(1)

CONTAINER_NAME = "qdrant-test"
CONFIG_PATH = "tests/fixtures/config.toml"

def log(msg, status="INFO"):
    """Log with status"""
    prefix = f"{status}: " if status != "INFO" else ""
    print(f"{prefix}{msg}", file=sys.stderr)

def load_test_config():
    """Load test Qdrant URL from config"""
    with open(CONFIG_PATH, "rb") as f:
        config = tomllib.load(f)
    return config.get("qdrant_url", "http://localhost:6335")

def check_container_running():
    """Check if qdrant-test container exists and is running"""
    try:
        result = subprocess.run(
            ["docker", "ps", "-a", "--filter", f"name={CONTAINER_NAME}", "--format", "{{.Status}}"],
            capture_output=True,
            text=True,
            check=True
        )
        status = result.stdout.strip()
        if not status:
            return None  # Container doesn't exist
        return "Up" in status  # True if running, False if exists but stopped
    except subprocess.CalledProcessError:
        return None

def start_container():
    """Start or create the test Qdrant container"""
    status = check_container_running()
    
    if status is None:
        # Container doesn't exist - create it
        log("Creating test Qdrant container...")
        subprocess.run([
            "docker", "run", "-d",
            "-p", "6335:6333",  # HTTP port
            "-p", "6336:6334",  # gRPC port
            "--name", CONTAINER_NAME,
            "qdrant/qdrant"
        ], check=True)
        time.sleep(2)  # Give it time to start
    elif status is False:
        # Container exists but stopped - start it
        log("Starting existing test Qdrant container...")
        subprocess.run(["docker", "start", CONTAINER_NAME], check=True)
        time.sleep(2)
    else:
        log("Test Qdrant container already running")

def check_qdrant_health(url, max_retries=10):
    """Check if Qdrant is responding to health checks"""
    health_url = url.replace("6335", "6335") + "/healthz"
    
    for attempt in range(max_retries):
        try:
            response = urllib.request.urlopen(health_url, timeout=2)
            if response.status == 200:
                log(f"PASS: Qdrant healthy at {url}", "PASS")
                return True
        except Exception as e:
            if attempt < max_retries - 1:
                time.sleep(1)
            else:
                log(f"FAIL: Qdrant not responding after {max_retries} attempts: {e}", "FAIL")
                return False
    return False

def get_collections(url):
    """Get list of collections from Qdrant"""
    try:
        collections_url = url + "/collections"
        response = urllib.request.urlopen(collections_url, timeout=5)
        data = json.loads(response.read())
        collections = data.get("result", {}).get("collections", [])
        return [c["name"] for c in collections]
    except Exception as e:
        log(f"WARNING: Could not get collections: {e}")
        return []

def main():
    log("=== Tier 1: Test Qdrant Manager ===")
    
    # Load config
    test_url = load_test_config()
    log(f"Test Qdrant URL: {test_url}")
    
    # Ensure container is running
    try:
        start_container()
    except subprocess.CalledProcessError as e:
        log(f"FAIL: Could not start container: {e}", "FAIL")
        log("       Is Docker running?", "INFO")
        return False
    
    # Health check
    if not check_qdrant_health(test_url):
        return False
    
    # Show current collections
    collections = get_collections(test_url)
    if collections:
        log(f"Existing collections: {', '.join(collections)}")
    else:
        log("No existing collections (clean state)")
    
    log("✓ Test Qdrant ready", "PASS")
    return True

if __name__ == "__main__":
    sys.exit(0 if main() else 1)
