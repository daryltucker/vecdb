#!/usr/bin/env python3
"""
Tier 2 CLI Compliance Test
Enforces that every subcommand in `vecdb-cli/src/main.rs` is documented in `docs/CLI.md`.
"""
import re
import sys
from pathlib import Path

# Paths
ROOT = Path(__file__).parent.parent
MAIN_RS = ROOT / "vecdb-cli/src/main.rs"
DOCS_MD = ROOT / "docs/CLI.md"

def extract_cli_commands(content):
    """
    Extracts enum variants from the `Commands` enum in main.rs.
    """
    # Find enum Commands start
    start_pattern = re.compile(r"enum Commands\s*\{")
    start_match = start_pattern.search(content)
    if not start_match:
        print("Error: Could not find enum Commands in main.rs")
        return set()

    start_idx = start_match.end()
    brace_count = 1
    enum_content = []
    
    # Iterate char by char from start_idx
    for char in content[start_idx:]:
        if char == '{':
            brace_count += 1
        elif char == '}':
            brace_count -= 1
        
        if brace_count == 0:
            break
        enum_content.append(char)
        
    block = "".join(enum_content)
    commands = set()
    
    # Remove comments
    block = re.sub(r"//.*", "", block)
    block = re.sub(r"///.*", "", block)
    
    # Iterate lines
    for line in block.split('\n'):
        line = line.strip()
        if not line: continue
        if line.startswith("#"): continue # Attributes
        
        # Regex: Optional whitespace, Uppercase Start, alphanumeric/underscore, optional whitespace, optional { or ( or ,
        match = re.search(r"^\s*([A-Z][a-zA-Z0-9_]+)\s*(?:[\{\(,]|$)", line)
        if match:
             cmd = match.group(1)
             # Filter out keywords type names if they appear at start of line (unlikely in enum variants list but possible inside struct defs)
             # Basic heuristic: Variants are usually single words. Structured variants have { or (
             if cmd not in ["Completions", "None", "Some", "Box", "Arc", "String", "Option", "Vec", "PathBuf", "bool", "usize"]:
                 commands.add(cmd.lower())
                 
    return commands

def extract_doc_commands(content):
    """
    Extracts command names from level 3 headers in CLI.md.
    Format: ### `command` ...
    """
    commands = set()
    
    # Regex for "### `command_name`" or "### `command_name [ARGS]`"
    # match ### `word
    pattern = re.compile(r"###\s+`([a-z0-9_-]+)")
    
    for match in pattern.finditer(content):
        commands.add(match.group(1).lower())
        
    return commands

def main():
    if not MAIN_RS.exists():
        print(f"FAIL: {MAIN_RS} not found")
        sys.exit(1)
    if not DOCS_MD.exists():
        print(f"FAIL: {DOCS_MD} not found")
        sys.exit(1)

    with open(MAIN_RS, 'r') as f:
        rs_content = f.read()
        
    with open(DOCS_MD, 'r') as f:
        md_content = f.read()
        
    rs_cmds = extract_cli_commands(rs_content)
    doc_cmds = extract_doc_commands(md_content)
    
    # Handle known divergences or sub-commands
    # For now, simplistic check
    
    missing_in_docs = rs_cmds - doc_cmds
    
    # "history" might be documented as "history ingest" -> "history" match?
    # CLI.md has: ### `history ingest`
    # My regex extracts "history". So it should match.
    
    print("Running CLI Compliance Checks...")
    print("--------------------------------")
    print(f"Code Commands: {sorted(list(rs_cmds))}")
    print(f"Doc Commands:  {sorted(list(doc_cmds))}")
    
    if missing_in_docs:
        print(f"❌ FAIL: The following commands are missing from `docs/CLI.md`:")
        for c in missing_in_docs:
            print(f"   - {c}")
        sys.exit(1)
    else:
        print("✅ PASS: All commands documented.")
        sys.exit(0)

if __name__ == "__main__":
    main()
