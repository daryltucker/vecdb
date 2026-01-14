# Renders canonical diff schema to Markdown

def _diff_format_item:
  "### " + (.file // "unknown") + " (" + (.change_type // "UNKNOWN") + ")\n" +
  "```diff\n" +
  (.hunks[]?.content // "") +
  "\n```";

def diff_format:
  if (type == "array") then
    map(_diff_format_item) | join("\n")
  else
    _diff_format_item
  end;
