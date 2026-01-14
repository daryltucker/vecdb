# Renderer: Architecture Pruned Graph -> Formats

def architecture_format:
  .graphs[] |
  "# Architecture: " + (.label // "System") + "\n\n" +
  ([.nodes | to_entries[] | "- " + .value.label] | join("\n"));

def _mermaid_safe: gsub("[^a-zA-Z0-9]"; "_");

def architecture_format_mermaid:
  .graphs[] |
  "graph TD\n" +
  "  classDef file fill:#e1f5fe,stroke:#01579b,stroke-width:2px,color:#000000;\n" +
  "  classDef module fill:#f3e5f5,stroke:#4a148c,stroke-width:2px,color:#000000;\n" +
  "  classDef object fill:#fff3e0,stroke:#e65100,stroke-width:1px,color:#000000;\n" +
  "  classDef default fill:#ffffff,stroke:#333333,stroke-width:1px,color:#000000;\n" +
  ([.nodes | to_entries | group_by((.value.metadata.path // .value.label // "root") | gsub("^\\./"; "") | split("/")[0] // "root")[] |
      (.[0].value.metadata.path // .[0].value.label // "root" | gsub("^\\./"; "") | split("/")[0] // "root") as $group |
      "  subgraph " + ($group | _mermaid_safe) + " [" + ($group) + "]\n" +
      (map(
        "    " + (.key | _mermaid_safe) + "[\"" + (.value.label | gsub("\""; "'")) + "\"]" + 
        (if .value.metadata.type == "file" then ":::file"
         elif .value.metadata.type == "module" then ":::module"
         elif .value.metadata.type == "class" or .value.metadata.type == "structure" then ":::object"
         else "" end)
      ) | join("\n")) +
      "\n  end"
  ] | join("\n\n")) +
  "\n" +
  ([.edges[] | 
      "  " + (.source | _mermaid_safe) + " -.-> " + (.target | _mermaid_safe)
  ] | join("\n"));
