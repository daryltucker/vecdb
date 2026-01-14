# Renders canonical artifact schema to Markdown

def _artifact_format_item:
  (if .status == "FAILURE" then "❌" elif .status == "WARNING" then "⚠️" else "ℹ️" end) + " **" + (.type // "Artifact") + "**: " + (.summary // "No summary") + "\n" +
  "  Location: " + (.location.file // "?") + ":" + ((.location.line // "?") | tostring) + "\n" +
  (if .details then "  Details: " + .details else "" end);

def artifact_format:
  if (type == "array") then
    map(_artifact_format_item) | join("\n")
  else
    _artifact_format_item
  end;
