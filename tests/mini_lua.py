#!/usr/bin/env python3
import os
import sys
import subprocess
import time
import psutil

def log(msg):
    print(f"[MiniTest] {msg}")

def main():
    cli_bin = "target/release/vecdb"
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
    
    start_time = time.time()
    # Run ingest in background to monitor it
    proc = subprocess.Popen([cli_bin, "ingest", lua_file, "--collection", "mini_test"], env=env)
    
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
