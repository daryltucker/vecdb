# VecDB MCP Server Guide

> **Purpose**: Enable AI Agents (Claude, etc.) to semantically search, ingest, and reason about your codebase.

## 1. Quick Start

### 🚀 Claude Desktop (Legacy Stdio)
To use `vecdb` as a standard MCP server, you **MUST** specify the `--stdio` flag in your `claude_desktop_config.json`:

```json
{
  "mcpServers": {
    "vecdb": {
      "command": "vecdb-server",
      "args": ["--stdio"],
      "env": {
        "VECDB_PROFILE": "default",
        "VECDB_ALLOW_LOCAL_FS": "true"
      }
    }
  }
}
```

### ⚡ HTTP / JSON-RPC (Big Boi Mode)
By default, the server runs in HTTP mode for remote access and dashboards.

```bash
# Start server
vecdb-server --port 3000

# Test with curl
curl -X POST http://localhost:3000/ \
  -H "Content-Type: application/json" \
  -d '{"jsonrpc": "2.0", "method": "list_collections", "params": {}, "id": 1}'
```

> [!TIP]
> **Troubleshooting**: If the server hangs or you need to restart it:
> `pkill -9 vecdb-server`

---

## 2. Operating Modes

### HTTP Mode (Default)
Enables remote connectivity and future streaming capabilities.
- **Protocol**: JSON-RPC 2.0 over HTTP POST
- **Port**: Default 3000

### Stdio Mode
Required for localized MCP clients that spawn the server process directly.
- **Protocol**: JSON-RPC over stdin/stdout
- **Flag**: `--stdio`

---

## 3. Tools Available

| MCP Tool | CLI Equivalent | Description |
| :--- | :--- | :--- |
| `search_vectors` | `vecdb search` | Semantic search with smart routing support. |
| `ingest_path` | `vecdb ingest` | Ingest local files or directories. |
| `ingest_historic_version` | `vecdb history ingest` | Ingest a specific git revision (Time Travel). |
| `code_query` | `vecq <PATH> <QUERY>` | Structural analysis using Tree-sitter + JQ. |
| `list_collections` | `vecdb list` | List available collections and stats. |
| `delete_collection` | `vecdb delete` | Delete a collection with safety confirmation. |
| `embed` | N/A | Generate vectors from raw text. |

### `search_vectors`
Semantic search.
*   `query`: Natural language query (e.g., "How does authentication work?").
*   `collection`: (Optional) Target collection.
*   `profile`: (Optional) Profile to resolve collection from.
*   `smart`: (Optional, Boolean) Enable multi-hop semantic routing.

### `delete_collection`
Delete a collection. Requires implicit or explicit safety check.
*   `collection`: Name of the collection to delete.
*   `confirmation_code`: (Safety) Must be `{collection}-DELETE`.

### `code_query`
Structural code search/extraction using `vecq` syntax (jq-for-code).
*   `path`: File path.
*   `query`: JQ filter (e.g., `.functions[] | select(.name=="new")`).
*   `source`: "local" (Remote git support pending).
*   **Security**: Requires `VECDB_ALLOW_LOCAL_FS="true"`.

---

## 4. Configuration & Control

### Environment Variables

| Variable | Description | Default |
| :--- | :--- | :--- |
| `VECDB_PROFILE` | Selects the active profile from `config.toml`. | `default` |
| `VECDB_ALLOW_LOCAL_FS` | Enables tools to read the server's local filesystem. | `false` |
| `VECDB_CONFIG` | Overrides the location of the config file. | `~/.config/vecdb/config.toml` |
| `QDRANT_URL` | Overrides the Qdrant connection URL. | `http://localhost:6334` |

### CLI Flags

| Flag | Description |
| :--- | :--- |
| `--version` | Print version information. |
| `--allow-local-fs` | Enable local filesystem access. |
| `--stdio` | Force legacy stdio mode (required for local MCP). |
| `--port <PORT>` | Specify the HTTP port (default: 3000). |

---

## 5. Resources

The server exposes machine-readable resources for agents:
*   `vecdb://registry`: JSON summary of the server status and collections.
*   `vecdb://services`: Alias for registry.
*   `vecdb://manual`: The Agent Interface Specification (this guide).
*   `vecdb://collections/{name}`: Full metadata stats for a specific collection.

---

## 6. Profile & Collection Management

### Configuration
The MCP server loads **one** embedding model at startup.
1.  **Local Embedder**: If using `embedder_type="local"`, the model used is determined by the global `local_embedding_model` in `config.toml`.
2.  **Ollama**: If using `embedder_type="ollama"`, the server connects to an external Ollama instance.

### Switching Contexts
Agents can pass a `profile` argument to tools to resolve collections from other profiles, provided the vector dimensions are compatible. The `list_collections` tool returns an `is_compatible` flag to help agents avoid errors.
