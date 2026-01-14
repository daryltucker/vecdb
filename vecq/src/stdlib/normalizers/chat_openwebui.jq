# Helper: decode_html
def _decode_html:
  split("&amp;") | join("&") |
  split("&lt;") | join("<") |
  split("&gt;") | join(">") |
  split("&quot;") | join("\"") |
  split("&#x27;") | join("'") |
  split("&#x60;") | join("`");

def _get_content: 
  if (.content | type) == "array" then 
    (.content | join("")) 
  else 
    (.content // "") 
  end | _decode_html;

# Helper: Normalize single conversation object
def openwebui_conversation_to_chat:
  {
    meta: {
      schema_version: "v1",
      normalizer: "openwebui_chat",
      original_id: (.id // null),
      title: (.title // null)
    },
    messages: [.chat.messages[] | {
      role: (.role // "unknown"),
      content: _get_content,
      timestamp: (.timestamp // null),
      name: (.name // null),
      "x-source": "openwebui"
    }]
  };

# Transforms OpenWebUI JSON export → canonical chat schema
def openwebui_to_chat:
  if (type == "array") then
    # Bulk export: array of conversations
    map(openwebui_conversation_to_chat)
  else
    # Single conversation object
    openwebui_conversation_to_chat
  end;
