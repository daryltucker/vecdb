# Examples

This directory contains example resources for learning and testing `vecdb` and `vecq`.

## Qdrant

In order to use `vecdb`, you must have a Qdrant instance available.  The easiest way is to use the `docker-compose-qdrant.yml` example.
If you wish to add Ollama capabilities and do not already have Ollama, you can uncomment the Ollama portion.

```bash
cd examples/
docker compose -f docker-compose-qdrant.yml up -d
```


## Functions

### Structure

*   **`functions/`**: Reusable `jq` macro libraries for `vecq`.
    *   `chat_format.jq`: Renders chat logs to Markdown.
    *   `gh_issue.jq`: Formats GitHub issue JSON.
    *   `tree.jq`: Navigates JSON file trees.
    *   (And more...)

### Usage

You can use these functions by passing the directory to `vecq` via the `-L` (library path) flag:

```bash
# Use the Chat Formatter
vecq -L examples/functions data.json -q 'openwebui_to_chat | chat_format'

# Use a custom schema normalizer (overriding the built-in one)
vecq -L examples/functions/schemas data.json -q 'include "log"; nginx_to_log'
```

> **Note**: The functions in `schemas/` are **built-in** to the `vecq` binary as strict standard libraries. You do not need to import them manually unless you want to override them or inspect their source code.
