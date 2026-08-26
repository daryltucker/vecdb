# vecdb CLI Reference

The `vecdb` command-line tool is the primary interface for humans and scripts to interact with the project. It handles ingestion, searching, and collection management.

## Global Options


| Option | Description |
| :--- | :--- |
| `--profile <NAME>` | **WHICH** embedder and store — the configuration profile to use (overrides `VECDB_PROFILE`). |
| `--embedder <NAME>` | **WHAT** model, and how it is tuned. Overrides the profile's (and a collection's) `embedder`; the store is unchanged. |
| `--backend <NAME>` | **WHERE** the embedder runs. Same model, `num_ctx` and batch — only the host changes. Refuses to cross a backend `kind`. |
| `-j, --json` | **Force** JSON output (bypasses smart detection). |
| `-m, --markdown` | **Force** Human-Readable output (bypasses smart detection). |
| `-h, --help` | Show help information. |
| `-V, --version` | Show version information. |

One flag per configuration layer, so a run can change one without redefining it
in `config.toml`. The common use is two machines filling one collection at once,
so a single embed host does not become everyone's queue:

```bash
vecdb --profile code ingest -c code ./                    # embedder's own backend
vecdb --profile code --backend blade ingest -c code ./    # same model, other GPU
```

Both write comparable vectors because `--backend` changes only *where* the model
runs. A run whose model weights or dimension disagree with the collection is
refused, not silently accepted.

`vecdb list` ignores `--embedder` and `--backend`: it resolves every profile to
enumerate stores, so overriding an embedder across all of them has no meaning.

## Output Standardization (Smart Defaults)
**"Pipes want Data, Humans want Headers."**

`vecdb` and `vecq` automatically adapt their output based on the context:
1.  **interactive (TTY)**: Output is formatted for humans (Tables, Markdown, Colors).
2.  **Pipe / Redirection**: Output is raw JSON for machine consumption.

**Example**:
- `vecdb list` → Displays a pretty ASCII table.
- `vecdb list | cat` → Outputs a JSON array.

You can **force** a specific format using the global flags `-j` (JSON) or `-m` (Markdown/Text).

---

## Commands

### `init`
Initialize or show configuration status.
*   Shows current config file location.
*   Displays the default profile name.

### `ingest [PATH]`
Recursively ingest documents from a path into a collection.
*   **Arguments**: `[PATH]` (defaults to `.` for current directory). Use `-` for stdin.
*   **Options**:
    *   `-c, --collection <NAME>`: Target collection name (created if missing).
    *   `-m, --metadata <KEY=VALUE>`: Custom metadata (accumulates).
    *   `--respect-gitignore`: Skips files ignored by `.gitignore` (disabled by default).
    *   `--ignore-vectorignore`: Bypass `.vectorignore` rules (ingest everything).
    *   `--chunk-size <INT>`: Target chunk size (tokens for text, chars for default).
    *   `-o, --overlap <INT>`: Chunk overlap.
    *   `--extensions <EXT>`: Whitelist file extensions (e.g. `rs,md`).
    *   `--excludes <GLOB>`: Exclude patterns (e.g. `*.tmp`, `target/`).
    *   `--dry-run`: Dry run: List files without processing.
    *   `-P, --concurrency <INT>`: Max concurrent file processing tasks.
    *   `-G, --gpu-concurrency <INT>`: Max concurrent GPU embedding tasks.
    *   `--allow-quantization-delta`: Permit writing into a collection whose
        model matches on architecture and parameter size but differs in
        quantization. Off by default; see
        [EMBEDDING_MODELS.md](EMBEDDING_MODELS.md).

### `search <QUERY>`
Perform semantic search against the index.
*   **Arguments**: `<QUERY>` (semantic natural language query).
*   **Options**:
    *   `-c, --collection <NAME>`: Collection to search in.
    *   `--profile <NAME>`: Profile to use.
    *   `-n, --limit <INT>`: Max results (default: 10).
    *   `--min-score <FLOAT>`: Minimum similarity (0.0-1.0). Applied by the
        vector store before the limit is imposed, so a threshold never silently
        shortens a full page of results.
    *   `--smart`: Enable `key:value` facet qualifiers in the query
        (e.g. `"parse errors language:rust"`). Off by default.
        See [VECTOR_FACETS.md](VECTOR_FACETS.md).
    *   `--no-smart`: Explicitly disable qualifier parsing.

With `--json`, the response is an object — `{collection, query, limit,
min_score, applied_filters, results[]}` — not a bare array. `results.length ==
limit` means the list was truncated; re-run with a higher `--limit`.

### `list`
List every collection on each configured backend, with its statistics and the
model that created it.

Collections not created by vecdb are shown and labelled
(`— not a vecdb collection`, or `"is_vecdb": false` under `--json`) rather than
hidden. A Qdrant instance is shared infrastructure; a name that is absent from
`list` but rejects an ingest is worse than a labelled one.

### `status`
Show system health, connectivity, and detailed collection stats.

### `config <SUBCOMMAND>`
Manage configuration settings.
*   **Subcommands**:
    *   `set-quantization <COLLECTION> <TYPE>`: Set quantization config (scalar, binary, none).
    *   `get`: View current config values.

### `optimize <COLLECTION>`
Apply optimization (quantization) to a collection based on its config.
*   **Arguments**: `<COLLECTION>` name.

### `delete <COLLECTION>`
Safely delete a collection.
*   **Options**:
    *   `--yes`: Skip confirmation (Danger!).

    *   `-c, --collection <NAME>`: Target collection.



### `history ingest [REPO_PATH]`
Ingest a specific version of a repository (Time Travel).
*   **Options**:
    *   `-r, --git-ref <REF>`: The SHA, branch name, or tag to ingest.
    *   `-c, --collection <NAME>`: Target collection.

### `enableusages [PATHS]`
Enable usage/reference extraction mode for source files.
*   **Arguments**: `[PATHS]` (one or more files or directories to analyze).
*   **Options**:
    *   `-o, --output <FORMAT>`: Output format for each usage (json, yaml, table, ast; default: json).
    *   `-f, --filter <TYPE>`: Filter usages by type (all, calls, references, assignments, methods; default: all).
    *   `-F, --format <FORMAT>`: Output format for the analysis summary: json, yaml, table, or ast (default: json).

### `snapshot`
Manage collection snapshots (backups).
*   **Commands**:
    *   `create`: Create a new snapshot.
    *   `list`: List available snapshots.
    *   `download <NAME>`: Download a specific snapshot.
    *   `restore <PATH>`: Restore a snapshot file.
    *   `-C, --collection <NAME>`: Override the target collection.

### `completions <SHELL>`
Generate shell completion scripts (bash, zsh, fish, powershell, elvish).
*   **Usage**: `source <(vecdb completions bash)`

### `man`
Display the project manual.
*   **Arguments**: `[COMMAND]` (View manual for a specific command).
*   **Options**:
    *   `--agent`: Output raw, machine-readable specification for AI Agents.

---

## Integration Tips

### Piping from Stdin
`vecdb` is designed for Unix-style composition:
```bash
cat docs.txt | vecdb ingest - --collection temp_notes -m source=scratchpad
```

### JSON Processing
Use `--json` with `jq` for advanced filtering:
```bash
vecdb search "auth policy" --json | jq '.[].content'
```
