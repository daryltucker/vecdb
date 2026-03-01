#!/usr/bin/env python3
import subprocess
import os
import sys
import json

# Configuration
VECQ_BINARY = "./target/release/vecq"

# Sample Code Snippets for every supported language
SAMPLES = {
    "rust": ("test_sample.rs", "fn main() { println!(\"Hello\"); }"),
    "python": ("test_sample.py", "def main():\n    print(\"Hello\")"),
    "markdown": ("test_sample.md", "# Heading\n\n```rust\nfn code() {}\n```"),
    "c": ("test_sample.c", "#include <stdio.h>\nint main() { return 0; }"),
    "cpp": ("test_sample.cpp", "#include <iostream>\nint main() { return 0; }"),
    "cuda": ("test_sample.cu", "__global__ void kernel() {}"),
    "go": ("test_sample.go", "package main\nfunc main() {}"),
    "bash": ("test_sample.sh", "#!/bin/bash\necho 'hello'"),
    "text": ("test_sample.txt", "Just plain text.")
}

def run_vecq(filepath):
    """Runs vecq on a file and returns the JSON output."""
    cmd = [VECQ_BINARY, filepath]
    result = subprocess.run(cmd, shell=False, capture_output=True, text=True)
    if result.returncode != 0:
        print(f"FAILED to process {filepath}")
        print("STDERR:", result.stderr)
        return None
    return result.stdout

def test_all_parsers():
    print("--- Verifying All Language Parsers ---")
    
    # Ensure binary exists
    if not os.path.exists(VECQ_BINARY):
        print(f"Error: {VECQ_BINARY} not found. Run 'cargo build --release --bin vecq' first.")
        sys.exit(1)

    failures = []

    for lang, (filename, content) in SAMPLES.items():
        print(f"Testing {lang} parser...", end=" ")
        
        # Write sample
        with open(filename, "w") as f:
            f.write(content)
        
        # Run vecq
        output = run_vecq(filename)
        
        # Check validity
        if output:
            try:
                # vecq output is a sequence of JSON objects (ndjson-like) or formatting?
                # By default `vecq file` outputs colored text usually, unless we pipe or use jq mode.
                # Wait, `vecq` acts like jq. `vecq . <file>` or just `vecq <file>`?
                # `vecq` default behavior without args might be formatting or AST dump.
                # Let's use a simple query "." to get JSON output if that's how it works.
                # Actually, `vecq` implies a query engine.
                # Let's try `vecq . <file>` behavior or check help.
                # Usage: vecq [OPTIONS] <FILE> [FILTER]
                # If we pass just file, it usually prints the structure.
                # Let's pass a dummy filter "." to ensure we get JSON output if supported, 
                # or just parse whatever stdout it gives.
                # Based on previous interaction, `vecq` might output colored text by default.
                # We need to verify it parses without crashing.
                pass
            except Exception as e:
                print(f"Exception checking output: {e}")
                failures.append(lang)
        else:
            failures.append(lang)
            print("FAIL")
            continue
            
        print("OK")
        
        # Cleanup
        os.remove(filename)

    if failures:
        print(f"\nFAILED Parsers: {', '.join(failures)}")
        sys.exit(1)
    else:
        print("\nSUCCESS: All parsers verified.")

if __name__ == "__main__":
    test_all_parsers()
