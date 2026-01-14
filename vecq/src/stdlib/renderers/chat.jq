# Renders canonical chat schema to Markdown

def _chat_format_msg:
  "### " + (.role // "unknown") + 
  (if .name then " (" + .name + ")" else "" end) +
  (if .timestamp then " - " + (.timestamp | tostring) else "" end) +
  "\n" + (.content // "") + "\n\n---\n";

def _chat_format_msgs:
  map(_chat_format_msg) | join("\n");

def chat_format:
  if (type == "object" and .messages) then
    # Render Metadata Header (if present)
    (if .meta then 
      "> **Schema**: " + (.meta.schema_version // "?") + " | **Normalizer**: " + (.meta.normalizer // "?") + "\n\n" 
     else "" end) +
    # Render Messages
    (.messages | _chat_format_msgs)
  else
    # Legacy: Assume array of messages
    _chat_format_msgs
  end;

# ============================================
# UTILITIES
# ============================================

def chat_filter_role($role):
  map(select(.role == $role));

def chat_search($pattern):
  map(select(.content | contains($pattern)));

def chat_tail($n):
  .[-$n:];

def chat_head($n):
  .[:$n];
