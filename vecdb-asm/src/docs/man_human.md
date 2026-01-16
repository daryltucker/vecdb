# vecdb-asm(1) - The Knowledge Assembler

## NAME
**vecdb-asm** - Assembly engine for constructing coherent knowledge from fragmented streams and versioned states.

## SYNOPSIS
`vecdb-asm --strategy [stream|state] [OPTIONS] [INPUT]`

## DESCRIPTION
`vecdb-asm` is a pipe-oriented tool designed to sit between raw data extraction (like `vecq`) and vector ingestion (`vecdb`). It solves the "fragmentation problem" by assembling raw, noisy data into semantic units.

It operates in two distinct modes (strategies):

1.  **Stream Consolidation** (`--strategy stream`):
    For append-only logs (e.g., chat logs, server telemetry). It dedupes and "stitches" overlapping text fragments into a single continuous narrative.

2.  **State Reduction** (`--strategy state`):
    For versioned artifacts (e.g., `task.md`, `task.md.1`). It computes semantic diffs and "timeline trees" to track how a document evolved over time, separating incremental updates from massive rewrites.

## STRATEGIES

### Stream Strategy
Merges overlapping JSON objects (usually from `ag-snatch`).
*   **Input**: JSON/JSONL stream of partial text fragments.
*   **Output**: Deduplicated, stitched JSONL.

**Flags:**
*   `--no-dedupe`: Disable hash-based deduplication.
*   `--stitch`: Enable smart text stitching (merges "Hello Wor" + "lo World" -> "Hello World").

### State Strategy
Analyzes evolution of documents.
*   **Input**: JSON Array of file snapshots (from `vecq --slurp`).
*   **Output**: "Evolution Events" (diffs) or "Timeline Analysis".

**Flags:**
*   `--detect-timelines`: Activates "Big Bang" detection. If a version changes >50% of content, it is branched into a new timeline ID suitable for "Growth-Only" analysis.

## EXAMPLES

**Stitch a conversation stream:**
`cat stream.jsonl | vecdb-asm --strategy stream --stitch`

**Analyze artifact evolution with timeline detection:**
`vecq -t md --slurp $(find . -name "*.resolved.*" | sort) | vecdb-asm --strategy state --detect-timelines`

## SEE ALSO
vecdb(1), vecq(1)
