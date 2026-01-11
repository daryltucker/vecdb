<!--
  PURPOSE:
    Define our OpenAPI 3.1 compatibility policy and requirements for the vecdb-mcp API.
    
  REQUIREMENTS:
    - MCP Discovery compatibility
    - Self-documenting API for agents and humans
    - JSON Schema alignment (Draft 2020-12)

  SELF-HEALING:
    - Update when MCP spec evolves
    - Last Verified: 2025-12-31
-->

# OpenAPI Policy & Requirements

> **Baseline**: OpenAPI 3.1 (JSON Schema Draft 2020-12)  
> **Purpose**: Ensure vecdb-mcp API is discoverable, self-documenting, and tool-compatible.

---

## Core Requirements

### 1. Full JSON Schema Compliance

All tool `inputSchema` definitions MUST be valid JSON Schema Draft 2020-12.

**Required Keywords**:
- `type`: Use array syntax for nullable (e.g., `["string", "null"]`)
- `description`: Every property must be documented
- `required`: Explicitly list required fields
- `examples`: Prefer over deprecated `example`

**Example**:
```json
{
  "inputSchema": {
    "type": "object",
    "properties": {
      "query": {
        "type": "string",
        "description": "The semantic search query"
      },
      "limit": {
        "type": "integer",
        "default": 10,
        "description": "Max results to return"
      }
    },
    "required": ["query"]
  }
}
```

### 2. Discovery via `tools/list`

Every exposed tool MUST include:
- `name`: Unique identifier (snake_case)
- `description`: Human-readable purpose
- `inputSchema`: Complete JSON Schema

**Our Tools**:
| Tool | Description |
|------|-------------|
| `search_vectors` | Semantic search against the index |
| `ingest_file` | Index a file or directory |
| `list_collections` | List available collections |

### 3. Server Capabilities

The `initialize` response MUST include:
```json
{
  "capabilities": {
    "tools": {}
  },
  "serverInfo": {
    "name": "vecdb-mcp",
    "version": "0.1.0"
  }
}
```

**Extended Discovery** (Future):
```json
{
  "capabilities": {
    "tools": {},
    "embeddings": ["minilm", "code-bert"],
    "rerankers": ["bge-reranker-base"]
  }
}
```

---

## Best Practices (from OpenAPI 3.1)

### Modularity
- Use `$ref` for reusable schemas
- Define common types in `components/schemas`

### Nullable Fields
```json
// Correct (OpenAPI 3.1)
"type": ["string", "null"]

// Deprecated (OpenAPI 3.0)
"nullable": true
```

### Examples
```json
"examples": {
  "basic": { "value": { "query": "hello world" } }
}
```

---

## Compatibility Matrix

| Feature | vecdb-mcp | OpenAPI 3.1 | MCP Spec |
|---------|-----------|-------------|----------|
| JSON Schema | ✅ Draft 2020-12 | ✅ | ✅ |
| `tools/list` | ✅ | N/A | ✅ |
| `inputSchema` | ✅ | ✅ | ✅ |
| WebSocket | ❌ (Stdio) | ✅ | ✅ |

---

## References

- [OpenAPI 3.1 Spec](https://spec.openapis.org/oas/v3.1.0)
- [JSON Schema 2020-12](https://json-schema.org/draft/2020-12/json-schema-core)
- [MCP Protocol](https://modelcontextprotocol.io/)

---

*Last Updated: 2025-12-31*
