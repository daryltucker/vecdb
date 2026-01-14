# Renderer: JGF v2 -> Generic Markdown
# Raw fact-based display

def graph_format:
  .graphs[] |
  "# Graph: " + (.label // .id) + "\n\n" +
  "## Nodes\n" +
  ([.nodes | to_entries[] | "- **" + .key + "** (" + (.value.metadata.type // "node") + ")"] | join("\n")) +
  "\n\n## Relationships\n" +
  ([.edges[] | "- " + .source + " --[" + .relation + "]--> " + .target] | join("\n")) +
  "\n";

# Renderer: JGF v2 -> Mermaid Diagram
# Visual dependency graph
def _mermaid_id: gsub("[^a-zA-Z0-9]"; "_");

def graph_format_mermaid:
  .graphs[] |
  "graph TD\n" +
  ([.nodes | to_entries[] | 
      "  " + (.key | _mermaid_id) + "[\"" + (.value.label // .key) + "\"]"
  ] | join("\n")) +
  "\n" +
  ([.edges[] | 
      "  " + (.source | _mermaid_id) + " -- " + (.relation // "related") + " --> " + (.target | _mermaid_id)
  ] | join("\n")) +
  "\n\n  %% Styling\n  classDef file fill:#e1f5fe,stroke:#01579b,stroke-width:2px\n  classDef function fill:#fff3e0,stroke:#e65100,stroke-width:1px\n  classDef struct fill:#e8f5e9,stroke:#1b5e20,stroke-width:1px\n  classDef module fill:#f3e5f5,stroke:#4a148c,stroke-width:2px\n" +
  "\n";
