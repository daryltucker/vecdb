#!/usr/bin/env python3
"""
Tier 1: vecdbrc Warning Fix Validation

Purpose: Verify that vecdbrc warnings print ONCE, not per-route.

Bug: When .vecdbrc has multiple routes that differ from CLI collection,
the warning was printed once per route (spam).
Fix: Should print ONCE if any mismatch exists.

Test: Simulate interactive TTY, run vecdb ingest on directory with .vecdbrc
having multiple routes. Count warning occurrences.
"""

import subprocess
import sys
import os
import tempfile
import shutil
from pathlib import Path

# Use test config
TEST_CONFIG = os.environ.get("VECDB_CONFIG", "tests/fixtures/config.toml")
PROJECT_ROOT = Path(__file__).parent.parent
VECDB_BIN = os.environ.get("VECDB_BIN", str(PROJECT_ROOT / "target" / "debug" / "vecdb"))

def log(msg, status="INFO"):
    use_colors = sys.stderr.isatty()
    if use_colors:
        colors = {"PASS": "\033[32m", "FAIL": "\033[31m", "INFO": "\033[34m"}
        reset = "\033[0m"
        print(f"{colors.get(status, '')}{msg}{reset}", file=sys.stderr)
    else:
        print(f"{status}: {msg}", file=sys.stderr)

def run_interactive(cmd, cwd):
    """Run command with simulated TTY to trigger interactive mode"""
    # Use `script` to simulate TTY
    full_cmd = ["script", "-q", "-c", " ".join(cmd), "/dev/null"]
    result = subprocess.run(
        full_cmd,
        capture_output=True,
        text=True,
        cwd=cwd,
        env={**os.environ, "VECDB_CONFIG": TEST_CONFIG},
        timeout=30
    )
    return result

def test_warning_spam():
    """Test: Warning should print at most once, not per-route"""
    
    # Create temp directory with .vecdbrc having multiple routes
    with tempfile.TemporaryDirectory() as tmpdir:
        test_dir = Path(tmpdir)
        
        # Create .vecdbrc with multiple routes different from CLI collection
        vecdbrc = test_dir / ".vecdbrc"
        vecdbrc.write_text("""[default]
collection = "code-lts"

[[routes]]
glob = "*.md"
collection = "brain-lts"

[[routes]]
glob = "*.rs"
collection = "docs-lts"

[[routes]]
glob = "*.py"
collection = "code-lts"
""")
        
        # Create dummy files
        (test_dir / "test.md").write_text("# Test")
        (test_dir / "test.rs").write_text("// Test")
        (test_dir / "test.py").write_text("# Test")
        
        # Run with collection "code" (different from all routes)
        # Use simulated TTY to trigger interactive mode
        cmd = [VECDB_BIN, "ingest", str(test_dir), "-c", "code"]
        result = run_interactive(cmd, str(test_dir))
        
        # Combine stdout + stderr for checking
        output = result.stdout + result.stderr
        
        # Count warning occurrences
        warning_count = output.count("Warning: .vecdbrc routes to")
        
        if warning_count > 1:
            log(f"FAIL: Warning printed {warning_count} times (spam bug!)", "FAIL")
            # Show relevant output
            for line in output.split('\n'):
                if 'arning' in line or 'ARNING' in line:
                    log(f"  {line.strip()}", "FAIL")
            return False
        
        if warning_count == 0:
            log(f"FAIL: Warning did not print at all (is_interactive issue?)", "FAIL")
            log(f"  stdout: {result.stdout[:200]}", "INFO")
            log(f"  stderr: {result.stderr[:200]}", "INFO")
            return False
        
        log(f"PASS: Warning printed {warning_count} time(s) - no spam", "PASS")
        return True

def main():
    if not Path(TEST_CONFIG).exists():
        log(f"FAIL: Test config not found: {TEST_CONFIG}", "FAIL")
        sys.exit(1)
    
    success = test_warning_spam()
    sys.exit(0 if success else 1)

if __name__ == "__main__":
    main()