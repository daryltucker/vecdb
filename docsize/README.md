# docsize v0.0.9

`docsize` is a contextualized LLM wrapper designed to work seamlessly with `vecdb` and `vecq`. It gathers directory structure and semantic search results to provide a high-fidelity prompt to local LLMs (primarily Ollama).

>  This tool serves a practical, useful purpose, while being an example of how to use `vecdb` and `vecq` libraries in your own project.

## External Dependencies

### Ollama (Docker)
`docsize` needs an Ollama instance to work. Check `examples/docker-compose-qdrant` for an easy example to spin up your own Ollama.

If you don't have one running, you can start one via Docker:

```bash
docker run -d \
  --gpus=all \
  -v ollama_ollama:/root/.ollama \
  -p 11434:11434 \
  --name ollama \
  ollama/ollama
```
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
- **� Debug Mode**: Inspect the exact prompt and raw server responses with `--debug`.
- **�🚀 CUDA-Boosted**: Integrates with `vecdb` binaries for rapid local embeddings.
- **🌲 Precise Context**: Uses optimized `tree` filtering to generate a clean, path-based directory overview.
- **💾 Session Management**: Maintains conversation history in `~/.config/docsize/convo.json`.

## Installation

### Method 1: Full Suite (Recommended)
From the root of the `vecdb-mcp` repository:
```bash
./install.sh
```
This installs `docsize`, `vecdb` (CUDA enabled), `vecdb-server` (CUDA enabled), and `vecq`.

### Method 2: Standalone
```bash
cd docsize
./install.sh
```

### Method 3: Cargo
```bash
cargo install --git https://github.com/daryltucker/vecdb-mcp docsize
```

## Usage

```bash
docsize [QUERY] [OPTIONS]
```

### Options
- `-d, --dir <DIR>`: Target directory for context gathering (default: `.`)
- `-m, --model <MODEL>`: Specify the LLM model to use
- `-n, --no-context`: Omit providing directory and semantic context
- `-a, --append`: Append to the current conversation session
- `--smart`: Enable Smart Routing (detect filters from query)
- `--debug`: Show the final prompt sent to the LLM and raw server logs
- `man --agent`: Show agent-optimized documentation

## Configuration

Settings are stored in `~/.config/docsize/config.toml` (XDG Compliant).

### Custom Ollama URL
If using a custom endpoint or proxy (e.g., edge proxy):
```toml
ollama_url = "https://ollama-003.edge.nugit.net"
```

### Prompt Template
The prompt template is stored in `~/.config/docsize/prompt.md`. You can customize it to change how the AI responds.
Supported placeholders:
- `{{ %DOCSIZE_TREE% }}`: Directory path list
- `{{ %DOCSIZE_VECDB_EMBEDDING_RESPONSE% }}`: Semantic search blocks (Context)
- `{{ %QUERY% }}`: User query
