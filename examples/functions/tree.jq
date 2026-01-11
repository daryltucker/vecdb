# tree.jq - Helpers for 'tree -J' output
# Usage: tree -J | vecq -l json 'paths | select(contains("task.md"))'

# Recursive descent through tree structure
# Usage: walk_tree | select(.name == "foo")
def walk_tree:
  .[] | (., (if .contents then (.contents | walk_tree) else empty end));

# Map tree entries to full paths (strings)
# Usage: paths | select(contains("target"))
def paths(prefix):
  .[] | (prefix + .name) as $p | ($p, (if .contents then (.contents | paths($p + "/")) else empty end));
def paths: paths("");

# Filter for files only
def files: walk_tree | select(.type == "file");

# Filter for directories only
def dirs: walk_tree | select(.type == "directory");

# Find entries by name (regex)
def find_item(p): walk_tree | select(.name | test(p));

# List files with their sizes (if available)
def manifest: files | "\(.name) (\(.size // "unknown"))";
