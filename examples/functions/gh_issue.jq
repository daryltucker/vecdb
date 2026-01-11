# gh_issue.jq - Transform GitHub Issue JSON to Chat Schema
# Usage: vecq issue.json -q 'gh_issue' (Legacy compatible)
#        vecq issue.json -q 'gh_issue_to_chat | chat_format'

def _gh_label_badge:
  "[" + .name + "](https://github.com/labels/" + .name + ")";

# Adapter: Convert GitHub Issue to Chat Session
def gh_issue_to_chat:
  {
    meta: {
      schema_version: "v1",
      normalizer: "gh_issue",
      source: "github",
      title: (.title // "Untitled"),
      id: (.number // null)
    },
    messages: [
      {
        role: "user",
        name: (.user.login // "ghost"),
        content: (
          "# " + (.title // "Untitled") + " (#" + (.number | tostring) + ")\n" +
          "**Status**: " + (.state // "unknown") + " | " +
          "**Labels**: " + (if .labels then (.labels | map(_gh_label_badge) | join(", ")) else "None" end) + "\n\n" +
          "---\n\n" +
          (.body // "No description provided.")
        )
      }
    ]
  };

# Legacy Wrapper (Preserves backward compatibility)
def gh_issue:
  gh_issue_to_chat | chat_format;
