# vecq - jq for source code

`vecq` is a command-line tool that turns any source code or structured text into queryable JSON, allowing you to use the powerful `jq` language to extract specific information.

**New in v0.1.0:** Enhanced CLI flags for Unix philosophy compatibility (`-f`, `-L`, `-q`).

## Purpose
Developers often need to answer questions about their codebase that `grep` cannot handle easily:
*   "List all public functions in this Rust file"
*   "Show me the headers of this Markdown file"
*   "Find all functions with 'Todo' in the name"

`vecq` parses the file structure (AST-lite) and outputs it as JSON, which can then be filtered, transformed, and formatted.

### Installation
`vecq` is part of the `vecdb-mcp` suite.
```bash
cargo install --git https://github.com/daryltucker/vecdb-mcp vecq
```
Or build from source:
```bash
cargo install --path vecq
```
`vecq` is the engine behind `vecdb ingest`, but it functions as a standalone tool.

## Basic Usage

### 1. View Structure
See what `vecq` sees in your file:
```bash
vecq src/main.rs
```
*Defaults to outputting the full JSON structure.*

### 2. Querying
Use `jq` syntax to filter the output.
```bash
# List all functions
vecq src/main.rs '.functions[] | .name'

# List public functions
vecq src/main.rs '.functions[] | select(.visibility == "pub") | .name'

# Load query from file
vecq src/main.rs -f queries/public_functions.jq

# Use custom library functions
vecq src/main.rs -L ~/.config/vecq/functions -q 'my_custom_filter'
```

> **Note on `-q` flag**: While you can pass the query as a positional argument (e.g., `vecq file.rs '.filter'`), using `-q` (`vecq file.rs -q '.filter'`) is recommended when piping input to avoid ambiguity.

### 3. Integration
Output in grep-compatible format for editor integration:
```bash
vecq src/ --grep-format -q '.functions[] | select(.name | contains("test"))'
```

### 4. Syntax Highlighting
Render files or piped content with syntax highlighting in the terminal:
```bash
# Highlight a specific file
vecq syntax README.md

# Pipe content (auto-detects or force language)
vecdb search "policy" | vecq syntax -l md
```

### 5. Documentation Generator
Generate clean Markdown documentation from any source code:
```bash
vecq doc src/lib.rs
```
*Note: This command uses the standardized `doc.jq` library embedded within `vecq`.*


## Supported Languages

```bash
$ vecq list-filters
```

```bash
$ vecq --list-types

Supported file types:
  Markdown (md, markdown)
  Rust (rs)
  Python (py, pyw)
  C (c, h)
  C++ (cpp, cc, cxx, hpp, hxx)
  CUDA (cu, cuh)
  Go (go)
  Bash (sh, bash)
  JSON (json, jsonl, ndjson)

```

### Man Page

`vecq` includes a built-in manual system oriented towards both humans and agents:

```bash
# General help
vecq help

# Human-readable manual
vecq man

# Agent-optimized technical reference (JSON/AST specs)
vecq man --agent
```

---
### AST Reference

To see the exact schema for a file, run `vecq --convert <file>`.

#### Markdown (`.md`)
- `.headers[]`: List of headers (`{ content, level, line_start }`)
- `.code_blocks[]`: Fenced code blocks (`{ language, content }`)
- `.links[]`: Hyperlinks (`{ text, url }`)

#### Rust/Python/Go/C++
- `.functions[]`: Function definitions (`{ name, signature, content }`)
- `.structs[]`: Class/Struct definitions (`{ name, content }`)
- `.imports[]`: Import statements
- `.comments[]`: Top-level comments

## Documentation
*   [Examples](EXAMPLES.md): Common recipes and query patterns.
*   [Configuration](CONFIG.md): Setting up custom functions and defaults.
*   [Functions](FUNCTIONS.md): Extending `vecq` with reusable macros.
