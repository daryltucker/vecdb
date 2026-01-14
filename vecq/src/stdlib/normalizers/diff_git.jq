def git_status_to_diff:
  # Input: Parsed object from git status --porcelain (requires pre-parsing to JSON)
  # Example input: {"status": "M", "file": "src/main.rs"}
  {
    file: .file,
    change_type: (
      if .status == "M" then "MODIFIED"
      elif .status == "A" then "ADDED"
      elif .status == "D" then "DELETED"
      elif .status == "R" then "RENAMED"
      else "UNKNOWN" end
    ),
    additions: 0, # Not available in simple status
    deletions: 0
  };
