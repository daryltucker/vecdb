# Vecq Documentation Generator Standard Library
# Usage: import "doc" as doc; doc::markdown

# ------------------------------------------------------------------
# Internal Helpers
# ------------------------------------------------------------------

# Clean up docstrings
def _clean_doc:
  if . == null then ""
  else tostring
  end;

# Recursive node formatter
def _format_node(level):
  (("#" * level) + " ") as $h |
  if .type == "struct" or .type == "enum" or .type == "class" or .type == "interface" or .type == "trait" or .type == "implementation" or .type == "module" then
    $h + (.name // ("unnamed " + .type)) + "\n" +
    (if (.attributes.docstring) then (.attributes.docstring | _clean_doc) + "\n" else "" end) +
    (if .children and (.children | length > 0) then 
      "\n" + ([.children[] | _format_node(level + 1)] | join("")) 
     else "" end) + "\n"
  elif .type == "header" then
    # Markdown headers
    $h + .content + "\n"
  else
    # Default to function-like display (Signature + Doc)
    $h + (.name // ("unnamed " + .type)) + "\n" +
    (if (.attributes.signature) then "```rust\n" + .attributes.signature + "\n```\n" else "" end) +
    (if (.attributes.docstring) then (.attributes.docstring | _clean_doc) + "\n" else "" end) +
    (if .children and (.children | length > 0) then 
      "\n" + ([.children[] | _format_node(level + 1)] | join("")) 
     else "" end) + "\n"
  end;

# Format any item based on its type
def _format_item:
  (_format_node(3));

# Map JSON keys to Section Headers
def _get_header_name:
  if . == "functions" then "Functions"
  elif . == "structs" then "Structs"
  elif . == "enums" then "Enums"
  elif . == "traits" then "Traits"
  elif . == "implementations" then "Implementations"
  elif . == "modules" or . == "module" then "Modules"
  elif . == "imports" or . == "use_statements" then "Imports"
  elif . == "variables" then "Variables"
  elif . == "constants" then "Constants"
  elif . == "classes" then "Classes"
  elif . == "interfaces" then "Interfaces"
  else .
  end;

# Define semantic ordering for sections
def _section_priority:
  if . == "structs" then 10
  elif . == "enums" then 11
  elif . == "classes" then 12
  elif . == "interfaces" then 13
  elif . == "kernels" then 20
  elif . == "functions" then 30
  elif . == "methods" then 31
  elif . == "host_functions" then 32
  elif . == "device_functions" then 33
  else 99
  end;

# ------------------------------------------------------------------
# Public API
# ------------------------------------------------------------------

# Generate full Markdown documentation for the current AST
def markdown:
  [
    "# Documentation\n\n",
    
    # Dynamic iteration over all top-level keys
    (to_entries 
     | sort_by(.key | _section_priority)
     | .[] 
     | select(.value | length > 0)
     | select(.key != "metadata" and .key != "file_type" and .key != "elements")
     | (
         ("## " + (.key | _get_header_name) + "\n\n") +
         ([.value[] | select(.crumbtrail == null) | _format_item] | join(""))
       )
    )
  ] | join("")
;
