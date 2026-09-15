#!/usr/bin/env python3
import os
import sys
import subprocess
import time
import psutil

import sys, os as _os
sys.path.insert(0, _os.path.dirname(_os.path.abspath(__file__)))
from paths import find_bin

def log(msg):
    print(f"[MiniTest] {msg}")

def main():
    cli_bin = find_bin("vecdb")
    lua_file = "tests/fixtures/external/lua-5.4.6/src/lapi.c"
    
    if not os.path.exists(lua_file):
        print(f"File not found: {lua_file}")
        sys.exit(1)

    log(f"Ingesting: {lua_file}")
    
    env = os.environ.copy()
    # Use a dummy config dir
    import tempfile
    tmp_config = tempfile.mkdtemp()
    env["XDG_CONFIG_HOME"] = tmp_config
    
    # Initialize
    subprocess.run([cli_bin, "init"], env=env, check=True)

    # Pin the endpoint. `init` writes a config with no explicit URL, so without
    # this the ingest below lands on the DEFAULT endpoint — production. This is
    # an ad-hoc profiling script rather than a gate test, so nothing else stops
    # it. Same failure that put a `test_` collection into production from T3.6.
    test_url = os.environ.get("VECDB_TEST_QDRANT_URL", "http://localhost:6336")
    cfg_file = os.path.join(tmp_config, "vecdb", "config.toml")
    with open(cfg_file) as f:
        cfg = f.read()
    assert "[profiles.default]" in cfg, "generated config has no [profiles.default] to pin"
    with open(cfg_file, "w") as f:
        f.write(cfg.replace("[profiles.default]",
                            f'[profiles.default]\nqdrant_url = "{test_url}"', 1))
    
    start_time = time.time()
    # Run ingest in background to monitor it
    proc = subprocess.Popen([cli_bin, "ingest", lua_file, "--collection", "test_mini_lua"], env=env)
    
    p = psutil.Process(proc.pid)
    max_rss = 0
    while proc.poll() is None:
        try:
            rss = p.memory_info().rss / (1024 * 1024)
            max_rss = max(max_rss, rss)
            if rss > 500: # 500MB is way too much for one C file
                log(f"ALERT: Memory reached {rss:.2f} MB!")
            time.sleep(0.1)
        except psutil.NoSuchProcess:
            break
            
    duration = time.time() - start_time
    log(f"Ingestion finished in {duration:.2f}s")
    log(f"Max RSS: {max_rss:.2f} MB")
    
    if max_rss > 500:
        log("❌ FAILED: Memory leaked during single file ingestion")
        sys.exit(1)
    else:
        log("✅ PASSED: Memory usage within limits")

if __name__ == "__main__":
    main()
