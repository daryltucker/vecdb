# Specification: Advanced Chunking Strategy

> **Status**: Draft — **partially implemented.** See the reality notes below;
> two clauses describe behaviour that does not exist yet.
> **Parent**: [INGESTION_DESIGN.md](INGESTION_DESIGN.md)

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
>
> **Since 2026-10-27,** a line with no newline in the window is cut after its last
> whitespace rather than mid-word, and a chunk that opens with a label (see
> `context_label` below) has that label repeated at the top of **every** part, so
> part 3 of a long section still says which section it is.

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
| `context_label` | The label the chunk's text opens with — Markdown heading path, or a JSON fragment's key path and identifying fields | `# Guide\n\n## Install` |

> `docstring` is payload only — it is not part of the embedded text, so it is
> filterable but not matchable. `context_label` is the opposite: it IS in the
> embedded text, and the payload copy lets a reader separate label from source.
>
> These names, and the file-level ones (`path`, `source`, `full_path`,
> `language`, …), are reserved: `ingest` refuses `-m`, `.vecdbrc` route
> `metadata` or path-rule captures that reuse one, because the built-in value
> would silently replace yours. The list is `RESERVED_PAYLOAD_KEYS` in
> `vecdb-core/src/ingestion/options.rs`.

## 6. Hybrid Architecture: Small vs. Large

We utilize a bifurcated pipeline based on file size to balance AST precision with system stability.

### Phase 1: Small Files (< 50MB)
*   **Engine**: `vecq` (node-based). `CodeChunker` was the indentation-based
    alternative here; it was unreachable and has been deleted.
*   **Redundancy Filtering**: Skips structural "container" nodes (e.g., a class body) if its children already cover >90% of the text. This prevents "Double Counting" where the same code exists as a Class chunk and several Method chunks.
    *   *Exception*: Nodes with docstrings or critical types (Functions, Classes) are always preserved to maintain semantic anchoring.
    *   *Since 2026-10-27:* a container whose members are emitted contributes only its **frame** — its own text with the members cut out (`impl Foo {`, a class header, members that are not separate elements). No member is embedded twice, and the container's own words are not lost. The ratio above now applies only when `skip_redundant_parents` is off.
*   **Completeness** *(since 2026-10-27)*: for every tree-sitter language (Rust, C, C++, CUDA, Go, Bash, JavaScript) every node of the parse tree lands in an element — nothing the grammar parsed is dropped. Each language's table classifies every node kind its grammar can produce; a grammar upgrade that adds a kind fails the test suite until it is classified. CUDA is parsed with its own grammar (`tree-sitter-cuda`), so `__qualifiers__` and `<<<…>>>` launches are understood.
*   **Units are never merged** *(since 2026-10-27)*: a Markdown section or a named code unit (function, method, class, struct, interface, trait) is always its own chunk, whatever the file's size — size caps a unit, it never joins two. A file below the pack target is one chunk only when it has at most one unit (flat prose, a small JSON document, a file with one function). A 2-function file is two chunks, each named for its own function.
*   **Comments travel with what they describe** *(since 2026-10-27)*: a comment on its own line belongs to the element after it; a comment after code on the same line belongs to that code. A function's doc comment is embedded in the function's chunk, not at the end of the previous one.
*   **Lossless parse** *(since 2026-10-27)*: for the tree-sitter languages and Markdown, the elements vecq produces, with the whitespace and comments each one owns, reproduce the file byte for byte. Positions live behind the AST (not in `attributes`), so query output is unchanged.
    *   *Source code only.* **JSON** is never destructured by this ratio — a JSON object's keys are not child nodes, so dropping the object would drop every key. A JSON object or array that fits `max_chunk_bytes` is one chunk, whole, and its values are not embedded again separately. One that does not fit is cut into pieces that render `"key": value` and open with the record's key path and short identifying fields (`[2] {"name": "…", "version": "…"}`).
    *   **Markdown** blocks nested inside a list, list item, blockquote or footnote are not embedded separately from the block that contains them. Every chunk opens with its heading path, and its `crumbtrail` is that path (`Guide::Install`); a heading always travels with the content under it.
        *   Two sections never share a chunk, however small; a short section is a short chunk that still opens with its heading path.
        *   Text is embedded **as written** — backticks, emphasis markers and link URLs included. HTML blocks, front matter and link reference definitions are embedded too.
        *   A thematic break (`---`) is a **break opportunity**: it changes nothing when the content fits, and when a chunk would overflow, the cut lands at the last `---` rather than wherever the size ran out. The bare `---` line itself is not embedded.
    *   **JSON** is embedded **as written**: keys in document order, numbers as typed, and JSONC comments included, each with the member it describes.
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
