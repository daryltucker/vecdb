#!/usr/bin/env python3
import json
import subprocess
import os
import sys

# Setup test artifacts
ARTIFACT_BASE = "/tmp/test_artifact.md"
SNAP_0 = f"{ARTIFACT_BASE}.resolved.0"
SNAP_1 = f"{ARTIFACT_BASE}.resolved.1"

CONTENT_0 = """# Task
- [ ] Item 1
- [ ] Item 2
"""

CONTENT_1 = """# Task
- [x] Item 1
- [ ] Item 2
"""

def setup():
    with open(SNAP_0, "w") as f:
        f.write(CONTENT_0)
    with open(SNAP_1, "w") as f:
        f.write(CONTENT_1)

def run_test():
    # Simulate vecq --slurp output
    # Note: vecdb-asm needs the "metadata.path" to be correct
    input_data = [
        {
            "metadata": {
                "path": SNAP_0,
                "modified": "2026-01-01T00:00:00Z"
            },
            "elements": [] 
        },
        {
            "metadata": {
                "path": SNAP_1,
                "modified": "2026-01-01T01:00:00Z"
            },
            "elements": []
        }
    ]
    
    # Write input json to valid path or pass via stdin
    input_json_path = "/tmp/asm_state_input.json"
    with open(input_json_path, "w") as f:
        json.dump(input_data, f)
        
    cmd = [
        "cargo", "run", "-q", "-p", "vecdb-asm", "--",
        "--strategy", "state",
        input_json_path
    ]
    
    result = subprocess.run(cmd, capture_output=True, text=True, check=True)
    try:
        output_json = json.loads(result.stdout)
    except json.JSONDecodeError:
        print("Failed to decode JSON output")
        print("STDOUT:", result.stdout)
        sys.exit(1)
        
    # Validation
    # We expect 2 events:
    # 1. Creation (Version 0)
    # 2. Evolution (0 -> 1)
    
    if len(output_json) != 2:
        print(f"FAILED: Expected 2 events, got {len(output_json)}")
        print(json.dumps(output_json, indent=2))
        sys.exit(1)
        
    # Check Evolution Event
    evo_event = output_json[1]
    if evo_event["event_type"] != "evolution":
        print("FAILED: Second event is not 'evolution'")
        sys.exit(1)
        
    diff_summary = evo_event["diff_summary"]
    expected_diff_part = "- - [ ] Item 1"
    expected_add_part = "+ - [x] Item 1"
    
    if expected_diff_part not in diff_summary or expected_add_part not in diff_summary:
        print("FAILED: Diff summary missing expected changes")
        print("Got:", diff_summary)
        sys.exit(1)
        
    print("SUCCESS: State strategy correctly identified diffs.")

if __name__ == "__main__":
    setup()
    run_test()
