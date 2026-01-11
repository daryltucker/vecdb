#!/usr/bin/env python3
"""
Tier 2 Config Compliance Test
Enforces that every field in `vecdb-core/src/config.rs` structs is documented in `docs/CONFIG.md`.

Mappings:
- struct Config -> Top-Level Options
- struct Profile -> Profile Options
- struct CollectionConfig -> Collection Profile Options
- struct IngestionConfig -> Ingestion Options
"""
import re
import sys
from pathlib import Path

# Paths
ROOT = Path(__file__).parent.parent
CONFIG_RS = ROOT / "vecdb-core/src/config.rs"
DOCS_MD = ROOT / "docs/CONFIG.md"

def extract_rust_struct_fields(content, struct_name):
    """
    Extracts public fields from a Rust struct definition.
    Simple regex-based parser.
    """
    # Find struct block
    struct_pattern = re.compile(r"pub struct " + struct_name + r"\s*\{(.*?)\}", re.DOTALL)
    match = struct_pattern.search(content)
    if not match:
        print(f"Error: Could not find struct {struct_name} in config.rs")
        return set()

    block = match.group(1)
    fields = set()
    
    # Iterate over lines to find "pub field_name:"
    # We ignore comments and attributes
    for line in block.split('\n'):
        line = line.strip()
        if line.startswith("//"): continue
        if line.startswith("#["): continue
        
        # Match "pub field_name:"
        field_match = re.match(r"pub\s+([a-z0-9_]+)\s*:", line)
        if field_match:
            fields.add(field_match.group(1))
            
    return fields

def extract_markdown_table_keys(content, header_regex):
    """
    Finds a markdown table under a specific header and extracts keys from the first column.
    """
    # Find the header
    header_match = re.search(header_regex, content, re.IGNORECASE)
    if not header_match:
        print(f"Error: Could not find header matching '{header_regex}' in CONFIG.md")
        return set()
    
    start_pos = header_match.end()
    
    # Look for the table structure: | Key | ...
    # We skip text until we find a table row
    
    keys = set()
    lines = content[start_pos:].split('\n')
    in_table = False
    
    for line in lines:
        line = line.strip()
        if not line:
            if in_table: break # End of table
            continue
            
        if line.startswith("|"):
            # It's a table row
            if "---" in line: continue # Separator row
            if "Key" in line and "Type" in line: continue # Header row
            
            # Extract first column
            parts = [p.strip() for p in line.split("|")]
            if len(parts) > 1:
                key_cell = parts[1] # First part is empty string due to leading |
                # Key might be "`field_name`" or "field_name"
                key = key_cell.strip("` ")
                if key:
                    keys.add(key)
            in_table = True
        elif in_table:
            # Table ended
            break
            
    return keys

def main():
    if not CONFIG_RS.exists():
        print(f"FAIL: {CONFIG_RS} not found")
        sys.exit(1)
    if not DOCS_MD.exists():
        print(f"FAIL: {DOCS_MD} not found")
        sys.exit(1)

    with open(CONFIG_RS, 'r') as f:
        rs_content = f.read()
        
    with open(DOCS_MD, 'r') as f:
        md_content = f.read()
        
    # Define checks
    checks = [
        ("Config", "Top-Level Options", {"profiles", "collections", "collection_aliases", "ingestion"}), # Exclude structural maps
        ("Profile", "Profile Options", set()),
        ("CollectionConfig", "Collection Profile Options", set()),
        ("IngestConfig", "Ingestion Options", {"overrides", "path_rules"}) # IngestionConfig is mapped to IngestionOptions. Check struct name (IngestionConfig)
    ]
    
    # Correction: struct name is IngestionConfig
    checks[3] = ("IngestionConfig", "Ingestion Options", {"overrides", "path_rules"})

    failures = 0

    print("Running Configuration Compliance Checks...")
    print("------------------------------------------")

    for struct_name, doc_header, exclusions in checks:
        print(f"Checking {struct_name} vs '{doc_header}'...")
        rs_fields = extract_rust_struct_fields(rs_content, struct_name)
        doc_keys = extract_markdown_table_keys(md_content, doc_header)
        
        # Filter exclusions
        rs_fields = {f for f in rs_fields if f not in exclusions}
        
        missing_in_docs = rs_fields - doc_keys
        
        if missing_in_docs:
            print(f"  ❌ FAIL: The following fields in `{struct_name}` are missing from `{doc_header}` documentation:")
            for f in missing_in_docs:
                print(f"     - {f}")
            failures += 1
        else:
            print(f"  ✅ PASS: All {len(rs_fields)} fields documented.")
            
    if failures > 0:
        print("\nCompliance Check FAILED. Please update `docs/CONFIG.md`.")
        sys.exit(1)
    else:
        print("\nCompliance Check PASSED.")
        sys.exit(0)

if __name__ == "__main__":
    main()
