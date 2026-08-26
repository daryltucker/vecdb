#!/usr/bin/env python3
"""
Tier 1: Config Validation & Profile Testing

Purpose: Validate that test config.toml loads correctly and test all profiles

This test validates:
1. Config.toml parses without errors
2. Qdrant URL points to test instance (port 6335, not production 6334)
3. All profiles have required fields
"""

import sys
import json
import subprocess
from pathlib import Path

try:
    import tomllib  # Python 3.11+
except ImportError:
    try:
        import tomli as tomllib  # Fallback
    except ImportError:
        print("ERROR: Need Python 3.11+ or 'pip install tomli'", file=sys.stderr)
        sys.exit(1)

CONFIG_PATH = "tests/fixtures/config.toml"

def log(msg, status="INFO"):
    """Log with optional colors (only in TTY)"""
    use_colors = sys.stderr.isatty()
    
    if use_colors:
        colors = {"PASS": "\033[32m", "FAIL": "\033[31m", "INFO": "\033[34m"}
        reset = "\033[0m"
        print(f"{colors.get(status, '')}{msg}{reset}", file=sys.stderr)
    else:
        # Non-TTY: plain text
        prefix = f"{status}: " if status != "INFO" else ""
        print(f"{prefix}{msg}", file=sys.stderr)

def load_config():
    """Load the test config.toml"""
    with open(CONFIG_PATH, "rb") as f:
        return tomllib.load(f)

def run_vecq(query):
    """Run vecq query on config.toml"""
    result = subprocess.run(
        [VECQ_BIN, CONFIG_PATH, query],
        capture_output=True,
        text=True
    )
    if result.returncode != 0:
        raise Exception(f"vecq failed: {result.stderr}")
    return json.loads(result.stdout)

def test_config_loading():
    """Test 1: Config file loads without errors"""
    log("Test 1: Config Loading")
    
    if not Path(CONFIG_PATH).exists():
        log(f"FAIL: Config not found at {CONFIG_PATH}", "FAIL")
        return False
    
    try:
        config = load_config()
        profile_count = len(config.get("profiles", {}))
        log(f"PASS: Config loaded successfully ({profile_count} profiles)", "PASS")
        return True
    except Exception as e:
        log(f"FAIL: {e}", "FAIL")
        return False

def test_qdrant_url():
    """Test 2: Every profile's Qdrant URL points at the test instance.

    Previously read a top-level `qdrant_url` key. That key was never part of the
    Config schema — serde ignored it — so the test was validating dead config
    while the values that actually get used went unchecked. Profiles are where
    `qdrant_url` lives, so that is what gets verified.
    """
    log("Test 2: Qdrant URL Validation")

    try:
        config = load_config()
        profiles = config.get("profiles", {})

        if not profiles:
            log("FAIL: config defines no profiles", "FAIL")
            return False

        for name, profile in profiles.items():
            url = profile.get("qdrant_url")
            if not url or not isinstance(url, str):
                log(f"FAIL: profiles.{name} has no qdrant_url", "FAIL")
                return False
            if "6335" not in url and "6336" not in url:
                log(f"FAIL: profiles.{name} uses production Qdrant! URL: {url}", "FAIL")
                log("       Expected: http://localhost:6335 or 6336 (test instance)", "FAIL")
                return False
            log(f"PASS: profiles.{name} -> {url}", "PASS")

        return True
    except Exception as e:
        log(f"FAIL: {e}", "FAIL")
        return False


def test_all_profiles():
    """Test 3: Iterate and validate all profiles"""
    log("Test 3: Profile Validation")
    
    try:
        config = load_config()
        profiles = config.get("profiles", {})
        
        if not profiles or len(profiles) == 0:
            log("FAIL: No profiles found in config", "FAIL")
            return False
        
        log(f"Found {len(profiles)} profiles to test")
        
        # A profile names an embedder; the embedder names a backend and a model.
        # Validating the whole chain here is the point — a dangling reference is
        # the one config error the three-layer split makes possible, and it must
        # not survive to first use.
        embedders = config.get("embedder", {})
        backends = config.get("backend", {})

        for profile_name, profile_data in profiles.items():
            embedder_name = profile_data.get("embedder")
            if not embedder_name:
                log(f"FAIL: Profile '{profile_name}' names no embedder", "FAIL")
                return False
            if embedder_name not in embedders:
                log(f"FAIL: Profile '{profile_name}' -> unknown embedder '{embedder_name}'", "FAIL")
                return False

            embedder = embedders[embedder_name]
            backend_name = embedder.get("backend")
            if not backend_name or backend_name not in backends:
                log(f"FAIL: embedder '{embedder_name}' -> unknown backend '{backend_name}'", "FAIL")
                return False
            if not embedder.get("model"):
                log(f"FAIL: embedder '{embedder_name}' names no model", "FAIL")
                return False

            kind = backends[backend_name].get("kind")
            log(f"  ✓ {profile_name}: {embedder_name} = {embedder['model']} on {backend_name} ({kind})")
        
        log(f"PASS: All {len(profiles)} profiles valid", "PASS")
        return True
        
    except Exception as e:
        log(f"FAIL: {e}", "FAIL")
        return False

def main():
    log("=== Tier 1: Config Validation ===")
    
    tests = [
        test_config_loading,
        test_qdrant_url,
        test_all_profiles
    ]
    
    for test in tests:
        if not test():
            log(f"\n✗ Test suite failed at: {test.__name__}", "FAIL")
            return False
        print()  # Blank line between tests
    
    log("✓ All config tests passed", "PASS")
    return True

if __name__ == "__main__":
    sys.exit(0 if main() else 1)
