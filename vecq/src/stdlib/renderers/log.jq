# Renders canonical log schema to Markdown

def _log_format_item:
  "[" + (.timestamp // "timestamp?") + "] " +
  "**" + (.level // "INFO") + "** " +
  "(" + (.source // "source?") + "): " +
  (.message // "no message");

def log_format:
  if (type == "array") then
    map(_log_format_item) | join("\n")
  else
    _log_format_item
  end;
