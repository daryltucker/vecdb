# Vector Facets & Smart Routing

> **Philosophy**: *"Broad filters for retrieval, specific filters for refinement."*

## What are Facets?
In Vector Search, a **Facet** is a metadata tag attached to your content (e.g., `language=rust`, `platform=linux`, `year=2024`).

While **Embeddings** (Vectors) capture *semantic meaning* ("how to compile kernel"), **Facets** capture *discrete properties* ("on Linux").

Facets allow you to slice your knowledge base into deterministic buckets before asking the AI to find relevant content. This is much faster and more accurate than asking the AI to "ignore Windows results" from a soup of mixture vectors.

## The Problem: "Embedding Dilution"
If you ingest thousands of documents about "Installation", some for Windows, some for Linux, and some for macOS, they all cluster together in vector space because they are semantically similar.

When you search for *"install on ubuntu"*:
1.  The vector for "install" matches all OS guides powerfully.
2.  The vector for "ubuntu" pulls the result slightly towards Linux.
3.  **Result**: You often get Windows installation guides because the "Install" signal overwhelms the "Ubuntu" signal. The specificity is "diluted."

**The Solution**: Smart Routing.
Instead of hoping the embedding model understands "Ubuntu", we detect the keyword `ubuntu` in your query and apply a **Hard Filter**:
`search("install", filter={ platform: "linux" })`

Now, the vector search ONLY runs against Linux documents. The Windows documents might as well not exist.

## Smart Routing (Auto-Detection)
`vecdb` includes a **Dynamic Router** that listens to specific metadata keys (configured in `config.toml`).

If your query contains a value that matches a known facet (e.g., you type "python"), `vecdb` automatically applies the filter `language=python`.

### How it works
1.  **Discovery**: `vecdb` checks what values exist in your database for configured keys.
2.  **Matching**: When you search, it checks if any of those values appear in your query (using precise whole-word matching).
3.  **Filtering**: If a match is found (e.g., "rust"), it locks the search to that subset.

**Example**:
```bash
# Data in DB:
# doc1: { content: "...", metadata: { platform: "windows" } }
# doc2: { content: "...", metadata: { platform: "linux" } }

vecdb search "setup on windows"
# Router detects "windows" -> Applies filter: platform="windows"
# Only doc1 is searched.
```

## Smart Ingestion (Path Parsing)
While Facets are powerful, manually tagging files with `vecdb ingest -m year=2025` is tedious. 
**Path Parsing Rules** allow you to extract metadata automatically from your directory structure using Regex.

### How to use
Add `[[ingestion.path_rules]]` to your `config.toml`:

```toml
[[ingestion.path_rules]]
# Matches: invoices/2025/Q1/doc.pdf
# Use Python/Rust style named groups (?P<name>...)
pattern = "invoices/(?P<year>\\d{4})/(?P<quarter>Q\\d)/.*"

# Matches: src/v1.2.0/main.rs
[[ingestion.path_rules]]
pattern = "src/(?P<version>v\\d+\\.\\d+\\.\\d+)/.*"
```

Now, when you run `vecdb ingest`, files matching these patterns will automatically have `year=2025` or `version=v1.2.0` attached as metadata. This works perfectly with Smart Routing!

## The "Refinement Strategy" (Broad to Specific)
A common mistake is to make facets too granular too early (e.g., `ubuntu-22.04`). This leads to fragmented data where a search for "linux" misses "ubuntu" results.

**Best Practice**: Use broad primary facets, and refine later.
1.  **Ingest Broadly**: Tag content with `platform=linux` or `platform=windows`.
2.  **Route Broadly**: Let Smart Routing guide users to the "Linux" bucket.
3.  **Refine Later**: You can update metadata later to `platform=linux.ubuntu` without re-embedding! Qdrant supports hierarchical filtering.

## Configuration
You control which keys `vecdb` monitors for routing. This is defined in your `config.toml`.

**Default Configuration**:
```toml
[smart_routing]
# Keys to monitor. vecdb will scan the DB for values in these fields.
keys = ["language", "source_type"]
```

**Custom Configuration (Power User)**:
If you want to route by `platform` (OS) or `project` (Project Name), add them:
```toml
[smart_routing]
keys = ["language", "source_type", "platform", "project"]
```
*Note: Only enable keys that you strictly populate. If you enable `platform` but only 10% of your docs have it, you might accidentally hide 90% of your docs when a user types "windows".*

## FAQ

### Q: Does `vecdb` automatically know that "Ubuntu" means "Linux"?
**No.** `vecdb` is not an LLM. It is a deterministic engine.
If you have docs tagged `platform=ubuntu` and docs tagged `platform=linux`, they are separate buckets.
*Tip: Use the Refinement Strategy. Tag everything as components of a larger whole if you want them searchable together.*

### Q: I typed "win", why didn't it match "Windows"?
**Safety.** We use "Word Boundary" matching. We don't want "formatting" to match "for". You must type the full facet value.

### Q: Can I turn this off?
**Yes.** You can disable Smart Routing per query or globally by removing keys from `config.toml`.
