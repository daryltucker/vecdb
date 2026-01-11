# MCP Interface Definition

> **Status**: Implemented (v0.0.9) - Ready for Release
> **Type**: Technical Specification

---

## ⚠️ Redirect Notice
For a high-level guide on using the MCP server with Claude Desktop, see **[docs/MCP_SERVER.md](../MCP_SERVER.md)**.

---

## Overview
The `vecdb-server` implements the Model Context Protocol (MCP) via JSON-RPC 2.0 over stdio/stdout.

## Tools (JSON-RPC)
| Tool | Description |
|------|-------------|
| `search_vectors` | Semantic search with `profile` and `collection` overrides. |
| `list_collections` | Discovery tool for available indices and their compatibility. |
| `ingest_path` | Ingest local files (Requires `--allow-local-fs`). |
| `ingest_historic_version` | Git-based "Time Travel" ingestion. |
| `code_query` | Structural analysis using `vecq` AST filters. |
| `delete_collection` | Protected deletion tool with safety confirmation. |
| `embed` | Raw text-to-vector conversion. |

## Resources (URIs)
| URI | Description |
|-----|-------------|
| `vecdb://registry` | Combined JSON summary of server state. |
| `vecdb://manual` | Real-time Agent Interface Specification (Markdown). |
| `vecdb://collections/{name}` | Detailed metadata for a specific collection. |

---
*"One server, many profiles. One agent, total knowledge."*
