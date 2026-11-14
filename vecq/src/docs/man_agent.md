# AGENT INTERFACE SPECIFICATION: vecq
Version: this manual ships inside the binary — it describes the `vecq --version` you are running.

## PURPOSE
vecq lets an agent query source code and documents as JSON (an AST), with jq.

## WHEN TO REACH FOR IT
Before you grep for call sites, SQL or string literals across a repo; before you write a
script that parses source code; before you read a whole file to find one function. vecq
answers "where does X happen?" at function, call and string granularity.

- Not sure of the query? `vecq suggest "functions that call execute"`.
- Who calls or imports a name, across a tree? `vecq refs NAME PATH`.

## CAPABILITIES

### 1. Querying (`query`)
- **Action**: Parse source code and filter AST nodes using jq syntax.
- **Usage**: `vecq <INPUT>... -- <QUERY>` or `vecq <INPUT> -q <QUERY>`
- **Output**: JSON Lines or Array.
- **Recursion**: Use `-R` to recursively process a directory. This is the recommended default for directories. Hidden entries (`.git`, `.venv`) are not walked into.
- **Important**: Always use `--` to separate input files from the query, or use `-q` flag.
- **Common Queries**:
    - `.functions[]` : List all functions
    - `.structs[]` / `.classes[]` : List structs (Rust, C, Go) / classes (Python, JavaScript, C++)
    - `.code_blocks[]` : List markdown code blocks (useful for extraction)
    - `.imports[]` : List imports
- **Examples**:
    - `vecq -R src/ -q '.functions[]? | select(.attributes.is_public) | .attributes.signature' -r` (Public API)
    - `vecq README.md -q '.code_blocks[] | select(.attributes.language == "bash") | .attributes.code' -r` (Extract bash code)
    - `vecq src/main.rs -q '.functions | map(.name)'` (Using -q flag instead of --)
    - `vecq src/main.rs -L examples/functions -q 'my_lib::filter'` (Your own jq library)

### 2. Filtering & Manipulation (`select`, `map`)
- **`select(condition)`**: Keep nodes matching a predicate.
    - Example: `.functions[] | select(.attributes.visibility == "pub")`
- **`map(filter)`**: Apply transformation to each element.
    - Example: `.functions | map(.name)` -> Output list of function names.

### 3. Calls, strings and references (`calls[]`, `strings[]`, `refs`)
Every scope — function, method, class, impl, trait, mod, namespace — carries the calls and the
string literals directly inside it; what runs at top level is the **module scope**, `.module`.
Rust, Python, Go, C, C++, CUDA, Bash and JavaScript.

- **`calls[]`**: `{ callee, line, line_end, start, end, args[] }`
    - `callee` is as written, whitespace collapsed: `conn.execute`, `sqlite3.connect`,
      `crate::a::alpha`, `println!` (a Rust macro is a call, and calls inside its arguments are
      found too), `psql` (a shell command), `Pool` (JavaScript `new`). Not resolved by type:
      `x.alpha()` is a call to `x.alpha`.
    - `args[]`: `{ kind, value, keyword?, folded?, holes? }`, `kind` one of
      `string bytes name number tuple list dict call other`. A string's `value` is its text;
      anything else as written.
- **`strings[]`**: `{ kind, value, line, line_end, start, end, folded?, holes? }`, `kind` one of
  `plain fstring bytes docstring`.
    - **Folding**: a string built from parts is ONE string. `"UPDATE " + table + " SET x"` is
      `UPDATE {} SET x` with `holes: ["table"]`, `folded: true`; `"a" "b"` is `ab`; an f-string,
      a JavaScript template or a shell string's `$x` puts `{}` where each hole is.
    - Python values are decoded; every other language's are the text between the quotes as
      written (escapes not decoded).
- **`.module`**: `{ type: "module", name: "<module>", line_start, line_end, calls, strings }` —
  one object per file: a script's top-level work, `if __name__ == "__main__":` bodies.
- **`vecq refs <SYMBOL> [PATH]... [--json]`**: every call site and import of a name across
  files, with file, line, byte span and enclosing scope. Prints `file:line scope  kind name`.
    - Matching is by name: the symbol's segments are the reference's head or tail —
      `refs connect` and `refs sqlite3.connect` find `sqlite3.connect(..)`; `refs sqlite3` finds
      that call and `import sqlite3`. Segments, not substrings: `refs alpha` never matches
      `alphabet`, nor a comment or string that mentions it.
    - Directories are walked recursively, honouring `.vectorignore`, skipping hidden entries.
      A file that fails to parse is named on stderr and in `--json`'s `failed[]` — never silently
      skipped.
- **Examples**:
    - `vecq app.py -q '[.. | .calls? // empty | .[] | select(.callee|endswith("execute")) | .args[0].value] | unique'` (every SQL passed to execute)
    - `vecq -R src -q '[.. | .strings? // empty | .[] | select(.value|test("^\\s*SELECT";"i")) | .value]'` (SQL held in constants)
    - `vecq refs backends.connect src/ --json`

### 4. Codebase Mapping (`map`)
- **Action**: Analyze a directory and produce a structural graph (files, modules, symbols).
- **Usage**: `vecq map <PATH> [-d DEPTH] [-o FORMAT] [--no-gitignore] [--no-skip-hidden] [-s]`
- **Output Formats**:
    - `mermaid` (default): Mermaid diagram for documentation/IDEs
    - `json`: Full JGF v2 graph
    - `graph`: Pruned architectural graph
    - `human`: Readable summary with optional statistics
- **Use Cases**:
    - Generate architecture documentation before refactoring
    - Understand unfamiliar codebase structure
    - Visual dependency mapping for planning
- **Examples**:
    - `vecq map ./src -o mermaid > architecture.md`
    - `vecq map ./src -s` (show files analyzed/skipped)
    - `vecq map ./src -d 3` (limit depth to 3)

### 5. Conversion (`convert`)
- **Action**: Dump the full AST as JSON.
- **Usage**: `vecq --convert <INPUT>`

### 6. Introspection (`list-filters`, `elements`)
- **Action**: List tools and AST structures.
- **Usage**:
    - `vecq list-filters` : List available jq filters.
    - `vecq elements <EXT>` : List available AST elements for a language.

### 7. Documentation (`doc`)
- **Action**: Generate standardized Markdown documentation from semantic code structure.
- **Usage**: `vecq doc <INPUT>`
- **Output**: Pure Markdown (headers, code blocks, docstrings).
- **Mechanism**: Runs embedded `doc.jq` logic on strict Tree-sitter AST.

### 8. Normalization & Schemas (Unified Layer)
- **Action**: Transform disparate raw data (logs, issues, build errors) into canonical schemas.
- **Philosophy**: "Parse, Don't Validate". Lenient conversion to a shared contract.
- **Usage**: `vecq <INPUT> -q 'auto_normalize'` or specific functions like `nginx_to_log`.
- **Available Normalizers**:
    - **Logs**: `nginx_to_log`, `journald_to_log` -> `log.schema.json`
    - **Tasks**: `github_to_task`, `todo_to_task` -> `task.schema.json`
    - **Artifacts**: `cargo_to_artifact`, `junit_to_artifact` -> `artifact.schema.json`
    - **Diffs**: `git_diff_to_diff` -> `diff.schema.json`
- **Auto-Normalization**: Use `auto_normalize` to heuristically detect and convert input.

## THE ELEMENT CONTRACT
What every element of every file type promises — the same for Rust and for Markdown, so a
tool can edit a file by element without a per-language exception.

- **`content`** is the element's source text exactly as written: not rendered, not
  re-serialised, no trailing line break. A method's `content` starts at its first character
  (`pub fn …`), after its indent; a JSON pair is `"key": value` as in the file; a Markdown
  paragraph keeps its `**`, backticks and link syntax; a code block keeps its fences.
- **`line_start` / `line_end`** are **1-indexed and inclusive** in every language: the line of
  the element's first byte and the line of its last. Lines `line_start..=line_end`, cut at the
  element's first and last column, are `content`. Blank lines after an element are not its
  lines.
- **`attributes.kind`**: the grammar's own node kind, on elements the tree-sitter languages
  emit for what the walker does not name (`expression_statement`, `preproc_ifdef`, …).
    - **`"ERROR"`**: text the grammar could not parse. It is kept, spanned and embedded — never
      dropped. `metadata.syntax_errors` counts the ERROR and MISSING nodes in the file; `0` is a
      clean parse.
    - **`attributes.frame: true`**: a wrapper's own text outside its children — `#ifdef X` and
      `#endif` around the functions they guard.
    - Python is parsed by rustpython, which has no error recovery: a Python file with a syntax
      error fails to parse (an error, never an empty result).
- **`--source-info`** adds `"source": { start, end, leading, trailing }` to every element:
  byte offsets into the file (UTF-8, `end` exclusive) and the trivia the element owns —
  blank lines and the comments that describe it before, the rest of its line after.
  Concatenating every top-level element's `leading + source text + trailing` reproduces the
  file byte for byte. Off by default (positions are for tools that edit, not for reading).
    - `vecq src/lib.rs --source-info -q '.functions[] | {name, source}'`
- **Comments belong to what they describe**: an own-line comment leads the element after it
  (a method's doc comment is the method's, not the impl's); a same-line comment trails the
  element before it.

### Depth: what an agent can reach, per language
This release is **declaration-deep** for code. Every declaration at file level, and every
declaration inside a container (impl, class, struct), is an element with its own span.
**Function bodies are opaque**: a local, a statement or a value inside a body is part of the
function's `content` and is not addressable by selector. Calls and string literals inside a
body are reachable as spans on the enclosing scope (`calls[]`, `strings[]`), not as elements.
To edit inside a body, anchor on text within the function's `content`; statement-level
elements are the next campaign. The table below is generated by `vecq/tests/tier1_depth.rs`
from `vecq/tests/fixtures/depth/`, and that test fails whenever the binary and this table
disagree, in either direction.

Cells: `element` own element, own span, reachable by selector · `attribute` data on the
enclosing element, no span of its own · `span-only` a span in `calls[]`/`strings[]`, not an
element · `opaque` inside its parent's `content` · `—` the language has no such construct at
that level. Levels: `file` · `container body` (impl/class/struct) · `function body` · `block`
(an `if` inside a function). Kinds: `variable` (a binding) · `function` (at container level a
method; inside a body a nested function) · `statement` (a non-binding statement) · `call` ·
`string` · `value` (the right-hand side of the binding at that level).

<!-- depth-table: generated by vecq/tests/tier1_depth.rs — paste the test's output, do not edit by hand -->
**Rust**

| level | variable | function | statement | call | string | value |
|---|---|---|---|---|---|---|
| file | element | element | — | — | — | opaque |
| container body | element | element | — | span-only | span-only | opaque |
| function body | opaque | opaque | opaque | span-only | span-only | opaque |
| block | opaque | — | opaque | span-only | span-only | opaque |

**Python**

| level | variable | function | statement | call | string | value |
|---|---|---|---|---|---|---|
| file | element | element | element | span-only | span-only | attribute |
| container body | opaque | element | — | span-only | span-only | opaque |
| function body | opaque | opaque | opaque | span-only | span-only | opaque |
| block | opaque | — | opaque | span-only | span-only | opaque |

**JavaScript**

| level | variable | function | statement | call | string | value |
|---|---|---|---|---|---|---|
| file | element | element | element | span-only | span-only | opaque |
| container body | opaque | element | — | span-only | span-only | opaque |
| function body | opaque | opaque | opaque | span-only | span-only | opaque |
| block | opaque | — | opaque | span-only | span-only | opaque |

**Go**

| level | variable | function | statement | call | string | value |
|---|---|---|---|---|---|---|
| file | element | element | — | — | — | opaque |
| container body | opaque | element | — | span-only | span-only | — |
| function body | opaque | — | opaque | span-only | span-only | opaque |
| block | opaque | — | opaque | span-only | span-only | opaque |

**C**

| level | variable | function | statement | call | string | value |
|---|---|---|---|---|---|---|
| file | element | element | — | — | — | opaque |
| container body | element | — | — | — | — | — |
| function body | opaque | — | opaque | span-only | span-only | opaque |
| block | opaque | — | opaque | span-only | span-only | opaque |

**C++, CUDA**

| level | variable | function | statement | call | string | value |
|---|---|---|---|---|---|---|
| file | element | element | — | — | — | opaque |
| container body | opaque | element | — | span-only | span-only | opaque |
| function body | opaque | — | opaque | span-only | span-only | opaque |
| block | opaque | — | opaque | span-only | span-only | opaque |

**Bash**

| level | variable | function | statement | call | string | value |
|---|---|---|---|---|---|---|
| file | element | element | element | span-only | span-only | opaque |
| container body | — | — | — | — | — | — |
| function body | opaque | opaque | opaque | span-only | span-only | opaque |
| block | opaque | — | opaque | span-only | span-only | opaque |
<!-- /depth-table -->

Reading the table: the container-level `variable` is a struct field or class attribute; it is an
element only in C (and in Rust for an associated `const` in an `impl`, which is what the Rust
row probes — Rust struct fields are opaque). A `function` at container level is a method, which
is an element everywhere it exists; a method's own locals are opaque like any body.

## PROTOCOL COMPLIANCE
- **Errors**: Printed to STDERR, exit status non-zero.
- **Success**: Printed to STDOUT.
- **Format**: All data output is valid JSON (unless overridden by --grep-format or -r).
- **A failing query fails**: a jq runtime error (`.functions[]` on a file with no functions,
  indexing a string) is reported with the file it failed on and the run exits non-zero; the
  other inputs still produce output. Say "absent is fine" in the query: `.functions[]?` or
  `(.functions // [])[]`.

## OPTIONS
- `-f, --from-file <PATH>`: Read query filter from file
- `-q, --query <QUERY>`: Specify query filter as flag (useful for piped input)
- `-L, --library-path <PATH>`: Add directory to search for library modules (.jq files)
- `-o, --format <FORMAT>`: Output format (json, grep, human)
- `-s, --slurp`: Read all inputs into array before querying (Required when piping multiple files if you want to treat them as a single dataset).
- `--grep-format`: Shortcut for `-o grep` (Recommended for search tasks)
- `-r, --raw-output`: Output raw strings, not JSON text (Critical for code extraction)
- `-R, --recursive`: Recursively process directories
- `-C, --context-lines <N>`: Add N lines of context before/after each element
- `--no-content`: Leave every element's `content` out — names, signatures, lines, calls and strings only. Function bodies are most of the output when you want signatures.
- `--source-info`: Add each element's byte span and owned trivia (see THE ELEMENT CONTRACT)

## TROUBLESHOOTING

### Common Issues
- **"Input path does not exist"**: This usually means your query syntax is being interpreted as a file path. Always use `--` to separate files from queries: `vecq file.rs -- '.query'`
- **"jq error: cannot use null as iterable"**: The file has no such key (a Markdown file has no `.functions`). Use `.functions[]?`.
- **No output**: Check if your query path exists. Use `--convert` first to see the JSON structure.
- **Query syntax errors**: Use `--explain` with a simple query to debug: `vecq /dev/null -- '. | keys'`
- **Pipeline issues**: When using xargs, use `-I {}` syntax: `find . -name "*.rs" | xargs -I {} vecq {} -- '.query'`

### Getting Help
- `vecq --help`: Basic usage and options
- `vecq man --agent`: Complete agent interface specification (this document)
- `vecq elements <language>`: See what structural elements are available for parsing
- `vecq suggest "natural language description"`: Get query suggestions

## CONTEXT
Use this tool when you need to:
1. Understand the structure of a file without reading the whole text.
2. Find specific code elements (e.g. "all public functions") reliably.
3. Find every call site, SQL statement or string literal without a regex over source.
4. Extract executable code blocks from documentation.
5. Generate architecture diagrams for documentation.

## RECIPES (Structural Analysis)
Common Agentic workflows for codebase understanding.

### 1. The "API Surface Auditor"
**Goal**: List public functions to understand the external interface.
**Cmd**: `vecq -R src/ -q '.functions[]? | select(.attributes.is_public) | {name, signature: .attributes.signature}' --compact`

### 2. The "Complexity Hunter"
**Goal**: Find functions > 50 lines (prime candidates for bugs/refactoring).
**Cmd**: `vecq -R src/ -q '.functions[]? | select((.line_end - .line_start) > 50) | {name, lines: (.line_end - .line_start + 1)}' --compact`

### 3. The "Test Integrity Check"
**Goal**: Find tests lacking assertions (fragile tests).
**Cmd**: `vecq -R tests/ -q '.functions[]? | select(.name | contains("test")) | select([.calls[].callee | test("assert")] | any | not) | .name' --grep-format`

### 4. The "Dependency Mapper"
**Goal**: Find every file importing a specific module.
**Cmd**: `vecq refs target_module src/` (imports and call sites), or `vecq -R src/ -q '.imports[]? | select(.content | contains("target_module")) | .content' --grep-format`

### 5. The "SQL Inventory"
**Goal**: Every SQL statement passed to a call, with its function and line.
**Cmd**: `vecq -R src/ -q '.functions[]? | .name as $f | .calls[] | select(.args[0].kind == "string") | {function: $f, line, sql: .args[0].value}' --compact`

### 6. The "Architecture Explorer"
**Goal**: Generate a visual map of the codebase structure.
**Cmd**: `vecq map ./src -o mermaid > architecture.md`

## DISCOVERY PROTOCOL
To avoid hallucinating schema structure, Agents MUST follow this protocol when encountering unmatched files:

1. **Introspect Filters**: Run `vecq list-filters` to see available jq tools.
2. **Introspect Elements**: Run `vecq elements <extension> --json` (e.g., `vecq elements md --json`) to see available AST nodes.
   - Example: `vecq elements rs --json` -> `["functions", "structs", "implementations", ...]`
3. **Deep Probe**: Run `vecq <file> -q 'keys'`, then `vecq <file> -q '.functions[0] | keys'` to see a node's fields.

## AST REFERENCE (Common Nodes)
*Note: This list is for quick reference. Always authoritative source is `vecq elements <type>`.*

### Markdown (`.md`)
Run `vecq elements md` to see all available nodes. Common ones include:
- `.headers[]`: Headers (`content` is `## Install` as written; `attributes.level`)
- `.paragraphs[]`: Paragraphs (`content` as written; `attributes.text` is the rendered prose)
- `.code_blocks[]`: Code blocks (`content` with its fences; `attributes.code` is the code alone; `attributes.language`)
- `.links[]` / `.images[]`: `content` is the markup as written; `attributes.url`, `attributes.title`
- `.list_items[]`: List items (`attributes.task`, `attributes.checked` for `- [ ]` / `- [x]`)
- Every element has `.attributes.heading_path`: the headings it lies under,
  outermost first (`["X", "Y", "Tasks"]`); a heading's own path is its
  ancestors'. Select by section without counting lines:
  `.paragraphs[] | select(.attributes.heading_path == ["Guide", "Install"])`

### Rust/Python/Go/C/C++/CUDA/Bash/JavaScript
Run `vecq elements <ext>` to see available nodes. Common ones include:
- `.functions[]`: Function definitions (`{ name, content, attributes.signature, attributes.is_public, calls, strings }`). A method is listed here and as a child of its class/impl (`crumbtrail` names the parent).
- `.structs[]` / `.classes[]`: Struct / class definitions, methods in `.children[]`
- `.imports[]`: Import statements (`attributes.module` where the language names one)
- `.blocks[]`: Everything else the grammar parsed, with `attributes.kind` (see THE ELEMENT CONTRACT)
- `.variables[]`: Variables, constants and assignments — one key in every language (Rust
  splits `.constants[]` / `.statics[]`; TOML/YAML `.entries[]`, JSON `.pairs[]`). Every list
  key is plural. Python carries `attributes.value` — the right-hand side rendered from the
  AST (a literal, name or attribute path, spelled as Python: `True`, `None`), `"..."` for
  calls and complex expressions; the call itself is in `.module.calls`. Other languages do
  not carry the value yet.
    - `vecq probe.py -q '.variables[] | select(.name == "PAGE_SQL") | .attributes.value'`
- **JavaScript** (`.js .mjs .cjs .jsx`; not TypeScript): `export` declarations are functions and
  classes (`attributes.is_exported`, which is also `is_public`); `const f = () => …` is a
  function named `f` (`attributes.is_arrow`); `.variables[]` holds other bindings.

### Python: module-level executable statements (`.blocks[]`, `.module`)
Module code that is not a def/class/import/top-level assignment — `if
__name__ == "__main__":` blocks, setup calls — is never dropped. Each
statement is a `.blocks[]` element, as written, with its
`attributes.kind` (`expression_statement`, `if_statement`, …); what those
statements call is `.module.calls`.
- `vecq script.py -q '.blocks[].content' -r` — read what runs at import time
  without opening the file.

### JSON (`.json`, `.ndjson`, `.jsonl`)
Run `vecq elements json` to see available nodes:
- `.pairs[]`: Scalar key-value entries (`content` is `"key": value` as written)
- `.objects[]`: Nested object/array nodes with `.children[]`

**Cross-language query pattern** — works across `.rs`, `.py`, `.json` in one pass:
```bash
vecq -R src/ -- '(.functions // .pairs // empty)[] | .name'
```

**Get a specific value by key:**
```bash
vecq package.json -- '.pairs[] | select(.name == "version") | .content'
```

**List all top-level keys:**
```bash
vecq config.json -- '.pairs[] | .name'
```

### TOML and YAML
- `.tables[]`: TOML tables, flat, named by their full dotted path (`dependencies.serde`); YAML mappings and sequences
- `.entries[]`: Key-value entries (`content` is the line as written, comments owned)

## THE VECQ MINDSET
1.  **Filter Aggressively**: Use `| .[0:5]`, specific `select()` filters, or `--no-content` to keep JSON output small.
2.  **Trust the Crumbtrail**: If an element has a `crumbtrail`, it indicates hierarchical context (e.g., inside an `impl` block or `class`).
3.  **Raw for Extraction**: Always use `-r` (raw-output) when extracting code blocks or function bodies to avoid escaped JSON strings.
4.  **Calls before grep**: `calls[]`, `strings[]` and `vecq refs` see the code, not its comments.

---
*Generated by: Sextant*
