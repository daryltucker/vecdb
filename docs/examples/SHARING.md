# Sharing Knowledge Packs

You can export your trained `vecdb` collections as "Knowledge Packs" to share with other developers or agents. This allows you to distribute specific domain knowledge (like a "Project Manual" or "Legally Compliant Agent Laws") without requiring everyone to re-ingest raw files.

## 1. Create a Snapshot
First, create a snapshot of your desired collection.

```bash
# Default collection (usually 'docs')
vecdb snapshot --create

# Specific collection
vecdb snapshot --create --collection my_agent_brain
```
*Output:* `Snapshot created: my_agent_brain-2026-01-11....snapshot`

## 2. List & Download
Identify the snapshot you want to share.

```bash
# List available snapshots
vecdb snapshot --list

# Download to local file
vecdb snapshot --download my_agent_brain-2026-01-11....snapshot
```
This produces a single `.snapshot` file that contains all vectors and payloads.

## 3. Share
Move the `.snapshot` file to shared storage (S3, Shared Drive, Git LFS).

## 4. Restore (Import)
To import a shared knowledge pack:

```bash
# Restore file to a collection
vecdb snapshot --restore ./shared_brain.snapshot --collection team_docs
```

> **Note**: Restoring overwrites/merges into the target collection. It is recommended to restore into a fresh collection name to avoid conflicts.
