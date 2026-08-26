#!/usr/bin/env python3
"""
Tier 2 CLI Compliance Test

Enforces two things:
  1. Every subcommand in `vecdb-cli/src/commands/mod.rs` is documented in
     `docs/CLI.md`.
  2. Every MCP tool the server dispatches is documented in the agent manual.

(2) exists because `get_job_status` shipped undocumented and was found by an
external eval rather than by us — an agent cannot call a tool it has no way to
learn about, so an undocumented tool is a tool that does not exist.
"""
import re
import sys
from pathlib import Path

# Paths
ROOT = Path(__file__).parent.parent
MAIN_RS = ROOT / "vecdb-cli/src/commands/mod.rs"
DOCS_MD = ROOT / "docs/CLI.md"
DISPATCHER_RS = ROOT / "vecdb-server/src/rpc/dispatcher.rs"
AGENT_MAN = ROOT / "vecdb-cli/src/docs/man_agent.md"

def extract_cli_commands(content):
    """
    Extracts enum variants from the `Commands` enum in commands/mod.rs.
    """
    # Find enum Commands start
    start_pattern = re.compile(r"enum Commands\s*\{")
    start_match = start_pattern.search(content)
    if not start_match:
        print("Error: Could not find enum Commands in commands/mod.rs")
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
    
    failed = False

    if missing_in_docs:
        print("❌ FAIL: The following commands are missing from `docs/CLI.md`:")
        for c in sorted(missing_in_docs):
            print(f"   - {c}")
        failed = True
    else:
        print("✅ PASS: All commands documented.")

    # ── MCP tools vs the agent manual ────────────────────────────────
    print()
    print("Running MCP Tool Compliance Checks...")
    print("-------------------------------------")

    tools = set(re.findall(r'"name":\s*"([a-z_]+)"', DISPATCHER_RS.read_text()))
    manual = AGENT_MAN.read_text()

    # A tool counts as documented only if it has its own heading. A passing
    # mention inside prose is how `ingest_historic_version` stayed in the manual
    # for a release after the tool had been renamed to `ingest_history`.
    documented = set(re.findall(r"^###\s+`?([a-z_]+)`?\s*$", manual, re.M))

    print(f"Dispatched tools: {sorted(tools)}")
    undocumented = tools - documented
    if undocumented:
        print("❌ FAIL: MCP tools missing a `###` section in man_agent.md:")
        for t in sorted(undocumented):
            print(f"   - {t}")
        failed = True
    else:
        print(f"✅ PASS: All {len(tools)} MCP tools documented.")

    # And the reverse: a documented tool that is not dispatched is a promise the
    # server does not keep.
    phantom = {d for d in documented if d.islower() and "_" in d} - tools
    phantom -= {"vecdb_collections"}
    if phantom:
        print("❌ FAIL: man_agent.md documents tools the server does not dispatch:")
        for t in sorted(phantom):
            print(f"   - {t}")
        failed = True

    sys.exit(1 if failed else 0)

if __name__ == "__main__":
    main()
