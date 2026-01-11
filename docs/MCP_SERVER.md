# VecDB MCP Server Guide

> **Purpose**: Enable AI Agents (Claude, etc.) to semantically search, ingest, and reason about your codebase.

## 1. Quick Start (Claude Desktop)

Add the following to your `~/Library/Application Support/Claude/claude_desktop_config.json` (macOS) or `%APPDATA%\Claude\claude_desktop_config.json` (Windows) or `~/.config/Claude/claude_desktop_config.json` (Linux):

```json
{
  "mcpServers": {
    "vecdb": {
      "command": "vecdb-server",
      "env": {
        "VECDB_PROFILE": "default"
      }
    }
  }
}
```

### Enable Local Filesystem Access (Sandbox Escape)

By default, `vecdb` blocks agents from reading local files (except specific tools like `code_query` on git repos). To allow `ingest_path` on local directories:

```json
      "env": {
        "VECDB_ALLOW_LOCAL_FS": "true" 
      }
```

## 2. Tools Available

### `search_vectors`
Semantic search.
*   `query`: Natural language query (e.g., "How does authentication work?").
*   `collection`: (Optional) Target collection.
*   `profile`: (Optional) Profile to resolve collection from.
*   `smart`: (Optional, Boolean) Enable multi-hop semantic routing.
*   `json`: (Optional, Boolean) Output results as a pure JSON object instead of a human-readable summary.

### `delete_collection`
Delete a collection. Requires implicit or explicit safety check.
*   `collection`: Name of the collection to delete.
*   `confirmation_code`: (Safety) Must be `{collection}-DELETE`.
*   **Workflow**: Call once without specific code -> Server returns error with required code -> Call again with code.


### `code_query`
Structural code search/extraction using `vecq` syntax (jq-for-code).
*   `path`: File path.
*   `query`: JQ filter (e.g., `.functions[] | select(.name=="new")`).
*   `source`: "local" or "git".
*   `repo_path`: (If source="git") URL of the repo.
*   **Security**: `source="git"` relies on ephemeral sandboxes and is **always allowed**. `source="local"` requires `VECDB_ALLOW_LOCAL_FS="true"`.

### `ingest_path`
Ingest local files/folders.
*   `path`: Absolute path.
*   `profile`: (Optional) Profile to resolve collection from.
*   **Security**: Requires `VECDB_ALLOW_LOCAL_FS="true"`.

### `list_collections`
List available collections with compatibility status.
*   **Returns**:
    *   `name`, `count`, `dimension`
    *   `is_active`: Is this the default for the current profile?
    *   `is_compatible`: Does the dimension match the server's loaded embedder?

### `embed`
Generate embeddings for raw text.
*   `texts`: Array of strings to embed.
*   **Returns**: Array of float vectors.

## 3. Resources
The server exposes the following read-only resources:
*   `vecdb://registry`: A machine-readable JSON summary of the server (Profile, Version, Collection List).
*   `vecdb://manual`: The Agent Interface Specification (this guide).
*   `vecdb://collections/{name}`: Returns the full metadata stats for a specific collection as JSON.

## 4. Profile & Collection Management

### Configuration
The MCP server loads **one** embedding model at startup.
1.  **Local Embedder**: If using `embedder_type="local"`, the model used is determined by the global `local_embedding_model` in `config.toml`. Only ONE local model can be loaded per process.
2.  **Ollama**: If using `embedder_type="ollama"`, the server connects to the external Ollama instance.

### Switching Profiles
You don't need to restart the server to switch contexts!
*   **Default**: Server uses `VECDB_PROFILE` or `default_profile` from config.
*   **Agent Control**: Agents can pass a `profile` argument to tools (`search_vectors`, `ingest_path`, etc.) to resolve collections from other profiles.
    *   *Constraint*: The target profile's collection must hold vectors compatible with the server's running embedder (Dimension Check).
    *   *Safety*: The `list_collections` tool returns an `is_compatible` flag to warn agents about dimension mismatches.

### Example Workflow
1.  Agent calls `list_collections` -> sees `docs` (compatible) and `docs_qwen` (incompatible).
2.  Agent searches `docs`: `search_vectors(query="...")`
3.  Agent attempts `docs_qwen`: `search_vectors(collection="docs_qwen")` -> **Error**: "Vector dimension error".
