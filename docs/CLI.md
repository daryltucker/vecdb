# vecdb CLI Reference

The `vecdb` command-line tool is the primary interface for humans and scripts to interact with the project. It handles ingestion, searching, and collection management.

## Global Options


| Option | Description |
| :--- | :--- |
| `--profile <NAME>` | Specify the configuration profile to use (overrides `VECDB_PROFILE`). |
| `-j, --json` | **Force** JSON output (bypasses smart detection). |
| `-m, --markdown` | **Force** Human-Readable output (bypasses smart detection). |
| `-h, --help` | Show help information. |
| `-V, --version` | Show version information. |

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
    *   `--respect-gitignore`: Skips files ignored by `.gitignore`.
    *   `--chunk-size <INT>`: Target chunk size (tokens for text, chars for default).
    *   `-o, --overlap <INT>`: Chunk overlap.
    *   `--extensions <EXT>`: Whitelist file extensions (e.g. `rs,md`).
    *   `--excludes <GLOB>`: Exclude patterns (e.g. `*.tmp`, `target/`).
    *   `--dry-run`: Dry run: List files without processing.
    *   `-P, --concurrency <INT>`: Max concurrent file processing tasks.
    *   `-G, --gpu-concurrency <INT>`: Max concurrent GPU embedding tasks.

### `search <QUERY>`
Perform semantic search against the index.
*   **Arguments**: `<QUERY>` (semantic natural language query).
*   **Options**:
    *   `-c, --collection <NAME>`: Collection to search in.
    *   `--profile <NAME>`: Profile to use.
    *   `--smart`: Enable smart routing (multi-hop reasoning and facet detection).

### `list`
List available collections and their statistics.

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

### `history ingest [REPO_PATH]`
Ingest a specific version of a repository (Time Travel).
*   **Options**:
    *   `-r, --git-ref <REF>`: The SHA, branch name, or tag to ingest.
    *   `-c, --collection <NAME>`: Target collection.

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
