# Thunar Integration Guide

Integrate `vecdb` directly into your file manager (Thunar) to ingest files and folders with a Right-Click.

## Prerequisites

*   **Linux** (XFCE / Thunar)
*   **xfce4-terminal** (or similar, script can be adapted)
*   `vecdb` installed and in your PATH

## Setup

1.  **Install the Script**
    Ensure `vecdb-thunar-ingest.sh` is executable and in a known location (e.g., `~/bin/` or inside the repo).

    ```bash
    chmod +x /path/to/vecdb-mcp/scripts/vecdb-thunar-ingest.sh
    ```

2.  **Configure Thunar**
    *   Open **Thunar**.
    *   Go to **Edit** -> **Configure Custom Actions...**
    *   Click the **+** (Add) button.

3.  **Create Action**
    *   **Name**: `Ingest to VecDB`
    *   **Description**: `Add selected files/folders to vector database`
    *   **Command**:
        ```bash
        xfce4-terminal --hold --title="VecDB Ingest" -x /path/to/vecdb-mcp/scripts/vecdb-thunar-ingest.sh %F
        ```
        *(Replace `/path/to/...` with your actual path)*
    *   **Icon**: Select a cool icon (optional).

4.  **Appearance Conditions**
    *   Click the **Appearance Conditions** tab.
    *   **File Pattern**: `*`
    *   Check **Directories**, **Text Files**, **Other Files**.
    *   (You can exclude things like Images if you don't ingest them).

## Usage

1.  Right-click any folder or file(s).
2.  Select **Ingest to VecDB**.
3.  A terminal window will pop up.
4.  Enter the collection name (or press Enter for default).
5.  Watch the ingestion process!

## Troubleshooting

*   **Terminal closes immediately?**
    Make sure `--hold` is supported by your terminal, or the script has `read -p "Press enter"` at the end (our script does!).
*   **Command not found?**
    Use absolute paths in the Thunar command.

## FAQ

**Q: Does this work for directories?**
A: **Yes.** `vecdb ingest` is recursive. If you select a folder, all its contents are ingested.

**Q: Why use a Terminal window?**
A: Two reasons:
1.  **Input**: We need to ask for the *Collection Name*.
2.  **Visibility**: You can see progress and errors (which is helpful if ingestion hangs).

**Q: Can I just have a localized notification?**
A: Yes, but you'd need to hardcode the collection name. You can modify the script to use `notify-send` instead of `echo` and remove `read -p`.
