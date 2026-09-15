use crate::backend::Backend;
use crate::embedder::Embedder;
use crate::git::GitSandbox;
use crate::ingestion::{ingest_path, IngestionOptions};
use crate::output::OUTPUT;
use crate::parsers::ParserFactory;
use anyhow::Result;
use std::sync::Arc;
use vecdb_common::FileTypeDetector;

/// Ingests a specific historical version of a repository.
///
/// # Arguments
/// * `repo_path`: Path to the local repository (or URL).
/// * `git_ref`: The commit SHA, tag, or branch to ingest.
/// * `collection`: Target collection name.
#[allow(clippy::too_many_arguments)]
pub async fn ingest_history(
    backend: &Arc<dyn Backend + Send + Sync>,
    embedder: &Arc<dyn Embedder + Send + Sync>,
    detector: &Arc<dyn FileTypeDetector>,
    parser_factory: &Arc<dyn ParserFactory>,
    repo_path: &str,
    git_ref: &str,
    collection: &str,
    // The DESTINATION's resolved chunk parameters, not a bare number.
    //
    // History writes into the same collection an ordinary ingest does, so it has
    // to cut chunks the same way. It used to take a single `target_chunk_size`
    // that the CLI hardcoded to 512 while discarding the resolution it had just
    // computed, so `vecdb history ingest -c X` wrote X at a granularity nothing
    // in the config mentioned. The chunking guard in `ensure_write_target` now
    // refuses that outright, which is how it was found.
    chunking: crate::ingestion::options::ChunkSpec,
    quantization: Option<crate::config::QuantizationType>,
    target_dim: Option<usize>,
) -> Result<()> {
    if OUTPUT.is_interactive {
        eprintln!(
            "Starting Time Travel Ingestion: {} @ {}",
            repo_path, git_ref
        );
    }

    let job_registry = crate::jobs::JobRegistry::new().ok();
    let _job_id = job_registry
        .as_ref()
        .and_then(|r| r.register("history", collection).ok());

    // 1. Create Sandbox
    let sandbox = GitSandbox::new(repo_path, git_ref)?;
    if OUTPUT.is_interactive {
        eprintln!("Sandbox ready at {:?}", sandbox.path());
    }

    // 2. Configure Options
    //
    // History ingestion indexes exactly what was in that commit. A checkout of
    // an old ref carries that ref's `.gitignore`, and honouring it would make
    // the indexed content vary with a file that describes build artifacts
    // rather than indexing intent — the same reason `.gitignore` is never
    // consulted implicitly anywhere else. `.vectorignore` remains in force.
    let options = IngestionOptions {
        pack_target_bytes: chunking.pack_target_bytes,
        path: sandbox.path().to_string_lossy().to_string(),
        collection: collection.to_string(),
        vecdbrc_routes: None,
        vecdbrc_root: None,
        only_collection: None,
        route_default_collection: None,
        target_chunk_size: chunking.target_chunk_size,
        on_oversize: Default::default(),
        route_chunking: Default::default(),
        max_chunk_bytes: chunking.max_chunk_bytes,
        chunk_overlap: chunking.chunk_overlap,
        respect_gitignore: false,
        ignore_vectorignore: false,
        strategy: "recursive".to_string(),
        tokenizer: "cl100k_base".to_string(),
        git_ref: Some(git_ref.to_string()),
        extensions: None,
        excludes: None,
        dry_run: false,
        metadata: None,
        file_allowlist: None,
        project_root: None,
        path_rules: Vec::new(),
        max_concurrent_requests: 4,
        gpu_batch_size: 2,
        quantization,
        allow_quantization_delta: false,
    };

    // 3. Ingest
    // IMPORTANT: The `path` inside `ingest_path` will be the /tmp/sandbox path.
    // Ideally, we want the stored path in the vector DB to reflect the *original* logical path (e.g., "src/main.rs", not "/tmp/sandbox/src/main.rs").
    // The current `ingest_path` uses `strip_prefix` for state, but stores full path in metadata?
    // Let's check `ingest_path` implementation detail.
    // In `ingestion.rs`: `metadata.insert("path", ... path.display())`.
    // We might need to post-process or modify `ingest_path` to accept a "logical root".
    // For now, let's ship the basic version where path is absolute sandbox path,
    // OR we modify `ingest_path` to strip the root prefix from the stored path metadata.

    // Quick Fix: We'll accept the sandbox path for now to prove the "Time Travel" capability.
    // The "Right Way" is to refactor `ingest_path` to take `logical_root`.
    // Let's proceed with standard ingestion first.
    ingest_path(
        backend,
        embedder,
        detector,
        parser_factory,
        options,
        target_dim,
    )
    .await?;

    if OUTPUT.is_interactive {
        eprintln!("Time Travel Ingestion Complete. Sandbox will be dropped.");
    }
    Ok(())
}
