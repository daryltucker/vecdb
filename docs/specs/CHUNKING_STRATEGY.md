# Specification: Advanced Chunking Strategy

> **Status**: Draft
> **Parent**: [INGESTION_DESIGN.md](INGESTION_DESIGN.md)
> **Source**: [AdvancedCodeChunkingforRAG.md](../inquiries/responses/AdvancedCodeChunkingforRAG.md)

## 1. Philosophy: Syntax-First Architecture

Code is not clear text; it is a serialized graph of logical dependencies. `vecdb-core` treats code as "Structure First, content second."

## 2. Modular Strategy Interface

We define a selectable strategy pattern for chunking, configurable per-file (via `.config/vecdb/config.toml`).

```rust
trait ChunkingStrategy {
    fn chunk(&self, content: &str, params: ChunkParams) -> Result<Vec<Chunk>>;
    fn name(&self) -> &str;
}
```

### Supported Strategies

| Strategy | Engine | Best For | Description |
|----------|--------|----------|-------------|
| `recursive` | `text-splitter` | Prose, `.md`, `.txt` | Standard overlap-based splitting. |
| `code_aware`| `tree-sitter` | Source Code | AST-based traversal identifying "Atomic Nodes". |
| `notebook` | `serde_json` | `.ipynb` | Cell-aware splitting (Code vs Markdown cells). |

## 3. The `code_aware` Pipeline

1.  **Parse**: Generate AST using `tree-sitter` for the target language.
2.  **Traverse (Scope Visitor)**:
    *   Maintain a `ScopeStack` (e.g., `[Module, Class, Function]`).
    *   On entry: Push node name.
    *   On exit: Pop node name.
3.  **Identify Atomic Units**:
    *   Extract full `function_definition` or `class_definition` nodes.
    *   **Context Injection**: Prepend the current `ScopeStack` to the chunk text.
        *   Format: `// Context: {Module} > {Class} > {Function}\n{Content}`
4.  **Handle Oversized Nodes**:
    *   If `node.len() > max_chunk_size`:
        *   Attempt **Logical Split**: Break by child blocks (if/for/while).
        *   Fallback: **Dumb Split** (Char-based with overlaps) if logic structure is too dense.

## 4. Metadata Schema

Beyond standard file metadata, `code_aware` injects semantic fields:

| Field | Description | Example |
|-------|-------------|---------|
| `scope` | Fully qualified path | `my_module.MyClass.method_name` |
| `node_type` | AST Node Type | `function_definition`, `impl_item` |
| `imports` | List of imported modules | `["serde", "tokio"]` |
| `symbols_defined` | Symbols created | `["MyClass", "helper_fn"]` |

## 6. Hybrid Architecture: Small vs. Large

We utilize a bifurcated pipeline based on file size to balance AST precision with system stability.

### Phase 1: Small Files (< 50MB)
*   **Engine**: `vecq` (Node-based) or `CodeChunker` (Indentation-based).
*   **Redundancy Filtering**: Skips structural "container" nodes (e.g., a class body) if its children already cover >90% of the text. This prevents "Double Counting" where the same code exists as a Class chunk and several Method chunks.
    *   *Exception*: Nodes with docstrings or critical types (Functions, Classes) are always preserved to maintain semantic anchoring.
*   **Stable IDs**: Uses Uuid v5 derived from `doc_id::crumbtrail::content_hash`. This ensures that renaming a file or moving code within a file (stable trail) maintains the same vector ID if the content is identical.

### Phase 2: Large Files (> 50MB) - The "Two-Pass" Strategy
To prevent OOM when loading multi-gigabyte files, we use a segmentation approach:
1.  **Pass 1 (Segmentation)**: Files are sliced into 5MB segments with a 500KB overlap.
2.  **Pass 2 (Extraction)**: Each segment is independently parsed for chunks.
3.  **Assembly (Stitching)**: Chunks are deduplicated by content hash and re-assembled using `stitch_text` to bridge semantic gaps at segment boundaries.

## 7. The `crumbtrail` Pattern
Every code chunk includes a `crumbtrail` metadata field (e.g., `PaymentProcessor::init_vault::authorize`). This provides:
1.  **Semantic Context**: Embedders can use the trail to understand the "where" of the code.
2.  **Stable Identity**: Resilient to line-number shifts.
