# vecdb CLI Reference

The `vecdb` command-line tool is the primary interface for humans and scripts to interact with the project. It handles ingestion, searching, and collection management.

## Global Options

| Option | Description |
| :--- | :--- |
| `--profile <NAME>` | Specify the configuration profile to use (overrides `VECDB_PROFILE`). |
| `-h, --help` | Show help information. |
| `-V, --version` | Show version information. |

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
    *   `-c, --collection <NAME>`: Target collection name (defaults to profile's default).
    *   `-m, --metadata <KEY=VALUE>`: Custom metadata for stdin ingestion (accumulates).
    *   `--respect-gitignore`: Skips files ignored by `.gitignore` or `.vectorignore`.
    *   `--chunk-size <INT>`: Override profile's target chunk size.
    *   `-o, --overlap <INT>`: Override profile's chunk overlap.
    *   `--extensions <EXT>`: Whitelist file extensions (e.g. `rs,md`).
    *   `--excludes <GLOB>`: Exclude patterns (e.g. `*.tmp`, `target/`).

### `search <QUERY>`
Perform semantic search against the index.
*   **Arguments**: `<QUERY>` (semantic natural language query).
*   **Options**:
    *   `-c, --collection <NAME>`: Collection to search in.
    *   `--json`: Output results as a pure JSON array for piping.
    *   `--smart`: Enable smart routing (multi-hop reasoning and facet detection).

### `list`
List available collections and their statistics.
*   **Options**:
    *   `--json`: Output the collection list as JSON.

### `status`
Show system health, connectivity, and detailed collection stats.
*   **Aesthetics**: Uses rich terminal formatting by default.
*   **Options**:
    *   `--json`: Output full system status and collection details as JSON.

### `delete <COLLECTION>`
Safely delete a collection.
*   **Security**: Requires a randomized confirmation token to prevent accidents.

### `history ingest`
Ingest a specific version of a repository (Time Travel).
*   **Options**:
    *   `-r, --git-ref <REF>`: The SHA, branch name, or tag to ingest.
    *   `-c, --collection <NAME>`: Target collection.

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
