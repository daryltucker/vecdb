# openwebui_to_chat.jq - Normalizer for OpenWebUI exports
#
# Transforms OpenWebUI JSON export → canonical chat schema
#
# Input:  OpenWebUI export (array of conversation objects)
# Output: Array of chat messages conforming to schemas/chat.schema.json
#
# Usage:
#   vecq -L examples export.json -q 'webui_to_chat | chat_format'
#   vecq -L examples export.json -q 'webui_to_chat | .[-5:]'

# Decode common HTML entities
def decode_html:
  gsub("&gt;"; ">") |
  gsub("&lt;"; "<") |
  gsub("&quot;"; "\"") |
  gsub("&#x27;"; "'") |
  gsub("&#x60;"; "`") |
  gsub("&amp;"; "&"); 

# Extract content (handles string or array)
def get_content: 
  if (.content | type) == "array" then 
    (.content | join("")) 
  else 
    (.content // "") 
  end | decode_html;

# ============================================
# NORMALIZER: webui_to_chat
# ============================================
# Transforms OpenWebUI export to canonical chat schema
# 
# Input: OpenWebUI export array
# Output: Flat array of chat messages
def webui_to_chat:
  [.[] | {
    meta: {
      schema_version: "v1",
      normalizer: "openwebui_chat",
      original_id: (.id // null),
      title: (.title // null)
    },
    messages: [.chat.messages[] | {
      role: (.role // "unknown"),
      content: get_content,
      timestamp: (.timestamp // null),
      name: (.name // null),
      "x-source": "openwebui"
    }]
  }];

# Convenience: normalize single conversation
def webui_conversation_to_chat:
  {
    meta: {
      schema_version: "v1",
      normalizer: "openwebui_chat",
      original_id: (.id // null),
      title: (.title // null)
    },
    messages: [.chat.messages[] | {
      role: (.role // "unknown"),
      content: get_content,
      timestamp: (.timestamp // null),
      name: (.name // null),
      "x-source": "openwebui"
    }]
  };
