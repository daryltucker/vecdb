# Chunking & Context Settings Explained

> **CORRECTED 2026-08-24.** An earlier revision of this document described
> `num_ctx` as the *generating* LLM's context window and reserved tokens for
> output. **vecdb never generates anything** — it is a vector database. `num_ctx`
> is sent to the *embedding* model's `/api/embed`, which has no generation phase
> and no output reserve.
>
> That misconception was load-bearing: it is where the `(num_ctx * 0.75 /
> target_chunk_size)` batch-size formula came from — the `0.75` was "reserve 25% for
> generation" — and that formula divided tokens by characters and has since been
> removed. The same revision called `max_chunk_bytes` (then `max_chunk_bytes`) a limit in *tokens*; it is
> compared against `String::len()` and is **bytes**.
>
> Phase 2 below describes a RAG application *built on* vecdb, not vecdb itself.
> It is retained because the distinction is the point of the document.

This document explains how `num_ctx`, `target_chunk_size`, `max_chunk_bytes`, and
`chunk_overlap` work together, and which of them vecdb actually controls.

---

## The Two Different Contexts

Two things get called "context" and they are not the same.

| Parameter | What it controls | Whose |
|-----------|------------------|-------|
| `num_ctx` | The **embedding model's** input limit — the longest text `/api/embed` will accept. Sent verbatim; vecdb never scales or clamps it. | vecdb's |
| `target_chunk_size` | Target size when splitting a document at ingest. | vecdb's |
| *(a generating LLM's window)* | How much retrieved text your assistant can read when answering. | **not vecdb's** |

The third is what the rest of this document's Phase 2 is about. vecdb stores and
retrieves; something else generates.

### `num_ctx` is the real ceiling — `context_length` is not

Measured 2026-08-24 against `qwen3-embedding:0.6b-q8_0`, which *declares*
`context_length = 32768`:

| request | ~4086-token input | ~12258-token input |
|---|---|---|
| no `num_ctx` | accepted (boundary) | **refused** |
| `num_ctx = 16384` | — | **accepted** |

So `/api/embed` **does** honour `options.num_ctx`, and a model's declared
`context_length` is the *maximum you may request*, not what you get — without
`num_ctx` you get Ollama's default of 4096.

---

## The RAG Workflow

### Phase 1: Ingest (Breaking Documents into Chunks)

```
Document: "My Rust Tutorial - Chapter 1" (10,000 tokens)
                           ↓
              chunker.chunk(content, target_chunk_size=2048)
                           ↓
   ┌──────────┐ ┌──────────┐ ┌──────────┐ ┌──────────┐
   │ Chunk 1  │ │ Chunk 2  │ │ Chunk 3  │ │ Chunk 4  │
   │ (~2048)  │ │ (~2048)  │ │ (~2048)  │ │ (~2048)  │
   └──────────┘ └──────────┘ └──────────┘ └──────────┘
         ↓            ↓            ↓            ↓
   ┌──────────┐ ┌──────────┐ ┌──────────┐ ┌──────────┐
   │ Embed #1 │ │ Embed #2 │ │ Embed #3 │ │ Embed #4 │
   │ (vector) │ │ (vector) │ │ (vector) │ │ (vector) │
   └──────────┘ └──────────┘ └──────────┘ └──────────┘
                           ↓
              All stored in Qdrant as separate points
```

### Phase 2: Query (Retrieving Relevant Context)

```
Your Query: "How do I handle memory in Rust?"
                           ↓
   Query Vector → Semantic Search in Vector DB
                           ↓
   Finds TOP-K most similar chunks across ALL documents
   (e.g., Chunk 2 from Rust tutorial, Chunk 7 from C++ guide)
                           ↓
   Retrieved chunks sent to LLM with your question
                           ↓
   LLM generates answer using the retrieved context
```

---

## Why These Specific Values?

### num_ctx = 8192

This is the **LLM's context window** - the maximum tokens the model can "see" when generating a response.

```
┌─────────────────────────────────────────────────────────────────┐
│                    8192 TOKEN CONTEXT WINDOW                   │
├─────────────────────────────────────────────────────────────────┤
│   [ SYSTEM PROMPT ]  ──▶ ~500 tokens (fixed cost)              │
│   [ YOUR QUESTION ]  ──▶ ~100 tokens (query)                   │
│   [ RETRIEVED DOCS ]  ──▶ ~6000 tokens (variable)              │
│   [ RESPONSE SPACE ]  ──▶ ~1500 tokens reserved for generation │
└─────────────────────────────────────────────────────────────────┘
```

**Important**: You need to reserve ~1500-2000 tokens for generation (the KV cache / output). The model needs room to "think" and produce output, not just read input!

### target_chunk_size = 2048

This controls **how documents are split at ingest time**. Any document exceeding this size gets split into multiple chunks, each embedded as a separate vector.

**Why 2048 and not 4096 or 8192?**

```
Available for chunks = num_ctx - (system + query + generation_reserve)
                    = 8192 - 2100 (roughly)
                    = ~6092 tokens

6092 / 2048 = 2.97 chunks ≈ 3 chunks ✓
```

With your settings, you get **~3 chunks per query**, which is the "Goldilocks zone" for most RAG applications:

| Chunk Size | Chunks in Context | Trade-off |
|------------|-------------------|-----------|
| 1024 | ~5-6 | More chunks, but each has less complete context |
| **2048** | **~3** | **Sweet spot - complete thoughts, good diversity** |
| 4096 | ~1-2 | Less diverse information |
| 8192 | 1 | Very limited context for the LLM |

### max_chunk_bytes — **bytes**, not tokens

A hard cap on any single chunk, compared against `String::len()`. It is **bytes**,
while `target_chunk_size` counts whatever `tokenizer` counts (tokens under the default
`cl100k_base`). The two are not interchangeable and the gap is large: measured on
real source, a 6144-token chunk weighs about **31.9 KB**.

That matters more than "safety cap" suggests. A ceiling below the byte weight of
a full-size chunk is not a safety net — it fires on essentially every chunk and
becomes a second, fixed-width chunking pass that discards the boundaries the
first pass established. Leave it unset unless you have measured your corpus; the
derived default (`target_chunk_size × 6`, see `config::BYTES_PER_CHUNK_UNIT`) is chosen
to clear real content with headroom.

`on_oversize` decides what happens when it does fire: `"split"` keeps the content
and labels the parts, `"skip"` refuses the insert and reports it. Neither
truncates, and neither aborts the run.

### chunk_overlap = 256

```
Chunk 1: [............XXXX............]
                    ↓ 256 tokens shared
Chunk 2:           [............XXXX............]
```

This ensures **context continuity**. If a concept spans the boundary between two chunks (like a function definition split across paragraphs), the overlap ensures the LLM can still understand it.

---

## Why Not Round Numbers?

You might wonder: "Why not set num_ctx to 9000 so it divides evenly into 2048?"

The answer: **Most LLMs have hardcoded context limits**

| Model | Max Context |
|-------|-------------|
| nomic-embed-text | 8192 |
| Qwen3-Embedding-4B | 8192 |
| bge-m3 | 8192 |
| some older models | 4096 |

These are typically **powers of 2** for GPU memory alignment efficiency. The model literally *cannot* process more than its max, and Ollama will clamp or reject values outside the supported range.

---

## TL;DR Summary

```
┌─────────────────────────────────────────────────────────┐
│ Your settings are well-tuned:                           │
│                                                         │
│   num_ctx = 8192      ← LLM can see 8K tokens           │
│   target_chunk_size = 2048   ← ~3 chunks fit in context        │
│   max_chunk_bytes = 3072  ← Never exceeds 3K (safe!)    │
│   chunk_overlap = 256  ← Context flows between chunks   │
└─────────────────────────────────────────────────────────┘
```

The key insight: **target_chunk_size is both an ingest-time and query-time setting**. At ingest, it controls how documents are split into searchable vectors. At query time, it determines how many of those vectors fit in the LLM's context window.

---

## Related Docs

- [CONFIG.md](CONFIG.md) - Full configuration reference
- [EMBEDDING_MODELS.md](EMBEDDING_MODELS.md) - Embedding model options
