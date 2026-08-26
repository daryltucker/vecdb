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
Instead of hoping the embedding model understands "Ubuntu", you state the constraint and `vecdb` applies a **Hard Filter**:
`search("install", filter={ platform: "linux" })`

Now, the vector search ONLY runs against Linux documents. The Windows documents might as well not exist.

## Smart Routing (`key:value` qualifiers)
`vecdb` includes a **Dynamic Router** that recognizes specific metadata keys (configured in `config.toml`).

Opt in with `--smart`. Scoping is then **written into the query, never inferred from it**:

```bash
vecdb search "setup instructions platform:windows" --smart
```

### How it works
1.  **Parse**: Tokens of the exact form `key:value`, where `key` is a configured
    routing key, are lifted out of the query and become filters.
2.  **Strip**: The qualifier is removed from the text before embedding. Leaving
    `platform:windows` in would pollute the vector with tokens you meant as
    metadata, not as meaning.
3.  **Validate**: The value is checked against what actually exists in the
    collection. An unknown value is an **error listing the valid values** — not
    an empty result set, which is indistinguishable from "no answer here".
4.  **Report**: The filters that were applied come back in `applied_filters`, so
    you can always see how your search was narrowed.

Everything else is left alone: bare words, URLs (`https://…`), and `key:value`
pairs whose key is not a configured routing key all stay in the query text.

**Example**:
```bash
# Data in DB:
# doc1: { content: "...", metadata: { platform: "windows" } }
# doc2: { content: "...", metadata: { platform: "linux" } }

vecdb search "setup platform:windows" --smart
# → filter platform="windows"; embeds "setup"; only doc1 is searched.
# → applied_filters: {"platform": "windows"}

vecdb search "how do I set up windows" --smart
# → NO filter. "windows" is prose, not a qualifier. Both docs are searched.
```

### Why qualifiers instead of auto-detection

> **Changed in the 2026-234 release.** Earlier versions scanned the query for any
> bare word matching a known facet value and filtered on it automatically.

That was removed because it silently answered a different question than the one
asked. `"how do I parse rust files"` became `language=rust`, hiding every result
in every other language, with no way to see it had happened and no way to turn
it off. Worse, it was invisible: the caller saw a plausible, short result list
and concluded the corpus was thin.

Explicit qualifiers are predictable, greppable, reportable, and trivially
disabled (omit `--smart`, or pass `--no-smart`). A filter you did not ask for is
not a convenience.

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
**Values are matched exactly.** `platform:win` is not `platform:windows`. A
prefix match would silently widen or narrow your search depending on what else
is in the collection. Unknown values produce an error listing the valid ones, so
you will never have to guess.

### Q: Can I turn this off?
**It is off by default.** Qualifier parsing only runs with `--smart`. Pass
`--no-smart` to be explicit, or remove keys from `config.toml` to disable a key
globally.

### Q: How do I know whether a filter was applied?
Read `applied_filters` in the response. It is always present, and it is empty
when nothing was filtered. If a result list looks surprisingly short, check it
before concluding the collection is missing content.
