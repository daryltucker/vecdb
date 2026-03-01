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
    """Test 2: Verify Qdrant URL points to test instance"""
    log("Test 2: Qdrant URL Validation")
    
    try:
        config = load_config()
        qdrant_url = config.get("qdrant_url")
        
        if not qdrant_url or not isinstance(qdrant_url, str):
            log(f"FAIL: qdrant_url missing or invalid: {qdrant_url}", "FAIL")
            return False
        
        if "6335" not in qdrant_url and "6336" not in qdrant_url:
            log(f"FAIL: Config uses production Qdrant! URL: {qdrant_url}", "FAIL")
            log("       Expected: http://localhost:6335 or 6336 (test instance)", "FAIL")
            return False
        
        log(f"PASS: Qdrant URL correct: {qdrant_url}", "PASS")
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
        
        required_fields = ["embedder_type", "embedding_model", "chunk_size"]
        
        for profile_name, profile_data in profiles.items():
            # Validate required fields
            for field in required_fields:
                if field not in profile_data or not profile_data[field]:
                    log(f"FAIL: Profile '{profile_name}' missing field: {field}", "FAIL")
                    return False
            
            # Log profile details
            embedder = profile_data["embedder_type"]
            model = profile_data["embedding_model"]
            chunks = profile_data["chunk_size"]
            log(f"  ✓ {profile_name}: {embedder}/{model} (chunks={chunks})")
        
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
