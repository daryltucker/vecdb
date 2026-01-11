# Vecq Documentation Generator Standard Library
# Usage: import "doc" as doc; doc::markdown
#
# IMPORTANT: jaq requires STRICT definition-before-use ordering.
# Helper functions MUST be defined before they are used in other functions.
# Do not move `convert_to_array` or `_clean_doc` below functions that call them.

# ------------------------------------------------------------------
# Internal Helpers
# ------------------------------------------------------------------

# Polyfill for validation (jq's tonumber/tostring behavior varies)
def convert_to_array: .;

# Helper to debug raw AST for development
def debug: .;


# Clean up docstrings (remove leading /// or /**, trim whitespace)
def _clean_doc:
  if . == null then ""
  else
    # Simple heuristic: remove common Rust doc patterns
    # Real implementation might need more robust parsing
    tostring
    | gsub("^\\s*///\\s?"; "") 
    | gsub("^\\s*\\*\\s?"; "")
  end;

# Format any item based on its type
def _format_item:
  if .type == "struct" or .type == "enum" or .type == "class" or .type == "interface" then
    "### " + .name + "\n" +
    (if (.attributes.docstring) then (.attributes.docstring | _clean_doc) + "\n" else "" end) + "\n"
  elif .type == "header" then
    # Markdown headers
    (.content | sub("^#+\\s*"; "") | "### " + . + "\n")
  else
    # Default to function-like display (Signature + Doc)
    "### " + .name + "\n" +
    (if (.attributes.signature) then "```rust\n" + .attributes.signature + "\n```\n" else "```rust\nfn " + .name + "(...)\n```\n" end) +
    (if (.attributes.docstring) then (.attributes.docstring | _clean_doc) + "\n" else "" end) + "\n"
  end;

# Map JSON keys to Section Headers
def _get_header_name(key):
  if key == "structs" or key == "enums" or key == "classes" or key == "interfaces" then "Types"
  elif key == "functions" or key == "methods" or key == "host_functions" or key == "device_functions" then "API"
  elif key == "kernels" then "Kernels"
  elif key == "files" then "Files"
  else ("UNKNOWN: " + key) # Discovery mode for unknown keys
  end;

# Define semantic ordering for sections
def _section_priority(key):
  if key == "structs" then 10
  elif key == "enums" then 11
  elif key == "classes" then 12
  elif key == "interfaces" then 13
  elif key == "kernels" then 20
  elif key == "functions" then 30
  elif key == "methods" then 31
  elif key == "host_functions" then 32
  elif key == "device_functions" then 33
  else 99
  end;

# ------------------------------------------------------------------
# Public API
# ------------------------------------------------------------------

# Generate full Markdown documentation for the current AST
# Filters for structs, then functions, to create a logical layout.
def markdown:
  [
    "# Documentation\n\n",
    
    # Dynamic iteration over all top-level keys
    (to_entries 
     | convert_to_array 
     | sort_by(_section_priority(.key)) 
     | .[] 
     | select(.value | type == "array") 
     | select(.value | length > 0)
     | select(.key != "metadata") # Skip metadata
     
     | (
         "## " + (_get_header_name(.key)) + "\n\n" +
         (.value[] | _format_item)
       )
    )
  ] | join("")
;


