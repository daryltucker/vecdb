import json
import subprocess
import os
import sys

def run_asm(input_data):
    process = subprocess.Popen(
        ['cargo', 'run', '-p', 'vecdb-asm', '--', '--strategy', 'stream'],
        stdin=subprocess.PIPE,
        stdout=subprocess.PIPE,
        stderr=subprocess.PIPE,
        text=True
    )
    stdout, stderr = process.communicate(input=json.dumps(input_data))
    if process.returncode != 0:
        print(f"Error running vecdb-asm: {stderr}")
        sys.exit(1)
    return json.loads(stdout)

def test_sequencing():
    print("Testing vecdb-asm chronological sequencing...")
    input_data = [
        {
            "id": 1, 
            "content": "later event", 
            "metadata": {"modified": "2026-01-08T10:00:00Z"}
        },
        {
            "id": 2, 
            "content": "earlier event", 
            "metadata": {"modified": "2026-01-08T08:00:00Z"}
        }
    ]
    
    output = run_asm(input_data)
    
    assert len(output) == 2
    assert output[0]["content"] == "earlier event"
    assert output[1]["content"] == "later event"
    print("✅ Sequencing test passed!")

if __name__ == "__main__":
    test_sequencing()
