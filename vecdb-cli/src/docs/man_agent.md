# AGENT INTERFACE SPECIFICATION

## SYNOPSIS
`vecdb [COMMAND] [OPTIONS]`

## DESCRIPTION
vecdb is a Vector Database wrapper designed for Agentic interactions.
It abstracts connection details via Profiles and provides simple CLI tools for Ingestion and Search.

## COMMANDS

### ingest
Ingest a file or directory into the vector store.
`vecdb ingest [PATH] [OPTIONS]`

**Options:**
- `-c, --collection [NAME]`: Target collection. Optional (defaults to profile setting).
- `--chunk-size [INT]`: Max tokens per chunk (default: 1000).
- `--respect-gitignore`: Skips files ignored by .gitignore.
- `--extensions [LIST]`: Whitelist e.g. "rs,md".
- `--excludes [LIST]`: Blacklist globs e.g. "*.tmp".
- `--metadata [K=V]`: Attach metadata.

**Agent Usage:**
`vecdb ingest ./src`
(Use default collection defined in profile, typically 'docs' or project specific)

### search
Semantic search against the vector store.
`vecdb search [QUERY] [OPTIONS]`

**Options:**
- `-c, --collection [NAME]`: Source collection. Optional.
- `--json`: Output as JSON for parsing.
- `--smart`: Use smart routing (multi-hop / filter detection).

**Agent Usage:**
`vecdb search "authentication implementation" --json`

### list
List available collections and their statistics.

### delete
Delete a collection (requires confirmation).
`vecdb delete [COLLECTION] --yes`

## MCP SERVER CAPABILITIES
When running as an MCP Server (`vecdb-server`), the following additional tools are available via JSON-RPC:

### code_query
Query source code structure using vecq syntax.
`code_query(path, query, source="git"|"local")`
- **Security**: `source="git"` relies on ephemeral sandboxes and is always allowed. `source="local"` requires configuring `VECDB_ALLOW_LOCAL_FS="true"`.
- **Note**: This is powered by `vecq`. See `vecq man --agent` for query syntax.

### ingest
* **ingest** [PATH] [--respect-gitignore] [--chunk-size N] [--overlap M]
        Recursively ingest documents from a path.
        Metadata is automatically extracted (path, extension, etc).

### ingest_historic_version
Ingest a specific git commit or tag.
`ingest_historic_version(repo_path, git_ref, collection)`
- **Use Case**: "Time Travel" debugging.

### ingest_path
Ingest local files/folders.
`ingest_path(path, collection)`
- **Security**: Requires `VECDB_ALLOW_LOCAL_FS="true"`.

### search_vectors
Semantic search against vector collections.
`search_vectors(query, collection=null, profile=null, smart=false, json=false)`

### list_collections
List all available vector collections with metadata.
`list_collections()`

### delete_collection
Delete a collection (requires confirmation).
`delete_collection(collection, confirmation_code)`

### embed
Generate embeddings for raw text.
`embed(texts)`

## CONFIGURATION
Configuration is loaded from `~/.config/vecdb/config.toml`.
Profiles define connection details (Qdrant URL, Ollama URL, default collection).

## EXAMPLES

1. **Ingest Project**:
   `vecdb ingest .`

2. **Search for Code**:
   `vecdb search "database connection" --json`

3. **Check Status**:
   `vecdb list`
