# Querying & Filtering Guide

Learn how to extract exactly what you need from `vecdb` and `vecq`.

## 1. JSON & Pretty Printing

By default, `vecdb search` outputs a human-readable table. For programmatic use or raw inspection, use `--json`.

### Pretty Print (Colorized)
Pipe to `jq` or `bat` to see structured, colored JSON.

```bash
# Using jq (Standard)
vecdb search "my query" --json | jq .

# Using bat (If installed)
vecdb search "my query" --json | bat -l json
```

### Inspecting the First Result
Often you just want to see the shape of the data from the first match.

```bash
# Get the first element (index 0)
vecdb search "my query" --json | jq '.[0]'
```

## 2. Advanced Filtering with `vecq`

`vecq` allows you to query your source code structure using `jq` syntax.

### Basic Selection
```bash
# Find all functions in a file
vecq src/main.rs -q '.functions[]'
```

### Filtering by Attribute
```bash
# Find only public functions
vecq src/main.rs -q '.functions[] | select(.visibility == "pub")'
```

### Extraction (Raw Output)
Use `-r` (raw) to extract the actual code content without JSON quotes.

```bash
# Extract the content of all chat messages in a schema file
vecq examples/chat.json -q '.messages[].content' -r
```

## 3. Schema & Normalizers

`vecdb` uses "normalizers" to transform data (like OpenWebUI JSON) into a standard format.

### Example: Transforming Chat Data
If you have a `chat.json` from OpenWebUI:

```bash
# Normalize to canonical chat format and then format for reading
vecq -L examples/functions raw_chat.json -q 'include "openwebui_chat"; webui_to_chat | chat_format' -r
```

*   `include "openwebui_chat"`: Loads the filter library.
*   `webui_to_chat`: Converts input -> Canonical Chat.
*   `chat_format`: Converts Canonical Chat -> Markdown/Text for reading.
