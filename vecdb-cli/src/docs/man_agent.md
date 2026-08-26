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
- `--profile [NAME]`: Profile to use from config.toml — WHICH embedder and store.
- `--embedder [NAME]`: Override the profile's embedder — WHAT model and tuning.
- `--backend [NAME]`: Run that embedder elsewhere — WHERE only. Model, `num_ctx`
  and batch are unchanged, so vectors stay comparable. Refuses to cross a backend
  `kind` rather than silently discarding tuning that does not apply.
- `--chunk-size [INT]`: Max tokens per chunk (default: 1000).
- `-o, --overlap [INT]`: Chunk overlap (default: 0).
- `--respect-gitignore`: Skips files ignored by .gitignore.
- `--extensions [LIST]`: Whitelist e.g. "rs,md".
- `--excludes [LIST]`: Blacklist globs e.g. "*.tmp".
- `--metadata [K=V]`: Attach metadata (can be used multiple times).
- `--dry-run`: List files that would be ingested without processing.
- `-P, --concurrency [INT]`: Max concurrent file processing tasks.
- `-G, --gpu-concurrency [INT]`: Max concurrent GPU embedding tasks (batch size).
- `--allow-quantization-delta`: Permit writing into a collection whose model
  matches on architecture and parameter size but differs in quantization.
  Off by default — see EMBEDDING SPACES below.

**Agent Usage:**
`vecdb ingest ./src`
(Use default collection defined in profile, typically 'docs' or project specific)

### search
Semantic search against the vector store.
`vecdb search [QUERY] [OPTIONS]`

**Options:**
- `-c, --collection [NAME]`: Source collection. Optional.
- `--profile [NAME]`: Profile to use from config.toml — WHICH embedder and store.
- `--embedder [NAME]`: Override the profile's embedder — WHAT model and tuning.
- `--backend [NAME]`: Run that embedder elsewhere — WHERE only. Model, `num_ctx`
  and batch are unchanged, so vectors stay comparable. Refuses to cross a backend
  `kind` rather than silently discarding tuning that does not apply.
- `--json`: Output as JSON for parsing.
- `-n, --limit [INT]`: Max results (default: 10).
- `--min-score [FLOAT]`: Minimum similarity (0.0-1.0). Applied by the vector
  store BEFORE the limit is imposed, so a threshold never silently returns
  fewer results than exist above it.
- `--smart`: Enable `key:value` facet qualifiers in the query. Off by default.
- `--no-smart`: Explicitly disable them.

**Response shape (`--json`):**
```json
{
  "collection": "code",
  "query": "authentication implementation",
  "limit": 10,
  "min_score": null,
  "applied_filters": {},
  "results": [
    {"id":"...", "score":0.82, "content":"...", "document_id":"...",
     "metadata":{"path":"src/auth.rs","line_start":21,"line_end":88}}
  ]
}
```

**Reading the response:**
- `results.length == limit` means the list was CUT OFF. Re-run with a higher
  `--limit`; do not conclude the corpus is exhausted.
- `applied_filters` is non-empty only when `--smart` parsed a qualifier. If the
  results look narrower than expected, read this first.
- An empty `results` with a non-empty `applied_filters` or a `min_score` means
  the search was NARROWED, not that the collection is empty.

**Smart qualifiers (`--smart`):**
Scoping is written into the query, never inferred from it. A token of the form
`key:value` where `key` is a configured facet (`source_type`, `language`)
becomes a metadata filter and is removed from the text before embedding.
```
vecdb search "parse errors language:rust" --smart
```
Everything else — bare words, URLs, unconfigured keys — is left in the query
untouched. Naming a facet value that does not exist is an ERROR listing the
valid values, not an empty result set.

**Agent Usage:**
`vecdb search "authentication implementation" --json`

### list
List every collection on each configured backend, with its statistics and the
model that created it.

Collections vecdb did not create are LISTED, not hidden — a Qdrant instance is
shared infrastructure, and a name missing from `list` but rejecting an ingest is
worse than a labelled one. They show as `— not a vecdb collection` in the Model
column and `"is_vecdb": false` in `--json`.

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
When running as an MCP Server (`vecdb-server`), these tools are available via
JSON-RPC. This list is the whole set — nine tools, matching
`vecdb-server/src/rpc/dispatcher.rs`.

### search_vectors
Semantic search against a collection.
`search_vectors(query, collection=null, profile=null, smart=false, json=false, limit=10, min_score=null)`
- Returns an envelope: `{collection, query, limit, min_score, applied_filters, result_count, results[]}`.
- **`result_count == limit` means the list was truncated** — re-run with a higher `limit` before concluding the corpus is thin.
- `min_score` is applied by the vector store *before* the limit, so a threshold never silently returns fewer results than exist above it.
- `smart=true` enables `key:value` facet qualifiers in the query text (e.g. `"parse errors language:rust"`). Qualifiers are stripped before embedding and reported back in `applied_filters`. An unknown facet value is an error listing the valid ones, not an empty result set.

### ingest_path
Ingest local files or folders.
`ingest_path(path, collection=null, profile=null, concurrency=null, gpu_concurrency=null, ignore_vectorignore=false)`
- **Security**: requires `VECDB_ALLOW_LOCAL_FS="true"`.
- `.vectorignore` governs what is indexed. `.gitignore` is **not** consulted unless no `.vectorignore` exists anywhere, in which case it is used as a fallback and the output says so.

### ingest_history
Ingest a specific git commit or tag ("time travel").
`ingest_history(repo_path, git_ref, collection=null, profile=null)`
- Runs in an ephemeral sandbox. The CLI equivalent is `vecdb history ingest`.

### get_job_status
Report ingestion jobs.
`get_job_status(id=null)`
- With no `id`: `{local_jobs[], remote_tasks[], remote_tasks_error}`. With one: that job plus any matching remote task.
- `remote_tasks_error` is set when the backend cannot enumerate tasks at all — Qdrant exposes per-collection optimizer status, not a task list. An empty `remote_tasks` with an error set means "unknown", not "none running".

### list_collections
Every collection on every configured Qdrant endpoint, with metadata.
`list_collections()`
- Collections written by another tool are **listed and labelled, never hidden** — a name absent here but rejecting an ingest is worse than a labelled one.

### delete_collection
Delete a collection. Irreversible.
`delete_collection(collection, confirmation_code)`
- `confirmation_code` must be `"<collection>-DELETE"`. Resolves the collection's own backend, so it deletes from the instance the collection actually lives on.

### embed
Generate embeddings for raw text, without storing them.
`embed(texts)`

### code_query
Query source structure using vecq syntax.
`code_query(query, path, source="git"|"local", git_ref=null, repo_path=null)`
- **Security**: `source="git"` uses an ephemeral sandbox and is always allowed. `source="local"` requires `VECDB_ALLOW_LOCAL_FS="true"`.
- Powered by `vecq`; see `vecq man --agent` for query syntax.

### project_overview
Structural summary of a directory tree.
`project_overview(path, max_depth=null, ignore_patterns=[], respect_gitignore=null, ignore_vectorignore=null, skip_hidden=true)`

### Resources
`vecdb://collections/<name>` returns `{name, vector_count, vector_size, is_active, is_vecdb, is_compatible, model, reason}`.
- **`is_compatible` is the field to check before writing.** False means either the collection is not vecdb's (`is_vecdb: false`) or it was written by a model whose embedding space differs from the one this server resolves to; `reason` says which.

## CONFIGURATION
Loaded from `~/.config/vecdb/config.toml`. Three layers:

- `[backend.<name>]` — WHERE a model runs: `kind` (`ollama`/`fastembed`), `url`, credentials.
- `[embedder.<name>]` — WHAT model and HOW tuned: `backend`, `model`, `num_ctx`, `batch_inputs`/`batch_rows`, `dimension`.
- `[profiles.<name>]` — WHICH embedder, and WHICH Qdrant to write to.

One backend can serve several embedders. `vecdb config show -c <collection>`
prints every effective value and the layer it came from.

## EXAMPLES

1. **Ingest Project**:
   `vecdb ingest .`

2. **Search for Code**:
   `vecdb search "database connection" --json`

3. **Check Status**:
   `vecdb list`


## EMBEDDING SPACES

A collection is only searchable if every vector in it came from the same model.
Dimension alone does not establish that — 384 and 768 are the two most common
embedding dimensions in existence, and unrelated models collide there routinely.

Every collection vecdb creates carries a genesis point recording the model
`name`, `digest`, `architecture`, `family`, `parameter_size`,
`quantization_level` and `dimension`, plus a `vecdb:<version>` marker declaring
the collection as vecdb's.

**Ownership is checked before compatibility.** A collection without the marker
is not an "incompatible collection" — it is *not a vecdb collection*, and vecdb
will neither read nor write it. That is permanent, not a migration state: a
Qdrant instance is shared, and other tools keep their own collections on it.

**Compatibility, once ownership is settled:**

| tier | condition | read | write |
|---|---|---|---|
| identical | same model digest | yes | yes |
| compatible | same architecture + parameter_size + dimension, different quantization | yes, with a note | needs `--allow-quantization-delta` |
| incompatible | anything else, including insufficient recorded identity | no | no |

Writes are stricter than reads on purpose: **a bad write contaminates a
collection permanently and compounds with every later ingest, while a bad read
produces one mediocre ranking and evaporates.**

Tags are not identity. `qwen3-embedding:4b` and `qwen3-embedding:4b-q4_K_M` can
be the same weights while `4b-q8_0` is different weights under a nearly
identical name. The digest decides; the tag is only displayed.
