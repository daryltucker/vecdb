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

def test_deduplication():
    print("Testing vecdb-asm deduplication...")
    input_data = [
        {"id": 1, "content": "hello world"},
        {"id": 2, "content": "foo bar"},
        {"id": 1, "content": "hello world"}  # Duplicate
    ]
    
    output = run_asm(input_data)
    
    assert len(output) == 2
    assert output[0]["content"] == "hello world"
    assert output[1]["content"] == "foo bar"
    print("✅ Deduplication test passed!")

def test_no_dedupe_flag():
    print("Testing vecdb-asm --no-dedupe flag...")
    input_data = [
        {"id": 1, "content": "hello world"},
        {"id": 1, "content": "hello world"}
    ]
    
    process = subprocess.Popen(
        ['cargo', 'run', '-p', 'vecdb-asm', '--', '--strategy', 'stream', '--no-dedupe'],
        stdin=subprocess.PIPE,
        stdout=subprocess.PIPE,
        stderr=subprocess.PIPE,
        text=True
    )
    stdout, stderr = process.communicate(input=json.dumps(input_data))
    if process.returncode != 0:
        print(f"Error running vecdb-asm: {stderr}")
        sys.exit(1)
    output = json.loads(stdout)
    
    assert len(output) == 2
    print("✅ No-dedupe flag test passed!")

if __name__ == "__main__":
    test_deduplication()
    test_no_dedupe_flag()
