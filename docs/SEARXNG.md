# SearXNG & Web Memory Integration

> **Status**: Conceptual / In-Memory Support Implemented
> **Scope**: Defines how `vecdb-mcp` handles ephemeral data from the web.

---

## 1. The Context

Agents often perform web searches (via SearXNG) and need to "remember" the results for analysis or future recall. Unlike local files, these documents do not persist on disk in the project workspace.

---

## 2. Ingestion Path: `ingest_memory`

`vecdb-core` exposes `ingest_memory`:
```rust
pub async fn ingest_memory(
    backend: &Arc<dyn Backend>,
    embedder: &Arc<dyn Embedder>,
    content: &str,
    metadata: HashMap<String, Value>,
    collection: &str,
) -> Result<()>
```

### Usage Pattern
1.  **Agent Search**: Agent queries SearXNG -> gets search snippets/pages.
2.  **Memory Store**: Agent calls `vecdb:upsert_content` (MCP tool mapping to `ingest_memory`).
3.  **Metadata Tagging**:
    - `source_type`: "web"
    - `url`: `https://example.com/foo`
    - `query`: "original search query"
    - `timestamp`: Now

---

## 3. Storage Strategy

Web content should usually go into a distinct collection (e.g., `web_memory` or `scratchpad`) or be strictly tagged to prevent polluting the trusted Codebase knowledge graph.

**Profile Recommendation**:
```toml
[profile.web]
collection = "web_scrapes"
metadata = { trust_level = "low" }
```

---

## 4. Future Integration: Direct Piping

Future milestones may include a direct pipe:
`searxng-cli search "rust async traits" | vecdb ingest --stdin --profile web`

This allows "Slurping" live web context directly into the vector database for immediate RAG.
