# AGENT INTERFACE SPECIFICATION

## SYNOPSIS
`vecdb [COMMAND] [OPTIONS]`

## DESCRIPTION
vecdb is a Vector Database wrapper designed for Agentic interactions.
It abstracts connection details via Profiles and provides simple CLI tools for Ingestion and Search.

## AGENT CHEATSHEET

### 1. Ingest & Memorize
**Goal**: Quickly learn a new repository.
1. `vecdb ingest ./src --collection project_x --respect-gitignore`
2. `vecdb list` (Verify vectors exist)

### 2. Semantic Search
**Goal**: Find concepts when keywords fail.
*   `vecdb search "authentication logic" --collection project_x --json`
*   `vecdb search "memory leak patterns" --collection project_x --json`

### 3. Optimizing for Accuracy
**Goal**: Ensure best search performance.
1. `vecdb config set-quantization project_x binary` (Fastest) OR `scalar` (Balanced)
2. `vecdb optimize project_x`

## COMMANDS

### ingest
Ingest a file or directory into the vector store.
`vecdb ingest [PATH] [OPTIONS]`

**Options:**
- `-c, --collection [NAME]`: Target collection. Optional (defaults to profile setting).
- `--profile [NAME]`: Profile to use from config.toml.
- `--chunk-size [INT]`: Max tokens per chunk (default: 1000).
- `-o, --overlap [INT]`: Chunk overlap (default: 0).
- `--respect-gitignore`: Skips files ignored by .gitignore.
- `--extensions [LIST]`: Whitelist e.g. "rs,md".
- `--excludes [LIST]`: Blacklist globs e.g. "*.tmp".
- `--metadata [K=V]`: Attach metadata (can be used multiple times).
- `--dry-run`: List files that would be ingested without processing.
- `-P, --concurrency [INT]`: Max concurrent file processing tasks.
- `-G, --gpu-concurrency [INT]`: Max concurrent GPU embedding tasks (batch size).

**Agent Usage:**
`vecdb ingest ./src`
(Use default collection defined in profile, typically 'docs' or project specific)

### search
Semantic search against the vector store.
`vecdb search [QUERY] [OPTIONS]`

**Options:**
- `-c, --collection [NAME]`: Source collection. Optional.
- `--profile [NAME]`: Profile to use from config.toml.
- `--json`: Output as JSON for parsing.
- `--smart`: Use smart routing (multi-hop / filter detection).

**Agent Usage:**
`vecdb search "authentication implementation" --json`

### list
List available collections and their statistics.
Warns if collection size exceeds 1GB, suggesting optimization.

### config
Manage configuration settings.
`vecdb config set-quantization [COLLECTION] [scalar|binary|none]`
- Sets the quantization CONFIGURATION for a collection (persisted to config.toml).
- Does NOT apply it immediately to existing vectors (use `optimize`).

### optimize
Trigger background optimization (quantization) for a collection.
`vecdb optimize [COLLECTION]`
- Applies the configured quantization setting to the collection in Qdrant.
- Useful after `config set-quantization` or bulk ingestion.

### history
Time Travel / History Operations.
`vecdb history [COMMAND] [OPTIONS]`

**Commands:**
- `ingest [PATH]`: Ingest a specific version of a repository (requires `--git-ref`).

**Options:**
- `--git-ref [REF]`: Git commit, tag, or branch to ingest.
- `--collection [NAME]`: Target collection.

**Agent Usage:**
`vecdb history ingest . --git-ref v1.0.0 --collection legacy_v1`

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
