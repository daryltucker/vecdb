q# User Defined Functions (Macros)

`vecq` allows you to define reusable `jq` functions. This helps complex queries become readable, reusable "macros".

## Quick Start

1.  **Location**: `~/.config/vecq/functions/` (create if it doesn't exist).
2.  **File Extension**: Must end in `.jq`.
3.  **Loading**: All `.jq` files in that directory are automatically loaded before every query.
4.  **Custom Paths**: You can also use `-L path/to/funcs` to load from other directories.
5.  **From File**: You can use `-f path/to/query.jq` to load the main query from a file.

## Built-in Functions (Prelude)

These functions are always available in every query:

*   `empty`: Produces no output (equivalent to `[] | .[]`).
*   `select(f)`: Standard jq select function.
*   `map(f)`: Standard jq map function.

## Built-in Standard Library

These modules are compiled into the `vecq` binary and are always available. You do not need `-L` or `include` to use them.

### 1. Unified Schema Layer
Canonical normalizers for common data types.
*   **Logs** (`log.jq`): `nginx_to_log`, `journald_to_log`
*   **Tasks** (`task.jq`): `github_to_task`, `todo_to_task`
*   **Artifacts** (`artifact.jq`): `cargo_to_artifact`
*   **Diffs** (`diff.jq`): `git_status_to_diff`
*   **Auto**: `auto_normalize` (Heuristic detection)
*   **Graph** (`graph_src.jq`, `graph.jq`): `src_to_graph`, `graph_format`, `graph_format_mermaid`
*   **Architecture** (`architecture.jq`): `src_to_architecture`, `architecture_format`, `architecture_format_mermaid`

> **Note**: For more details on the schemas and their fields, see [schemas/README.md](../../schemas/README.md).

### 2. Documentation (`doc.jq`)
Generates Markdown documentation from AST.
*   `markdown`: Main entry point.
*   `_clean_doc`: Helper.

## Available Macro Libraries (Examples)

The following macro libraries are provided in the project root `examples/functions/` directory and can be used with `-L`.

### 1. Chat Formatting (`chat_format.jq`)
Canonical renderer for chat schemas.
*   `chat_format`: Main entry point (renders array to Markdown).
*   `chat_tail(n)`: Get last `n` messages.
*   `chat_search(pattern)`: Search messages by content.
*   `chat_filter_role(role)`: Filter by "user", "assistant", etc.

### 2. File Trees (`tree.jq`)
Helpers for processing `tree -J` output.
*   `walk_tree`: Recursive descent through tree structure.
*   `paths`: Map entries to full path strings.
*   `files` / `dirs`: Filter for specific entry types.
*   `find_item(pattern)`: Search for nodes by name.

### 3. OpenWebUI (`openwebui_chat.jq`)
Normalizer for OpenWebUI exports. Designed to be piped into `chat_format`.
*   `webui_to_chat`: Transform OpenWebUI export array to canonical chat schema.
*   `webui_conversation_to_chat`: Helper for single conversation objects.

### 4. GitHub Issues (`gh_issue.jq`)
Transform GitHub API JSON responses into clean Markdown reports.
*   `gh_issue`: Renders a single issue with title, metadata matches, and body.

### 5. NPM Audit (`npm_audit.jq`)
Turn `npm audit --json` into a developer-friendly security report.
*   `audit_summary`: Table of High/Critical vulnerabilities with fix suggestions.

### 6. VS Code Setup (`vscode_ext.jq`)
Generate setup scripts from project configuration JSON.
*   `vscode_install_script`: Converts `.vscode/extensions.json` into a bash script to install all recommended extensions.

### 7. Lighthouse (`lighthouse.jq`)
Extract Web Vitals and scores from Lighthouse JSON.
*   `lighthouse_badges`: One-line summary (e.g., "🟢 Performance: 98 | 🟢 Accessibility: 100").
*   `lighthouse_table`: Detailed Markdown table of scores.

**Example Usage**:
```bash
# Security Check
npm audit --json | vecq -L examples/functions -q 'audit_summary'

# Generate Setup Script
vecq -L examples/functions .vscode/extensions.json -q 'vscode_install_script' -r > setup.sh
```

```bash
# Convert OpenWebUI export to Markdown using the chat_format renderer
vecq export.json -L examples/functions -q 'openwebui_to_chat | chat_format'
```

---

## Example: OpenWebUI Chat

We have provided an example function set in `examples/functions/openwebui_chat.jq`. This macro is designed to process JSON exports from OpenWebUI chat history and format them into readable Markdown.

### The Macro Code (`openwebui_chat.jq`)

```jq
# Decode common HTML entities
def decode_html:
  gsub("&gt;"; ">") |
  gsub("&lt;"; "<") |
  gsub("&quot;"; "\"") |
  gsub("&#x27;"; "'") |
  gsub("&#x60;"; "`") |
  gsub("&amp;"; "&"); 

# Helper to handle content that might be string or array of strings
def get_content: 
  if (.content | type) == "array" then 
    (.content | join("")) 
  else 
    (.content // "") 
  end | decode_html;

# Format a single message with role header
def format_msg: 
  "### " + (.role // "System") + "\n" + get_content + "\n"; 

# Format the entire chat history
def format_chat: 
  .[] | 
  ("# " + .title + "\n"), 
  (.chat.messages[] | format_msg), 
  "\n---\n";
```

### How to Use It

1.  **Install**: Copy the file to your config directory.
    ```bash
    mkdir -p ~/.config/vecq/functions
    cp vecq/examples/functions/openwebui_chat.jq ~/.config/vecq/functions/
    ```

2.  **Run**: Now you can use `format_chat` as if it were a built-in `jq` command.
    ```bash
    vecq export.json 'format_chat' --raw-output
    ```

3.  **Use Locally (-L)**: Or keep it in your project and load explicitly.
    ```bash
    vecq -L ./examples/functions -q 'format_chat' -r export.json
    ```

**Output**:
```markdown
# My Chat Title
### User
Hello, how are you?

### Assistant
I am doing well, thank you!
...
```

## Best Practices & Limitations

### Definition Order (Strict strict)
Unlike some other `jq` implementations, the `jaq` engine used by `vecq` requires **strict definition-before-use ordering**.

*   **Correct**: Define helpers *before* the functions that call them.
*   **Incorrect**: Defining a helper function at the bottom of the file.

```jq
# WRONG
def main: helper;
def helper: "I am a helper";

# RIGHT
def helper: "I am a helper";
def main: helper;
```

### Module System
Use `import "filename" as name;` to load functions from other files. The filename is relative to the directory specified by `-L` (or the default `~/.config/vecq/functions`).

## Creating Your Own

You can create macros for anything. For example, a Rust helper:

**`~/.config/vecq/functions/rust.jq`**
```jq
# Select public items
def pub: select(.visibility == "pub");

# Select items with specific return type
def returns(t): select(.return_type == t);
```

**Usage**:
```bash
vecq src/lib.rs '.functions[] | pub | returns("Result<()>")'
```
