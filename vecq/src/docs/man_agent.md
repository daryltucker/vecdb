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

### 6. Normalization & Schemas (Unified Layer)
- **Action**: Transform disparate raw data (logs, issues, build errors) into canonical schemas.
- **Philosophy**: "Parse, Don't Validate". Lenient conversion to a shared contract.
- **Usage**: `vecq <INPUT> -q 'auto_normalize'` or specific functions like `nginx_to_log`.
- **Available Normalizers**:
    - **Logs**: `nginx_to_log`, `journald_to_log` -> `log.schema.json`
    - **Tasks**: `github_to_task`, `todo_to_task` -> `task.schema.json`
    - **Artifacts**: `cargo_to_artifact`, `junit_to_artifact` -> `artifact.schema.json`
    - **Diffs**: `git_diff_to_diff` -> `diff.schema.json`
- **Auto-Normalization**: Use `auto_normalize` to heuristically detect and convert input.


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

## RECIPES (Structural Analysis)
Common Agentic workflows for codebase understanding.

### 1. The "API Surface Auditor"
**Goal**: List public functions/structs to understand the external interface.
**Cmd**: `vecq -R src/ -q '(.functions // [])[] | select(.visibility == "pub") | {name, signature}' --compact`

### 2. The "Complexity Hunter"
**Goal**: Find functions > 50 lines (prime candidates for bugs/refactoring).
**Cmd**: `vecq -R src/ -q '(.functions // [])[] | select((.line_end - .line_start) > 50) | {name, lines: (.line_end - .line_start)}' --compact`

### 3. The "Test Integrity Check"
**Goal**: Find tests lacking assertions (fragile tests).
**Cmd**: `vecq -R tests/ -q '(.functions // [])[] | select(.name | contains("test")) | select(.content | contains("assert") | not) | .name' --grep-format`

### 4. The "Dependency Mapper"
**Goal**: Find all files importing a specific module.
**Cmd**: `vecq -R src/ -q '(.imports // [])[] | select(.path | contains("target_module")) | .path' --grep-format`

## DISCOVERY PROTOCOL
To avoid hallucinating schema structure, Agents MUST follow this protocol when encountering unmatched files:

1. **Introspect Filters**: Run `vecq list-filters` to see available jq tools.
2. **Introspect Elements**: Run `vecq elements <extension> --json` (e.g., `vecq elements md --json`) to see available AST nodes.
   - Example: `vecq elements rs --json` -> `["functions", "structs", "impls", ...]`

## AST REFERENCE (Common Nodes)
*Note: This list is for quick reference. Always authoritative source is `vecq elements <type>`.*

### Markdown (`.md`)
Run `vecq elements md` to see all available nodes. Common ones include:
- `.headers[]`: List of headers (`{ content, level, line_start }`)
- `.code_blocks[]`: Fenced code blocks (`{ language, content }`)
- `.links[]`: Hyperlinks (`{ text, url }`)

### Rust/Python/Go/C++
Run `vecq elements <ext>` to see available nodes. Common ones include:
- `.functions[]`: Function definitions (`{ name, signature, content }`)
- `.structs[]`: Class/Struct definitions (`{ name, content }`)
- `.imports[]`: Import statements
