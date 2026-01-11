# docsize v0.0.9

`docsize` is a contextualized LLM wrapper designed to work seamlessly with `vecdb` and `vecq`. It gathers directory structure and semantic search results to provide a high-fidelity prompt to local LLMs (primarily Ollama).

## Architecture & Data Flow

`docsize` implements a local RAG (Retrieval-Augmented Generation) pipeline:

1.  **Query**: You ask a question (e.g., `docsize "How do I install?"`).
2.  **Context Gathering**:
    *   **Filesystem**: Generates a clean directory tree (ignoring `target`, `node_modules`, etc.).
    *   **Vector Database**: Sends **only the query** to `vecdb-server` to find relevant code and docs.
3.  **Prompt Assembly**: Combines the Tree, Vector Search Results, and your Query into a `prompt.md` template.
4.  **Inference**: Streams the **full context-rich prompt** to the Main LLM (Ollama) for a grounded response.

## Features

- **⚡ Real-Time Streaming**: Responses are streamed token-by-token for immediate feedback.
- **🛡️ Intelligent Model Selection**: Interactive arrow-key selection of Ollama models (`/api/tags`).
- **🧠 Smart Routing**: Automatically routes queries to the best collection/filters via `vecdb`'s smart router.
- **🔎 Debug Mode**: Inspect the exact prompt and raw server responses with `--debug`.
- **🚀 CUDA-Boosted**: Integrates with `vecdb` binaries for rapid local embeddings.
- **🌲 Precise Context**: Uses optimized `tree` filtering to generate a clean, path-based directory overview.
- **💾 Session Management**: Maintains conversation history in `~/.config/docsize/convo.json`.

## Usage for Agents

Agents can use `docsize` to get a structured view of a project or to interact with local LLMs with full repo context.

```bash
docsize [QUERY] [OPTIONS]
```

### Options
- `-d, --dir <DIR>`: Target directory for context [default: `.`]
- `-m, --model <MODEL>`: Specify the LLM model to use
- `-n, --no-context`: Omit providing directory and semantic context
- `-a, --append`: Append to the current conversation session
- `--debug`: Show final prompt and raw server logs
- `man --agent`: (You are here) Show agent-optimized documentation

## Configuration

Settings are stored in `~/.config/docsize/config.toml` (XDG Compliant).

### Custom Ollama URL
```toml
ollama_url = "https://ollama-003.edge.nugit.net"
```

### Prompt Template
The prompt template is stored in `~/.config/docsize/prompt.md` and supports placeholders:
- `{{ %DOCSIZE_TREE% }}`: Directory path list
- `{{ %DOCSIZE_VECDB_EMBEDDING_RESPONSE% }}`: Semantic search blocks
- `{{ %QUERY% }}`: User query

> [!NOTE]
> DOCSIZE IS NOT A LIBRARY. IT IS ITS OWN, INDEPENDENT PROJECT.