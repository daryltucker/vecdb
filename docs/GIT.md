# Git Integration Strategy

> **Status**: Active / Partially Implemented
> **Phase**: 1 Completed, 2 In Progress, 3 Deferred
> **Scope**: Defines how `vecdb-mcp` interacts with Git repositories.

---

## 1. The Prime Directive: Read-Only Filesystem

**Rule**: The CLI and MCP tools must **NEVER** modify the user's working directory.
- No `git checkout` in the user's repo.
- No `git reset` or `git clean`.
- Metadata extraction (`git rev-parse`) is allowed.

**Rationale**: `vecdb` is an observer, not a participant. Modifying the user's files risks data loss and workspace corruption.

---

## 2. Primary Use Case: "The Now State"

The core function of git integration is to contextually valid embeddings for the **current state** of the code, including:
1.  **Tracked Files**: Files that are committed.
2.  **Divergences**: Local, uncommitted modifications.
3.  **Metadata**: Tagging vectors with the `commit_sha` of the base state.

**Implementation**:
- Ingestion checks `git rev-parse HEAD`.
- Injects `commit_sha` into `Document` metadata.
- Processing includes uncommitted changes (reading from FS, not git object database).

---

## 3. Phase 1: Metadata Injection (Completed)

We inject the current Commit SHA into every document ingested.
- **Goal**: Enable future filtering by commit version.
- **Method**: Client-side detection via `git` command wrapper.

---

## 4. Phase 2: Incremental Ingestion (Active)

We use Git only as a signal for optimization, but rely on **Content Hashing** for truth.
- **Problem**: Re-embedding 10,000 files is slow/costly.
- **Solution**:
    1.  Maintain `.vecdb/state.toml` (Path -> Hash).
    2.  Check file content hash before embedding.
    3.  If unchanged, skip embedding (reuse existing vector if ID deterministic).
    4.  Update state on success.
- **Git's Role**: `git diff` can be an optimization hint, but content hash is the authority.

---

## 5. Phase 3: Sandboxed "Time Travel" (Completed)

**Requirement**: To ingest a historic version (e.g., "How did this look in v1.0?"):
1.  **NEVER** touch the user's working directory.
2.  **Clone** the repo to a temporary cache directory (Sandbox).
3.  **Checkout** the target SHA/Tag in the sandbox.
4.  **Ingest** from the sandbox path.
5.  **Discard** (or cache) the sandbox.

**ID Generation**:
- To allow coexistence of `HEAD` and `v1.0`, IDs are composites: `Hash(path + commit_sha + content)`.

**Commands**:
- CLI: `vecdb history ingest --path . --git-ref <SHA>`
- MCP: `ingest_historic_version(repo_path, git_ref)`

**MCP Tool**: `ingest_historic_version(repo_url, sha)`
- This runs entirely isolated from the user's current work.

---
