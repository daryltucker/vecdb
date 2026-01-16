# AGENT INTERFACE SPECIFICATION: vecdb
Version: 0.1.0

## PURPOSE
`vecdb-mcp` provides high-performance vector search and structural discovery for large-scale source code repositories.

## CORE WORKFLOW
1.  **Initialize**: `vecdb init` to setup local config.
2.  **Ingest**: `vecdb ingest <path>` to vectorize the codebase.
    - Use `mcp_vecdb_ingest_path` for local files.
    - Use `mcp_vecdb_ingest_history` for "Time Travel" queries.
3.  **Search**: `mcp_vecdb_search_vectors` to find semantic targets.
4.  **Query**: `mcp_vecdb_code_query` to extract structural details from targets.

## TOOLS & COMMANDS

### 1. Vector Search (`search_vectors`)
- **Semantic Mapping**: Finds code concepts (e.g., "auth implementation") rather than exact strings.
- **Workflow**: Always search before reading large files.
- **Tips**:
    - Use `smart=true` for automatic filtering by language or version.
    - Use specific queries like "how is the login hash calculated" rather than just "login".

### 2. Job Status (`status`)
- **Action**: Check status of ongoing background jobs (ingestion, optimization).
- **Usage**: `vecdb status`
- **Agent Tip**: If a tool call returns "Successfully started ingest", use `vecdb status` to monitor progress.

### 3. Time Travel (`ingest_history`)
- **Usage**: Ingest a specific Git SHA or Tag to compare historical state.
- **Example**: `ingest_history(repo_path='.', git_ref='v1.0.0', collection='v1-docs')`

## THE AUDITOR'S EYE: AGENT PROTOCOLS
1.  **Semantic-First**: Never `grep` if you can `search_vectors`. Never `cat` if you can `code_query`.
2.  **Absolute Paths**: Always use absolute paths in tool calls to prevent ambiguity.
3.  **Discovery Protocol**: If you encounter an unknown schema, probe it with `vecq elements <type>`.
4.  **Trust but Verify**: Use `vecdb status` to ensure your indexing is complete before searching.

## THROUGH THE EYES OF AN AGENT: A WALKTHROUGH

### Phase 1: Structural Reconnaissance
When given a task on a file you've never seen, don't read the whole file. Instead, get an "X-Ray" view.

**Tool Call**:
```json
{
  "name": "mcp_vecdb_code_query",
  "arguments": {
    "path": "/absolute/path/to/parsers/rust.rs",
    "query": ".functions[] | select(.name==\"parse\") | {name, crumbtrail, range: [.line_start, .line_end]}"
  }
}
```

### Phase 2: The "Surgical" Edit
Now that you found the target (`parse` at lines 337-356) and understood its architectural context, you can make a high-confidence edit.

---
*Created by: Sextant*
*Verified: 2026-01-15*
