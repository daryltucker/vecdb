# vecdb(1) - The Vector Database Project CLI

## SYNOPSIS
    vecdb <COMMAND> [OPTIONS]

## DESCRIPTION
    **vecdb** is the primary interface for the Vector Database Project.
    It provides tools for ingesting documents, querying the semantic index,
    and managing the underlying Qdrant storage.

## COMMANDS

### Essential
    * **ingest** [PATH] [--respect-gitignore] [--chunk-size N] [--overlap M]
                 [--extensions e,xt] [--excludes glob*] [-m key=value]
        Recursively ingest documents from a path.
        Use `--respect-gitignore` to obey .gitignore files.
        Use `--chunk-size N` to override default token/char chunk size.
        Use `--overlap M` to override default chunk overlap.
        Use `--extensions` to whitelist extensions (e.g. `rs,md`).
        Use `--excludes` to blacklist paths (e.g. `*.tmp`).
        Use `-m key=value` to attach global metadata to all ingested files.

        Example: `vecdb ingest ./docs --extensions md --excludes private/`

    * **search** [QUERY] [-c COLLECTION]
        Search for relevant documents.
        Example: `vecdb search "vector embeddings"`

    * **delete** [COLLECTION] [--all] [--yes]
        Safely delete a collection. 
        Interactive: Requires a randomized security token.
        Non-interactive: Use `--yes` to force deletion (NOT RECOMMENDED).

    * **list**
        List all available collections and their statistics.

    * **status** [--json]
        Show system health, configuration, and collection stats.
        Use `--json` for machine-readable status.


### Advanced
    * **man** [--agent]
        Display this manual.
        Use `--agent` to see the Agent Context Specification.

## PHILOSOPHY
    "Context is an Ocean. We provide the Sextant."

    This tool is designed to be pipe-friendly and composable.
    Output is minimal by default; use `-v` for logs.

## CONFIGURATION
    Config file: `~/.config/vecdb/config.toml`

    Three layers: a profile names an embedder, an embedder names a backend.

    **[backend.<name>]** — where a model runs:
    - `kind = "fastembed"`: Built-in ONNX. No external services needed.
    - `kind = "ollama"`: Ollama API. Requires Ollama running + model pulled.

    **[embedder.<name>]** — which model, and how it is tuned (`model`,
    `num_ctx`, `batch_inputs`/`batch_rows`). One backend can serve several.

    **[profiles.<name>]** — which embedder, and which Qdrant to write to.

    Run `vecdb config show` to see the effective values and where each came from.

## EXAMPLES
    Ingest current directory:
    $ vecdb ingest .

    Search for a concept:
    $ vecdb search "vector embeddings" | jq
