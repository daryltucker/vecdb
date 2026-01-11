# vecdb-cli

CLI tool for `vecdb`: Ingestion and Management.

## Usage

### Ingestion
```bash
# Ingest a directory into the 'docs' collection
vecdb ingest ./my_docs

# Ingest from stdin (Pipe Mode)
echo "Hello World" | vecdb ingest - --collection test
```

### Search
```bash
# Simple search in 'docs'
vecdb search "my query"

# Search in a specific collection
vecdb search "query" --collection my_collection
```

## GCC Multi-Version Verification

This feature demonstrates context isolation by routing queries to specific versioned collections based on prefixes.

### Smart Routing
Commands like `vecdb search "In GCC 13, ..."` are automatically routed to the `gcc13` collection.

#### Verification Examples

1. **GCC 13 Highlights**
   ```bash
   vecdb search "In GCC 13, what are the highlights?"
   ```
   *Expectation*: CLI logs "Routing query to collection: gcc13" and returns GCC 13 specific C++23 features.

2. **GCC 14 Highlights**
   ```bash
   vecdb search "In GCC 14, what's new with printing?"
   ```
   *Expectation*: CLI logs "Routing query to collection: gcc14" and returns details about `std::print`.

3. **Context Isolation Test**
   ```bash
   vecdb search "In GCC 13, tell me about std::print"
   ```
   *Expectation*: Low relevance or unrelated results, as `std::print` is a GCC 14 specific highlight.

---

For more details on implementation, see `vecdb-core` and the [walkthrough.md](../docs/walkthrough.md).
