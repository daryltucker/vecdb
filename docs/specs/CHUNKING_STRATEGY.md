# Specification: Advanced Chunking Strategy

> **Status**: Draft — **partially implemented.** See the reality notes below;
> two clauses describe behaviour that does not exist yet.
> **Parent**: [INGESTION_DESIGN.md](INGESTION_DESIGN.md)
> **Source**: [AdvancedCodeChunkingforRAG.md](../inquiries/responses/AdvancedCodeChunkingforRAG.md)

## 1. Philosophy: Syntax-First Architecture

Code is not clear text; it is a serialized graph of logical dependencies. `vecdb-core` treats code as "Structure First, content second."

> **Reality, 2026-08-24: which strategy actually runs.**
>
> `ParserFactory::get_parser` returns a parser for every `is_supported()`
> FileType, and parser output is used verbatim, so `process_content` — the only
> place `target_chunk_size` is read — runs only when a parser *fails*. Independently,
> `Factory::get` forces `FixedWidthChunker` for `ParsingCapability::Simple` (which is
> what `Text` maps to), and `FixedWidthChunker` reads only `max_chunk_bytes`.
>
> Net effect: chunks are AST elements, and `max_chunk_bytes` is the only size
> control that takes effect. Measured — identical chunk counts with
> `target_chunk_size` at 50 and at 5000 (200 chunks for a `.rs` file, **1 chunk** for a
> 63 KB `.txt`).
>
> AST chunking is deliberately file-scoped for now; project-level linking was not
> in scope for this stage. The gap between this section and that behaviour is
> recorded here rather than silently tolerated.

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
| `recursive` | `text-splitter` | Prose, `.md`, `.txt` | Standard overlap-based splitting. **Not currently reached** — see note. |
| `simple` | fixed-width | Files with no useful structure | Byte-bounded splitting. |
| `notebook` | `serde_json` | `.ipynb` | Cell-aware splitting (Code vs Markdown cells). |

> **`code_aware` was removed, 2026-08-25.** It named a chunker that could never
> run: `processor.rs` uses a parser's chunks whenever a parser exists for the
> file type, and `VecqParserFactory` claims every type except `Unknown`, so a
> chunker never sees a source file. The strategy could only have applied to
> files with no recognised type — where AST splitting is meaningless.
>
> The pipeline below is real. It is simply not a strategy you select: it is what
> the **parser path** does, automatically, for every language vecq supports.
> A config still setting `strategy = "code_aware"` is refused at load.

## 3. The AST Pipeline (automatic; not selectable)

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
    *   If `node.len() > max_chunk_bytes`:
        *   Attempt **Logical Split**: Break by child blocks (if/for/while).
        *   Fallback: **Dumb Split** (Char-based with overlaps) if logic structure is too dense.

> **Reality, 2026-08-24.** Only the dumb split exists. An oversized element goes
> straight to `FixedWidthChunker`, which cuts at `max_chunk_bytes` **bytes** on the
> nearest preceding newline. The logical-split step was never written. What the
> split does now report is honest — parts carry `split_part` and
> `original_chunk_id` and real line bounds — and `on_oversize = "skip"` will
> refuse the insert instead if you prefer the corpus to stay as precise as the
> source.

## 4. Metadata Schema

Beyond standard file metadata, the AST path injects semantic fields.
Names below are as actually written to Qdrant, read back from a live
collection on 2026-08-25 — an earlier revision of this table named three
fields that have never existed (`scope`, `node_type`, `symbols_defined`):

| Field | Description | Example |
|-------|-------------|---------|
| `crumbtrail` | Fully qualified path | `services::db`, `MyClass` |
| `element_type` | vecq element kind | `function`, `class`, `variable` |
| `name` | Element's own name | `alpha`, `compute_checksum` |
| `language` | Source language | `python`, `rust` |
| `docstring` / `intent` | Docstring, where the language has one | `"Checks the residency policy."` |
| `line_start` / `line_end` | Bounds within the file | `0`, `2` |

> `docstring` is payload only — it is not part of the embedded text, so it is
> filterable but not matchable.

## 6. Hybrid Architecture: Small vs. Large

We utilize a bifurcated pipeline based on file size to balance AST precision with system stability.

### Phase 1: Small Files (< 50MB)
*   **Engine**: `vecq` (node-based). `CodeChunker` was the indentation-based
    alternative here; it was unreachable and has been deleted.
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
