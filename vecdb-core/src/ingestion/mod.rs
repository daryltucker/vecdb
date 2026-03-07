pub mod discovery;
pub mod options;
pub mod pipeline;
pub mod processor;
pub mod twopass;

pub use discovery::{build_walker, count_files};
pub use options::IngestionOptions;
pub use pipeline::{flush_chunks, process_content};
pub use processor::process_single_file;

use crate::backend::Backend;
use crate::embedder::Embedder;
use crate::output::OUTPUT;
use crate::parsers::ParserFactory;
use crate::state::IngestionState;
use anyhow::Result;
use indicatif::{ProgressBar, ProgressStyle};
use regex::Regex;
use std::path::Path;
use std::sync::Arc;
use std::time::Duration;
use vecdb_common::{FileType, FileTypeDetector};

/// Orchestrate ingestion of a path
pub async fn ingest_path(
    backend: &Arc<dyn Backend + Send + Sync>,
    embedder: &Arc<dyn Embedder + Send + Sync>,
    detector: &Arc<dyn FileTypeDetector>,
    parser_factory: &Arc<dyn ParserFactory>,
    options: IngestionOptions,
    target_dim: Option<usize>,
) -> Result<()> {
    let job_registry = crate::jobs::JobRegistry::new().ok();
    let job_id = job_registry
        .as_ref()
        .and_then(|r| r.register("ingest", &options.collection).ok());
    if OUTPUT.is_interactive {
        eprintln!("Ingesting path: {}", options.path);
    }

    let mut resolved_dim = target_dim;
    if !backend.collection_exists(&options.collection).await? {
        if OUTPUT.is_interactive {
            eprintln!(
                "Collection {} does not exist. Creating...",
                options.collection
            );
        }
        let dim = embedder.dimension().await?;
        resolved_dim = Some(dim);
        backend
            .create_collection(
                &options.collection,
                dim as u64,
                options.quantization.clone(),
            )
            .await?;
    } else if resolved_dim.is_none() {
        if let Ok(info) = backend.get_collection_info(&options.collection).await {
            resolved_dim = info.vector_size.map(|s| s as usize);
        }
    }

    let commit_sha = crate::git::get_head_sha(Path::new(&options.path)).unwrap_or(None);
    if let Some(ref sha) = commit_sha {
        eprintln!("Detected Git Repo. Injecting commit_sha: {}", sha);
    }

    let root_path_buf = Path::new(&options.path).to_path_buf();
    let root_path = root_path_buf.as_path();
    let mut state = match IngestionState::load(root_path) {
        Ok(s) => s,
        Err(e) => {
            if OUTPUT.is_interactive {
                eprintln!(
                    "Warning: Failed to load ingestion state: {}. Starting fresh.",
                    e
                );
            }
            IngestionState::default()
        }
    };

    // --- Collection ID Resolution Logic ---
    let collection_name = options.collection.clone();

    // 1. Get or Create Remote ID
    // We already ensured collection exists above.
    let remote_id = match backend.get_collection_id(&collection_name).await? {
        Some(id) => id,
        None => {
            // Collection exists but has no ID (legacy or just created without ID).
            // set_collection_id is best-effort: if it fails (e.g. dimension unknown on a
            // freshly created collection), we fall back to a local-only UUID and warn.
            // The worst case is a full re-scan on the next ingest — never data corruption.
            let new_id = uuid::Uuid::new_v4().to_string();
            if let Err(e) = backend.set_collection_id(&collection_name, &new_id).await {
                if OUTPUT.is_interactive {
                    eprintln!(
                        "Warning: Could not persist collection ID for '{}': {}. \
                         Next ingest will perform a full scan.",
                        collection_name, e
                    );
                }
            }
            new_id
        }
    };

    // 2. Check Local State
    let local_id = state.get_collection_id(&collection_name);

    // 3. Reconcile
    if local_id.as_ref() != Some(&remote_id) {
        if OUTPUT.is_interactive {
            if local_id.is_some() {
                eprintln!("Collection ID mismatch (Remote: {}, Local: {:?}). Assuming collection was recreated.", remote_id, local_id);
                eprintln!(
                    "Cleaning up stale tracking data for '{}'...",
                    collection_name
                );
            } else {
                eprintln!(
                    "Initializing tracking for collection '{}' (ID: {})...",
                    collection_name, remote_id
                );
            }
        }

        // This clears the files map for THIS collection and sets the new ID
        state.clear_collection(&collection_name, remote_id.clone());
        // Force save immediately to lock in the new ID
        state.save(root_path)?;
    }

    let mut state_changed = false;

    let builder = build_walker(&options);
    let pb = if OUTPUT.is_interactive {
        eprintln!("Scanning files...");
        let total_files = count_files(&builder);
        eprintln!("Found {} files.", total_files);

        let pb = ProgressBar::new(total_files);
        pb.set_style(ProgressStyle::default_bar()
            .template("{spinner:.green} [{elapsed_precise}] [{bar:40.cyan/blue}] {pos}/{len} ({percent}%) {msg}")
            .unwrap()
            .progress_chars("#>-"));
        pb.enable_steady_tick(Duration::from_millis(100));
        Some(pb)
    } else {
        None
    };

    let walker = builder.build();

    let mut compiled_rules: Vec<Regex> = Vec::new();
    for rule in &options.path_rules {
        match Regex::new(&rule.pattern) {
            Ok(re) => compiled_rules.push(re),
            Err(e) => {
                if OUTPUT.is_interactive {
                    eprintln!("Warning: Invalid Path Rule regex '{}': {}", rule.pattern, e);
                }
            }
        }
    }

    let mut chunks_buffer: Vec<crate::types::Chunk> = Vec::new();
    let batch_size = 20;

    let mut files_scanned = 0;
    let mut files_skipped = 0;
    let mut files_processed = 0;

    let collection_name_batch = options.collection.clone();
    let options_arc = Arc::new(options);

    let semaphore = Arc::new(tokio::sync::Semaphore::new(
        options_arc.max_concurrent_requests,
    ));
    let mut tasks = tokio::task::JoinSet::new();

    // Pipeline Channel: Decouples parsing from embedding
    // Provides 10 batches of "breathing room" before blocking the parser
    let (tx, mut rx) = tokio::sync::mpsc::channel::<Vec<crate::types::Chunk>>(10);

    // Dedicated Embedding Worker
    let backend_embed = backend.clone();
    let embedder_embed = embedder.clone();
    let gpu_batch_size = options_arc.gpu_batch_size;
    let max_chunk_size = options_arc.max_chunk_size;

    let embedding_handle = tokio::spawn(async move {
        while let Some(mut batch) = rx.recv().await {
            flush_chunks(
                &backend_embed,
                &embedder_embed,
                &collection_name_batch,
                &mut batch,
                gpu_batch_size,
                resolved_dim,
                max_chunk_size,
            )
            .await?;
        }
        Ok::<(), anyhow::Error>(())
    });

    'discovery_loop: for result in walker {
        match result {
            Ok(entry) => {
                if entry.file_type().is_some_and(|ft| ft.is_file()) {
                    files_scanned += 1;
                    let path = entry.path().to_path_buf();

                    if path.components().any(|c| c.as_os_str() == ".vecdb") {
                        continue;
                    }

                    let stripped = path.strip_prefix(root_path).unwrap_or(&path);
                    let canonical_root = std::fs::canonicalize(root_path).unwrap_or_else(|_| root_path.to_path_buf());
                    let project_dir_name = canonical_root.file_name().unwrap_or_else(|| std::ffi::OsStr::new(""));
                    let rel_path = if project_dir_name.is_empty() {
                        stripped.to_path_buf()
                    } else {
                        std::path::Path::new(project_dir_name).join(stripped)
                    };

                    if let Ok(meta_hash) = crate::state::compute_file_metadata_hash(&path) {
                        if !state.update_file(&collection_name, rel_path.clone(), meta_hash.clone())
                        {
                            // Skipped
                            if let Some(ref pb) = pb {
                                pb.set_message("⏭️  Skipping...");
                                pb.inc(1);
                            }
                            files_skipped += 1;
                            continue;
                        }
                        state_changed = true;
                    } else {
                        state_changed = true;
                    }

                    // Not skipped - Ingesting
                    if let Some(ref pb) = pb {
                        let short_path = rel_path.to_string_lossy();
                        let msg = if short_path.len() > 40 {
                            format!(
                                "📥 ...{}",
                                &short_path[short_path.len().saturating_sub(37)..]
                            )
                        } else {
                            format!("📥 {}", short_path)
                        };
                        pb.set_message(msg);
                        pb.inc(1);
                    }

                    let permit = semaphore.clone().acquire_owned().await?;

                    let detector = detector.clone();
                    let parser_factory = parser_factory.clone();
                    let rules = compiled_rules.clone();
                    let options_ref = options_arc.clone();
                    let commit_sha = commit_sha.clone();

                    tasks.spawn(async move {
                        let _permit = permit;
                        process_single_file(
                            path,
                            rel_path,
                            detector,
                            parser_factory,
                            rules,
                            options_ref,
                            commit_sha,
                        )
                        .await
                    });
                }
            }
            Err(err) => {
                if let Some(ref pb) = pb {
                    pb.suspend(|| eprintln!("Error walking directory: {}", err));
                } else if OUTPUT.is_interactive {
                    eprintln!("Error walking directory: {}", err);
                }
            }
        }

        // Drain finished parsing tasks while discovery continues
        while let Some(res) = tasks.try_join_next() {
            match res {
                Ok(Ok(Some(mut file_chunks))) => {
                    files_processed += 1;
                    chunks_buffer.append(&mut file_chunks);

                    if chunks_buffer.len() >= batch_size {
                        let batch = std::mem::take(&mut chunks_buffer);
                        if (tx.send(batch).await).is_err() {
                             // Background worker failed. Break to catch the real error below.
                             break 'discovery_loop;
                        }
                    }

                    if let Some(ref j_id) = job_id {
                        if let Some(ref r) = job_registry {
                            let total_files = files_scanned.max(1);
                            let finished = files_processed + files_skipped;
                            let _ = r.update_progress(j_id, finished as f32 / total_files as f32);
                        }
                    }
                }
                Ok(Ok(None)) => {
                    files_skipped += 1;
                }
                Ok(Err(e)) => {
                    if OUTPUT.is_interactive {
                        eprintln!("File processing error: {}", e);
                    }
                }
                Err(e) => {
                    if OUTPUT.is_interactive {
                        eprintln!("Task join error: {}", e);
                    }
                }
            }
        }
    }

    // Pass 2: Finish all pending parsing tasks
    'parsing_finish: while let Some(res) = tasks.join_next().await {
        match res {
            Ok(Ok(Some(mut file_chunks))) => {
                files_processed += 1;
                chunks_buffer.append(&mut file_chunks);
                if chunks_buffer.len() >= batch_size {
                    let batch = std::mem::take(&mut chunks_buffer);
                    if (tx.send(batch).await).is_err() {
                        break 'parsing_finish;
                    }
                }
            }
            Ok(Ok(None)) => {
                files_skipped += 1;
            }
            Ok(Err(e)) => {
                if OUTPUT.is_interactive {
                    eprintln!("File processing error: {}", e);
                }
            }
            Err(e) => {
                if OUTPUT.is_interactive {
                    eprintln!("Task join error: {}", e);
                }
            }
        }

        if let Some(ref j_id) = job_id {
            if let Some(ref r) = job_registry {
                let total_files = files_scanned.max(1);
                let finished = files_processed + files_skipped;
                let _ = r.update_progress(j_id, finished as f32 / total_files as f32);
            }
        }
    }

    // Flush last batch
    if !chunks_buffer.is_empty() {
        let _ = tx.send(chunks_buffer).await;
    }

    // Signal completion to embedding worker
    drop(tx);
    embedding_handle.await
        .map_err(|e| anyhow::anyhow!("Embedding background task panicked: {}", e))??;


    if let Some(ref j_id) = job_id {
        if let Some(ref r) = job_registry {
            let _ = r.complete(j_id);
        }
    }

    if state_changed {
        state.touch_collection(&collection_name);
        if let Err(e) = state.save(root_path) {
            let msg = format!("Warning: Failed to save ingestion state: {}", e);
            if let Some(ref pb) = pb {
                pb.suspend(|| eprintln!("{}", msg));
            } else if OUTPUT.is_interactive {
                eprintln!("{}", msg);
            }
        }
    }

    if let Some(ref pb) = pb {
        pb.finish_with_message("Done");
    }

    eprintln!(
        "Ingestion Summary: Scanned {}, Processed {}, Skipped {}",
        files_scanned, files_processed, files_skipped
    );

    Ok(())
}

#[allow(clippy::too_many_arguments)]
/// Ingest raw content from memory
pub async fn ingest_memory(
    backend: &Arc<dyn Backend + Send + Sync>,
    embedder: &Arc<dyn Embedder + Send + Sync>,
    content: &str,
    metadata: std::collections::HashMap<String, serde_json::Value>,
    collection: &str,
    chunk_size: Option<usize>,
    max_chunk_size: Option<usize>,
    chunk_overlap: Option<usize>,
    quantization: Option<crate::config::QuantizationType>,
    target_dim: Option<usize>,
) -> Result<()> {
    let options = IngestionOptions {
        path: "memory".to_string(),
        collection: collection.to_string(),
        chunk_size: chunk_size.unwrap_or(512),
        max_chunk_size,
        chunk_overlap: chunk_overlap.unwrap_or(50),
        respect_gitignore: false,
        strategy: "recursive".to_string(),
        tokenizer: "cl100k_base".to_string(),
        git_ref: None,
        extensions: None,
        excludes: None,
        dry_run: false,
        metadata: None,
        path_rules: Vec::new(),
        max_concurrent_requests: 4,
        gpu_batch_size: 2,
        quantization,
    };

    let mut chunks = process_content(
        content,
        &options,
        Path::new("memory"),
        &metadata,
        FileType::Text,
    )
    .await?;

    let mut resolved_dim = target_dim;
    if !backend.collection_exists(collection).await? {
        eprintln!("Collection {} does not exist. Creating...", collection);
        let dim = embedder.dimension().await?;
        resolved_dim = Some(dim);
        backend
            .create_collection(collection, dim as u64, options.quantization.clone())
            .await?;
    } else if resolved_dim.is_none() {
        if let Ok(info) = backend.get_collection_info(collection).await {
            resolved_dim = info.vector_size.map(|s| s as usize);
        }
    }

    flush_chunks(
        backend,
        embedder,
        collection,
        &mut chunks,
        options.gpu_batch_size,
        resolved_dim,
        max_chunk_size,
    )
    .await?;

    Ok(())
}
