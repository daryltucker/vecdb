# Finding Your Way: `elements` & `suggest`

`vecq` is powerful but knowing what to query (e.g., is it `.functions[]` or `.func[]`?) can be tricky. To solve this, `vecq` provides discovery tools.

## 1. `vecq elements`
The `elements` subcommand tells you exactly what structural components `vecq` knows how to extract from a specific file type.

### List all supported languages
```bash
vecq elements
```

### List elements for a specific language
```bash
vecq elements rs
# Output:
# Elements for Rust:
#   enums               functions           implementations     structs             
#   traits              use_statements      
```

### JSON Output for Scripts
```bash
vecq elements md --json
```

---

## 2. `vecq suggest`
The `suggest` subcommand helps you build `jq` queries by suggesting patterns based on natural language or structural keywords.

### How it works
It scans the internal `SchemaRegistry` to find fields that match your intent and provides ready-to-use jq snippets.

### Example: Finding functions
```bash
$ vecq suggest function
Query suggestions for: "function"
  1. .device_functions[]
  2. .device_functions[] | .name
  3. .device_functions[] | select(.name | contains("..."))
  4. .functions[]
  5. .functions[] | .name
  6. .functions[] | select(.name | contains("..."))
  7. .host_functions[]
  8. .host_functions[] | .name
  9. .host_functions[] | select(.name | contains("..."))
```

### Example: Finding Markdown items
```bash
$ vecq suggest link
Query suggestions for: "link"
  1. .links[]
  2. .links[] | .name
  3. .links[] | select(.name | contains("..."))
```

## Why use this?
1. **Discoverability**: You don't need to memorize the AST of 10+ different languages.
2. **"Codified Correctness"**: The suggestions are derived directly from the code's `SchemaRegistry`. If a new language is added to the engine, it automatically appears in `suggest`.
3. **Precision**: It helps you build queries that work the first time.

Tip: Use `vecq --explain <query>` after you've picked a suggestion to see what it technically does!
