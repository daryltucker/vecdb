# Database Management

`vecdb` provides several commands to manage your vector collections and their associated data.

## Checking Status

To check the health and status of your collections, used the `status` command (or `list`).

```bash
vecdb list
```

This will show:
- Collection Names
- Vector Counts
- Vector Dimensions
- Quantization Status (`Scalar`, `Binary`, or `None`)
- Memory Usage estimates (factoring in quantization)

## Deleting Collections

Deletions are permanent. `vecdb` implements a "Two-Key Turn" safety mechanism for interactive use.

### Interactive Deletion (Safe Mode)

```bash
vecdb delete my_collection
```
You will be prompted to type a randomly generated security token to confirm the action.

### Automated Deletion (CI/CD)

For scripts or when you are sure, you can bypass the prompt:

```bash
vecdb delete my_collection --yes
```

To delete **ALL** collections:

```bash
vecdb delete --all --yes
```

## Automatic State Cleanup

When you ingest files, `vecdb` tracks which files have been indexed in a local `.vecdb/state.toml` file to avoid re-processing unchanged files (Incremental Ingestion).

### The "Stale State" Problem
Previously, if you deleted a collection on the server but didn't delete the local `.vecdb` folder, `vecdb` would think it had already ingested the files and skip them, resulting in an empty collection.

### The Solution: Collection Identity
`vecdb` now assigns a unique **UUID** to every collection upon creation. This ID is stored in the vector database itself (as a distinct "Genesis Point").

When you run `vecdb ingest`:
1. It fetches the **Remote UUID** from the collection.
2. It compares it with the **Local UUID** stored in `.vecdb/state.toml`.
3. **Mismatch Detected?** If the IDs don't match (meaning the collection was deleted and recreated), `vecdb` automatically:
   - Wipes the stale tracking data for *that specific collection*.
   - Updates the Local UUID to match the Remote UUID.
   - Proceeds to full ingest.

This ensures that `vecdb delete` + `vecdb ingest` works seamlessly without manual cleanup.

## Quantization

Quantization compresses vector data to reduce memory usage (RAM/VRAM) and improve search speed, often with minimal loss in accuracy.

### Modes

| Mode | Description | Memory Reduction | Accuracy | Use Case |
|------|-------------|------------------|----------|----------|
| `none` | Full `f32` vectors (Default) | 1x | 100% | High-precision requirements, small datasets (<1M vectors). |
| `scalar` | `Int8` quantization | ~4x | ~99% | **Recommended**. Good balance for most production workloads. |
| `binary` | 1-bit quantization | ~32x | ~90-95% | Massive datasets (>10M vectors), initial coarse re-ranking. |

### Usage

You can specify quantization during ingestion. This setting is applied when the collection is **created**.

```bash
vecdb ingest ./docs --quantization scalar
```
