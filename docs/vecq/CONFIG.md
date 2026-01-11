# Configuration

`vecq` is primarily a stateless CLI tool, meaning it does not rely on a central `config.toml` file for its runtime behavior (unlike `vecdb`).

However, it supports **User-Defined Functions** which are loaded from a specific configuration directory.

## Configuration Directory
`vecq` looks for configuration in:
*   **Linux/macOS**: `~/.config/vecq/`
*   **Windows**: `%APPDATA%\vecq\`

## User Functions
You can define custom `jq` functions (macros) that are automatically loaded and available to every `vecq` query.

1.  Create the functions directory:
    ```bash
    mkdir -p ~/.config/vecq/functions
    ```

2.  Add a `.jq` file (e.g., `common.jq`):
    ```jq
    # ~/.config/vecq/functions/common.jq
    def public_fns: .functions[] | select(.visibility == "pub");
    ```

3.  Use it in your queries:
    ```bash
    vecq src/lib.rs 'public_fns | .name'
    ```

This allows you to build a personal library of shortcuts for complex queries.

See [Functions](FUNCTIONS.md) for a detailed walkthrough of creating complex macros.
