# Hash function using simple checksum for ID generation (since 'hash' native is missing)
def _src_hash:
  split("") | map(explode | .[0]) | add | tostring;

def _to_lower_tag:
  if . == "TODO" then "todo"
  elif . == "FIXME" then "fixme"
  elif . == "OPTIMIZE" then "optimize"
  elif . == "XXX" then "xxx"
  else . end;

def src_to_task:
  # Input range: Raw text file content (via -t text)
  # 1. Split into lines to allow scanning multiple tags per file
  # 2. Capture common comment symbols and tags
  split("\n")[] |
  select(re_test("(?i)(TODO|FIXME|OPTIMIZE|XXX)")) | 
  re_capture("(?<symbol>(?://|#|--|/\\*|^\\s*\\*))\\s*(?<tag>TODO|FIXME|OPTIMIZE|XXX):?\\s*(?<message>.*)$") | 
  
  # Normalize to Task Schema
  {
    "element_type": "task",
    "id": ("task_" + (.message | _src_hash)),
    "title": (.message | re_sub("\\s*\\*/$"; "")), # Clean up trailing C-style comments
    "status": "open",
    "priority": (
      if (.tag == "FIXME" or (.message | re_test("(?i)bug|leak|crash|urgent"))) then "high"
      elif (.tag == "OPTIMIZE") then "low"
      else "medium"
      end
    ),
    "tags": ["todo", (.tag | _to_lower_tag)],
    "metadata": {
      "source": "source_code",
      "comment_style": .symbol,
      "created": (now | todate)
    }
  };
