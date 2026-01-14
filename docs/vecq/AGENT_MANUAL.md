# AGENT INTERFACE SPECIFICATION: vecq
Version: 0.1.0

## PURPOSE
vecq allows AI Agents to query source code structure (AST) as if it were JSON.

## CAPABILITIES

### 1. Querying (`query`)
- **Action**: Parse source code and filter AST nodes using jq syntax.
- **Usage**: `vecq <INPUT> <QUERY>`
- **Output**: JSON Lines or Array.
- **Recursion**: Use `-R` to recursively process a directory. This is the recommended default for directories.
- **Common Queries**:
    - `.functions[]` : List all functions
    - `.structs[]` : List all structs/classes
    - `.code_blocks[]` : List markdown code blocks (useful for extraction)
    - `.imports[]` : List imports
- **Examples**:
    - `vecq -R src/ -q '.functions[] | select(.visibility == "pub")' --grep-format` (Find public functions)
    - `vecq README.md -q '.code_blocks[] | select(.attributes.language == "bash") | .content' -r` (Extract bash code)
    - `vecq src/main.rs -L examples/functions -q 'my_lib::filter'`

### 2. Filtering & Manipulation (`select`, `map`)
- **`select(condition)`**: Keep nodes matching a predicate.
    - Example: `.functions[] | select(.visibility == "pub")`
- **`map(filter)`**: Apply transformation to each element.
    - Example: `.functions | map(.name)` -> Output list of function names.

### 3. Conversion (`convert`)
- **Action**: Dump the full AST as JSON.
- **Usage**: `vecq --convert <INPUT>`

### 4. Introspection (`list-filters`)
- **Action**: List all available jq filters and functions (standard library).
- **Usage**: `vecq list-filters`

### 5. Documentation (`doc`)
- **Action**: Generate standardized Markdown documentation from semantic code structure.
- **Usage**: `vecq doc <INPUT>`
- **Output**: Pure Markdown (headers, code blocks, docstrings).
- **Mechanism**: Runs embedded `doc.jq` logic on strict Tree-sitter AST.

### 6. Normalization (Unified Schema Layer)
- **Action**: Convert raw data into canonical JSON schemas.
- **Usage**: `vecq <INPUT> -q 'auto_normalize'`
- **Components**:
    - `log.jq`: Logs (`nginx_to_log`) -> `schemas/log.schema.json`
    - `task.jq`: Tasks (`github_to_task`) -> `schemas/task.schema.json`
    - `artifact.jq`: Build Artifacts -> `schemas/artifact.schema.json`


## PROTOCOL COMPLIANCE
- **Errors**: Printed to STDERR.
- **Success**: Printed to STDOUT.
- **Format**: All data output is valid JSON (unless overridden by --grep-format or -r).

## OPTIONS
- `-f, --from-file <PATH>`: Read query filter from file
- `-q, --query <QUERY>`: Specify query filter as flag (useful for piped input)
- `-L, --library-path <PATH>`: Add directory to search for library modules (.jq files)
- `-o, --format <FORMAT>`: Output format (json, grep, human)
- `-s, --slurp`: Read all inputs into array before querying
- `--grep-format`: Shortcut for `-o grep` (Recommended for search tasks)
- `-r, --raw-output`: Output raw strings, not JSON text (Critical for code extraction)
- `-R, --recursive`: Recursively process directories

## CONTEXT
Use this tool when you need to:
1. Understand the structure of a file without reading the whole text.
2. Find specific code elements (e.g. "all public functions") reliably.
3. Extract executable code blocks from documentation.

## DISCOVERY PROTOCOL
To avoid hallucinating schema structure, Agents MUST follow this protocol when encountering unmatched files:

1. **Introspect Filters**: Run `vecq list-filters` to see available jq tools.
2. **Introspect Schema**: Run `vecq <file> -q 'keys'` to see top-level keys.
3. **Introspect Element**: Run `vecq <file> -q '.elements[0]'` to see node structure.

## AST REFERENCE (Common Nodes)

To see the exact schema for a file, run `vecq --convert <file>`.

### Markdown (`.md`)
- `.headers[]`: List of headers (`{ content, level, line_start }`)
- `.code_blocks[]`: Fenced code blocks (`{ language, content }`)
- `.links[]`: Hyperlinks (`{ text, url }`)

### Rust/Python/Go/C++
- `.functions[]`: Function definitions (`{ name, signature, content }`)
- `.structs[]`: Class/Struct definitions (`{ name, content }`)
- `.imports[]`: Import statements
- `.comments[]`: Top-level comments

## THROUGH THE EYES OF AN AGENT: A WALKTHROUGH

This section documents how an AI Agent (like me) actually uses this system to "see" into your code before touching a single byte.

### Phase 0: Semantic Discovery (Find the Hidden Attributes)
Before searching, I need to know what "keys" are available for an element (e.g., "Does a link have a title?"). Since I am an AI, I can run a "Probe" query on a small sample to discover the schema dynamically.

**Agent Action (Tool Call)**:
```json
{
  "name": "mcp_vecdb_code_query",
  "arguments": {
    "path": "/path/to/some/file.md",
    "query": ".links[0] | keys"
  }
}
```

**What the Agent "Sees" (Output)**:
```json
["attributes", "content", "line_end", "line_start", "name", "type"]
```

I then "Deep Probe" the attributes:
**Query**: `.links[0].attributes | keys` -> **Output**: `["file_path", "title"]`.

Now I know I can filter links by their title!

### Phase 1: Structural Reconnaissance
When I am given a task on a file I've never seen, I don't read the whole file. That wastes tokens and introduces noise. Instead, I use the MCP server to get a "X-Ray" view.

**Agent Action (Tool Call)**:
```json
{
  "name": "mcp_vecdb_code_query",
  "arguments": {
    "path": "/absolute/path/to/parsers/rust.rs",
    "query": ".functions[] | select(.name==\"parse\") | {name, crumbtrail, line_range: [.line_start, .line_end]}"
  }
}
```

**What the Agent "Sees" (Output)**:
```json
{
  "crumbtrail": "impl Parser for RustParser",
  "line_range": [337, 356],
  "name": "parse"
}
```

### Phase 2: Analyzing Relationships
Notice the `crumbtrail` above. I now know that `parse` isn't a standalone function; it's part of the `Parser` implementation for `RustParser`. 

If I need to see its "siblings" (other methods in that same block), I can broaden my eye:

**Agent Action (Tool Call)**:
```json
{
  "name": "mcp_vecdb_code_query",
  "arguments": {
    "path": "/absolute/path/to/parsers/rust.rs",
    "query": ".implementations[] | select(.name == \"impl Parser for RustParser\") | .children[].name"
  }
}
```

**What the Agent "Sees" (Output)**:
```text
"parse"
"file_extensions"
"language_name"
```

### Phase 3: The "Surgical" Edit
Now that I've found the target (`parse` at lines 337-356) and understood its architectural context, I can make a high-confidence tool call to `replace_file_content` using those exact line numbers.

### PRO-TIPS FOR AGENTS
1.  **Always use Absolute Paths** in MCP tool calls to avoid "File Not Found" errors.
2.  **Filter Aggressively**: Use `| .[0:5]` or specific `select()` filters to keep JSON output small.
3.  **Trust the Crumbtrail**: If an element doesn't have a crumbtrail, it's a top-level item. If it does, always look at the parent to understand the implications of your changes.

