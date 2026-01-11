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
*   `source`: "local" (Remote git support pending).
*   **Security**: Requires `VECDB_ALLOW_LOCAL_FS="true"`.

### `ingest_path`
Ingest local files/folders.
*   `path`: Absolute path.
*   `collection`: (Optional) Target collection name.
*   `profile`: (Optional) Profile to resolve collection from.
*   **Security**: Requires `VECDB_ALLOW_LOCAL_FS="true"`.

### `ingest_historic_version`
Ingest a specific git revision ('Time Travel').
*   `repo_path`: URL or local path to git repository.
*   `git_ref`: Tag, Branch, or SHA.
*   `collection`: (Optional) Target collection name.

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

> [!NOTE]
> **Snapshot Management** (backup/restore) is currently only available via the `vecdb snapshot` CLI command.

## 3. Configuration & Control

The server can be configured via Environment Variables and CLI Flags.

### Environment Variables

| Variable | Description | Default |
| :--- | :--- | :--- |
| `VECDB_PROFILE` | Selects the active profile from `config.toml`. | `default` |
| `VECDB_ALLOW_LOCAL_FS` | Enables tools like `ingest_path` to read the server's local filesystem. | `false` |
| `VECDB_CONFIG` | Overrides the location of the config file. | `~/.config/vecdb/config.toml` |
| `QDRANT_URL` | Overrides the Qdrant connection URL (if not in profile). | `http://localhost:6334` |
| `QDRANT_API_KEY` | Overrides Qdrant API Key. | None |

### CLI Flags

- `--version`: Print version information.
- `--allow-local-fs`: Enable local filesystem access (Same as `VECDB_ALLOW_LOCAL_FS=true`).

### Security Note
By default, `vecdb-server` runs in **API-Only Mode**. It blocks Agents from reading arbitrary system files to prevent sandbox escapes. To use tools like `ingest_path` or `code_query` (with `source='local'`), you must explicitly enable filesystem access.

## 4. Resources
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
