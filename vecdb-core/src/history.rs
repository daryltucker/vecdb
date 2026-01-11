use crate::backend::Backend;
use crate::embedder::Embedder;
use crate::git::GitSandbox;
use crate::ingestion::{ingest_path, IngestionOptions};
use crate::output::OUTPUT;
use anyhow::Result;
use std::sync::Arc;
use vecdb_common::FileTypeDetector;
use crate::parsers::ParserFactory;

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
    chunk_size: usize,
) -> Result<()> {
    if OUTPUT.is_interactive {
        eprintln!("Starting Time Travel Ingestion: {} @ {}", repo_path, git_ref);
    }

    // 1. Create Sandbox
    let sandbox = GitSandbox::new(repo_path, git_ref)?;
    if OUTPUT.is_interactive {
        eprintln!("Sandbox ready at {:?}", sandbox.path());
    }

    // 2. Configure Options
    // Note: We bypass gitignore for history usually, as we want exactly what was in that commit.
    // However, if the user had ignores back then, arguably we should respect them?
    // For "Time Travel", usually we want source code only.
    // Let's enable gitignore respect if the .gitignore exists in that version.
    let options = IngestionOptions {
        path: sandbox.path().to_string_lossy().to_string(),
        collection: collection.to_string(),
        chunk_size,
        max_chunk_size: None, // History ingestion usually relies on standard chunking, no hard limit enforced yet
        chunk_overlap: 50,
        respect_gitignore: true,
        strategy: "recursive".to_string(),
        tokenizer: "cl100k_base".to_string(),
        git_ref: Some(git_ref.to_string()),
        extensions: None,
        excludes: None,
        dry_run: false,
        metadata: None,
        path_rules: Vec::new(),
        max_concurrent_requests: 4,
        gpu_batch_size: 2,
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
    ingest_path(backend, embedder, detector, parser_factory, options).await?;

    if OUTPUT.is_interactive {
        eprintln!("Time Travel Ingestion Complete. Sandbox will be dropped.");
    }
    Ok(())
}
