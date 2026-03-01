# Chunking & Context Settings Explained

This document explains how `num_ctx`, `chunk_size`, `max_chunk_size`, and `chunk_overlap` work together in vecdb's RAG pipeline.

---

## The Two Different Contexts

You're probably mixing up two completely different contexts. They're not the same!

| Parameter | What it controls | Your Value |
|-----------|------------------|------------|
| `num_ctx` | **LLM's context window** - how many tokens the LLM can see when *generating* an answer | 8192 |
| `chunk_size` | **Vector DB chunk size** - how big each stored document piece is when *embedding* | 2048 |

These operate at **completely different phases** of RAG!

---

## The RAG Workflow

### Phase 1: Ingest (Breaking Documents into Chunks)

```
Document: "My Rust Tutorial - Chapter 1" (10,000 tokens)
                           ↓
              chunker.chunk(content, chunk_size=2048)
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

### chunk_size = 2048

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

### max_chunk_size = 3072

This is a **safety cap** - a hard limit on any single chunk.

Sometimes a document section is hard to split cleanly (e.g., a long code block or paragraph). The chunker tries for 2048, but might overshoot to 2500 or 2800 tokens.

`max_chunk_size = 3072` ensures:
- **Never more than 3072 tokens per chunk**
- Even the worst chunk fits comfortably in context (3072 < 8192)
- Prevents "chunk inflation" from breaking things

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
│   chunk_size = 2048   ← ~3 chunks fit in context        │
│   max_chunk_size = 3072  ← Never exceeds 3K (safe!)    │
│   chunk_overlap = 256  ← Context flows between chunks   │
└─────────────────────────────────────────────────────────┘
```

The key insight: **chunk_size is both an ingest-time and query-time setting**. At ingest, it controls how documents are split into searchable vectors. At query time, it determines how many of those vectors fit in the LLM's context window.

---

## Related Docs

- [CONFIG.md](CONFIG.md) - Full configuration reference
- [EMBEDDING_MODELS.md](EMBEDDING_MODELS.md) - Embedding model options
