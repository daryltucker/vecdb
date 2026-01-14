# Normalizer: Detailed Graph -> Architectural Graph
# Prunes functions, variables, and implementations to leave only structural bones.
# Input: A valid JGF v2 Graph Object (usually from src_to_graph)

def graph_to_architecture:
  {
    "graphs": [
      .graphs[] |
      {
        "id": .id,
        "label": .label,
        "directed": .directed,
        # Keep only File, Module, Class, Struct, Interface nodes
        "nodes": (
          .nodes | with_entries(select(
            .value.metadata.type == "file" or
            .value.metadata.type == "module" or
            .value.metadata.type == "class" or
            .value.metadata.type == "structure" or
            .value.metadata.type == "interface"
          ))
        ),
        # Keep edges only if both source and target exist in our filtered nodes
        "edges": (
           # We need a set of valid keys for fast lookup, but pure JQ is slow at that.
           # Instead, we'll just filter edges where source/target are in the filtered nodes keys.
           (.nodes | with_entries(select(
              .value.metadata.type == "file" or
              .value.metadata.type == "module" or
              .value.metadata.type == "class" or
              .value.metadata.type == "structure" or
              .value.metadata.type == "interface"
           )) | keys) as $valid_ids |
           
           .edges | map(select(
             (.source as $s | $valid_ids | index($s)) and
             (.target as $t | $valid_ids | index($t))
           ))
        )
      }
    ]
  };

# Convenience wrapper: Source -> Architecture
def src_to_architecture:
  src_to_graph | graph_to_architecture;
