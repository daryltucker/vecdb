# Normalizer: Source AST -> JGF v2 (Graph)
# Input: Single-file AST object or Array of AST objects (slurp)

def _src_to_graph_single:
  (.metadata.path // "unknown") as $file_path |
  [
    ((.functions // []) | .[] | . + {x_type: "function"}),
    ((.structs // []) | .[] | . + {x_type: "structure"}),
    ((.classes // []) | .[] | . + {x_type: "class"}),
    ((.interfaces // []) | .[] | . + {x_type: "interface"}),
    ((.enums // []) | .[] | . + {x_type: "enumeration"}),
    ((.traits // []) | .[] | . + {x_type: "trait"}),
    ((.modules // []) | .[] | . + {x_type: "module"}),
    ((.impls // []) | .[] | . + {x_type: "implementation"})
  ] as $elements |
  {
    "id": $file_path,
    "nodes": (
      reduce $elements[] as $it (
        {($file_path): { "label": $file_path, "metadata": { "type": "file" } } };
        . + {($file_path + "::" + ($it.name // "anon")): {
          "label": ($it.name // $it.x_type),
          "metadata": {
            "type": $it.x_type,
            "path": $file_path,
            "line_start": $it.line_start,
            "line_end": $it.line_end,
            "visibility": ($it.attributes.visibility // "private")
          }
        }}
      )
    ),
    "edges": (
      [ $elements[] | {
        "source": ($file_path + "::" + (.name // "anon")),
        "target": $file_path,
        "relation": "child-of"
      } ] +
      [ (.imports // [])[] | {
          "source": $file_path,
          "target": (.name // "unknown"),
          "relation": "imports"
      } ]
    )
  };

def src_to_graph:
  if type == "array" then
    {
      "graphs": [
        {
          "id": "project-graph",
          "label": "Project Architecture Map",
          "directed": true,
          "nodes": (reduce .[] as $item ({}; . + ($item | _src_to_graph_single).nodes)),
          "edges": (reduce .[] as $item ([]; . + ($item | _src_to_graph_single).edges))
        }
      ]
    }
  else
    { "graphs": [_src_to_graph_single] }
  end;
