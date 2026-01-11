# chat_format.jq - Renderer for canonical chat schema
#
# Transforms canonical chat schema → human-readable Markdown
#
# Input:  Array of chat messages (schemas/chat.schema.json)
# Output: Formatted Markdown string
#
# Usage:
#   vecq -L examples data.json -q 'some_normalizer | chat_format'

# ============================================
# LAYER 1: Format single message
# ============================================
def chat_format_msg:
  "### " + (.role // "unknown") + 
  (if .name then " (" + .name + ")" else "" end) +
  (if .timestamp then " - " + (.timestamp | tostring) else "" end) +
  "\n" + (.content // "") + "\n\n---\n";

# ============================================
# LAYER 2: Format array of messages
# ============================================
def chat_format_msgs:
  map(chat_format_msg) | join("\n");

# ============================================
# TOP-LEVEL: chat_format
# ============================================
# Main entry point for rendering chat schema to Markdown
# Input: ChatSession object (with .meta and .messages) or array of messages (legacy support)
def chat_format:
  if (type == "object" and .messages) then
    # Render Metadata Header (if present)
    (if .meta then 
      "> **Schema**: " + (.meta.schema_version // "?") + " | **Normalizer**: " + (.meta.normalizer // "?") + "\n\n" 
     else "" end) +
    # Render Messages
    (.messages | chat_format_msgs)
  else
    # Legacy: Assume array of messages
    chat_format_msgs
  end;

# ============================================
# UTILITIES
# ============================================

# Filter by role
def chat_filter_role($role):
  map(select(.role == $role));

# Search by content pattern (case-insensitive)
def chat_search($pattern):
  map(select(.content | test($pattern; "i")));

# Last N messages
def chat_tail($n):
  .[-$n:];

# First N messages
def chat_head($n):
  .[:$n];
