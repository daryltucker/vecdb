pub mod discovery;
pub mod options;
pub mod pipeline;
pub mod processor;
pub mod twopass;

/// Remove the points a document used to occupy, keeping only the chunks it
/// produces now.
///
/// A chunk ID is a UUIDv5 over the content, so re-ingesting an edited file
/// writes the new version under a new ID and leaves the old point behind.
/// Nothing collected those, so every edit grew the collection and searches
/// returned stale copies of code alongside the current one, indistinguishable
/// from it.
///
/// Takes the file's *complete* chunk set and runs before those chunks are
/// upserted, so a point that is about to be rewritten is never deleted — the
/// worst case is deleting a point and immediately restoring it.
///
/// A purge failure is reported but does not abort the run: a duplicate point is
/// a bad search result, while a half-finished ingest is a broken collection.
async fn purge_stale_for_document(
    backend: &Arc<dyn Backend + Send + Sync>,
    collection: &str,
    file_chunks: &[crate::types::Chunk],
) -> usize {
    let Some(document_id) = file_chunks.first().map(|c| c.document_id.clone()) else {
        return 0;
    };
    let keep: Vec<String> = file_chunks.iter().map(|c| c.id.clone()).collect();

    match backend
        .delete_stale_points(collection, &document_id, &keep)
        .await
    {
        Ok(n) => n,
        Err(e) => {
            if OUTPUT.is_interactive {
                eprintln!(
                    "warning: could not remove superseded chunks for document {document_id} \
                     in '{collection}': {e}. Stale copies may remain in search results."
                );
            }
            0
        }
    }
}

pub use discovery::{build_walker, count_files};
pub use options::IngestionOptions;
pub use pipeline::{flush_chunks, process_content, FlushParams, OversizeReport};
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

/// Resolve a write target: create it with a full genesis contract, or verify
/// that an existing one accepts vectors from this machine's embedder.
///
/// Every ingest path goes through here. The guard is asymmetric on purpose:
/// **a bad write contaminates a collection permanently and compounds with
/// every subsequent ingest, while a bad read produces one mediocre ranking and
/// evaporates.** So writes demand `Identical` unless the operator explicitly
/// opts into a quantization delta; reads (in `Core::search`) let `Compatible`
/// through with a note.
///
/// `target_dim` is the Matryoshka truncation request: the caller intends to
/// write vectors of that width rather than the model's native width. It is the
/// *effective* dimension and is therefore what gets compared and recorded — the
/// guard must never be handed the native dimension while truncated vectors go
/// to the backend behind it.
///
/// Returns the dimension to embed at.
pub async fn ensure_write_target(
    backend: &Arc<dyn Backend + Send + Sync>,
    embedder: &Arc<dyn Embedder + Send + Sync>,
    collection: &str,
    quantization: Option<crate::config::QuantizationType>,
    allow_quantization_delta: bool,
    target_dim: Option<usize>,
    // `chunking` is recorded into genesis only when THIS call creates the
    // collection. Ignored when it already exists: chunking describes how the
    // existing points were cut, and a later run at different parameters must
    // not rewrite that claim into looking like it was always so.
    chunking: Option<crate::types::ChunkingIdentity>,
) -> Result<usize> {
    let identity = embedder.identity().await?;
    let native_dim = embedder.dimension().await?;
    let dim = target_dim.unwrap_or(native_dim);

    // Truncation only ever narrows. Asking a 768-dim model for 1024 dimensions
    // is not a Matryoshka request, it is a bug in the caller, and padding to
    // satisfy it would put junk components into the space.
    if dim > native_dim {
        anyhow::bail!(
            "requested dimension {dim} exceeds what {} produces ({native_dim}-dim).\n\
             Matryoshka truncation can only narrow a vector, never widen one.",
            identity.describe(),
        );
    }

    if !backend.collection_exists(collection).await? {
        if OUTPUT.is_interactive {
            let note = if dim != native_dim {
                format!(" truncated from {native_dim}")
            } else {
                String::new()
            };
            eprintln!(
                "Creating collection '{}' ({}, {}-dim{})",
                collection,
                identity.describe(),
                dim,
                note
            );
        }
        backend
            .create_collection(collection, dim as u64, quantization)
            .await?;
        backend
            .write_genesis(
                collection,
                &crate::types::GenesisMetadata {
                    collection_id: uuid::Uuid::new_v4().to_string(),
                    model: identity,
                    dimension: dim as u64,
                    distance: "Cosine".to_string(),
                    chunking,
                    created_at: chrono::Utc::now().to_rfc3339(),
                },
            )
            .await?;
        return Ok(dim);
    }

    let genesis = backend.read_genesis(collection).await?;

    // Ownership before compatibility. "The models do not match" is a claim you
    // can only make about a collection whose model you know; for someone else's
    // collection the honest answer is simply that it is not ours. A Qdrant
    // instance is shared infrastructure, so this is a permanent condition, not
    // migration debt.
    if !genesis.is_vecdb() {
        anyhow::bail!(
            "'{collection}' is not a vecdb collection.\n\
             \n\
             It exists on this Qdrant but carries no vecdb marker, so it belongs \n\
             to another tool. vecdb will not write to it.\n\
             \n\
             fix: choose a different collection name, e.g. -c {collection}-vecdb"
        );
    }

    let report = crate::types::compare_spaces(
        &genesis.model,
        genesis.dimension,
        &identity,
        Some(dim as u64),
    );

    if !report.permits_write(allow_quantization_delta) {
        let hint = match report.tier {
            crate::types::Compatibility::Compatible => format!(
                "\n  {}\n\n  This is a quantization difference only. To accept it, re-run with \n  --allow-quantization-delta.",
                report.warning().unwrap_or_default()
            ),
            _ => report
                .suggestion
                .as_ref()
                .map(|s| format!("\n\n  fix: {s}"))
                .unwrap_or_default(),
        };
        anyhow::bail!(
            "embedding space mismatch for collection '{collection}'\n\
             \n\
             \x20 collection was created with:  {}  {}-dim\n\
             \x20 this machine resolves to:     {}  {}-dim\n\
             \n\
             \x20 {}{hint}",
            genesis.model.describe(),
            genesis.dimension.unwrap_or(0),
            identity.describe(),
            dim,
            report.reason,
        );
    }

    if let Some(w) = report.warning() {
        if OUTPUT.is_interactive {
            eprintln!("warning: {w}");
        }
    }

    Ok(genesis.dimension.map(|d| d as usize).unwrap_or(dim))
}

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

    // Creates with a full genesis contract, or refuses if this machine's
    // embedder would mix a second embedding space into an existing collection.
    //
    // Skipped for `--dry-run`, which answers "what would be ingested" and
    // writes nothing. Requiring a writable target to answer that is backwards:
    // it makes the one command that is safe to run against an unfamiliar
    // collection fail, and it fails in a way that says nothing about the file
    // selection the user actually asked about. It also creates the collection
    // as a side effect, which a dry run must never do.
    let resolved_dim = if options.dry_run {
        None
    } else {
        Some(
            ensure_write_target(
                backend,
                embedder,
                &options.collection,
                options.quantization.clone(),
                options.allow_quantization_delta,
                target_dim,
                Some(options.chunking_identity(&options.collection)),
            )
            .await?,
        )
    };

    // One tally for the whole run, shared with the embedding worker. Summarised
    // once at the end rather than shouted per chunk — a wall of per-chunk
    // warnings scrolls past, which is as good as silence.
    let oversize = Arc::new(OversizeReport::new());

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
    //
    // Skipped entirely for `--dry-run`. This block exists to keep the local
    // incremental-ingest state in step with the remote collection, and a dry
    // run neither reads nor writes chunks, so there is nothing to keep in step.
    // It also assumes the collection exists (`ensure_write_target` created it),
    // which is no longer true on the dry-run path, and it mutates
    // `.vecdb/state.toml` — a side effect a dry run must not have.
    let collection_name = options.collection.clone();

    if !options.dry_run {
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
    }

    let mut state_changed = false;

    // Check if this is an explicit single file (not a directory/glob)
    let path_is_file = std::path::Path::new(&options.path).is_file();

    if path_is_file {
        let path = std::path::PathBuf::from(&options.path);

        // Refuse to ingest files inside .vecdb state directories — same guard the walker applies.
        // Without this, ingesting state.toml creates .vecdb/.vecdb/state.toml next run, ad infinitum.
        if path.components().any(|c| c.as_os_str() == ".vecdb") {
            return Ok(());
        }

        let options_arc = Arc::new(options);
        let root_path = path
            .parent()
            .unwrap_or(std::path::Path::new("."))
            .to_path_buf();

        // Apply extensions filter (if set)
        if let Some(ref exts) = options_arc.extensions {
            let current_ext = path.extension().and_then(|s| s.to_str()).unwrap_or("");
            if !exts.iter().any(|e| e.eq_ignore_ascii_case(current_ext)) {
                eprintln!("Skipping: file extension not in allowlist");
                return Ok(());
            }
        }

        // Apply excludes filter (if set)
        if let Some(ref excludes) = options_arc.excludes {
            let path_str = path.to_string_lossy().to_string();
            for pattern in excludes {
                if let Ok(glob) = glob::Pattern::new(pattern) {
                    if glob.matches(&path_str)
                        || glob.matches(path.file_name().unwrap_or_default().to_str().unwrap_or(""))
                    {
                        eprintln!("Skipping: file matches exclude pattern '{}'", pattern);
                        return Ok(());
                    }
                }
            }
        }

        // Compute metadata hash and check state
        let rel_path = path.strip_prefix(&root_path).unwrap_or(&path).to_path_buf();
        let file_collection: String = if let Some(ref routes) = options_arc.vecdbrc_routes {
            let match_path = options_arc
                .vecdbrc_root
                .as_ref()
                .and_then(|root| path.strip_prefix(root).ok())
                .map(|p| p.to_string_lossy().to_string())
                .unwrap_or_else(|| rel_path.to_string_lossy().to_string());

            let coll =
                crate::vecdbrc::resolve_route(routes, &match_path, Some(&options_arc.collection)).0;
            if coll.is_empty() {
                options_arc.collection.clone()
            } else {
                coll
            }
        } else {
            options_arc.collection.clone()
        };

        if let Ok(meta_hash) = crate::state::compute_file_metadata_hash(&path) {
            if !state.update_file(&file_collection, rel_path.clone(), meta_hash.clone()) {
                eprintln!("Skipping: file unchanged (state match)");
                return Ok(());
            }
            state_changed = true;
        }

        // Process the file
        let compiled_rules: Vec<Regex> = options_arc
            .path_rules
            .iter()
            .filter_map(|rule| Regex::new(&rule.pattern).ok())
            .collect();

        let mut files_processed = 0;
        let mut files_skipped = 0;

        match process_single_file(
            path.clone(),
            rel_path.clone(),
            crate::ingestion::processor::FileProcessor {
                detector: detector.clone(),
                parser_factory: parser_factory.clone(),
                rules: compiled_rules,
            },
            options_arc.clone(),
            commit_sha.clone(),
            file_collection.clone(),
        )
        .await
        {
            Ok(Some(mut chunks)) => {
                // Guard and size the destination this file actually routed to.
                //
                // `resolved_dim` describes the primary target, which is only the
                // right answer when routing is inactive. A routed file was
                // previously embedded at the primary's dimension and written
                // without any space check — the directory path guards routed
                // destinations, so the single-file path must too.
                let file_dim = if file_collection == options_arc.collection {
                    resolved_dim
                } else {
                    Some(
                        ensure_write_target(
                            backend,
                            embedder,
                            &file_collection,
                            options_arc.quantization.clone(),
                            options_arc.allow_quantization_delta,
                            target_dim,
                            Some(options_arc.chunking_identity(&file_collection)),
                        )
                        .await?,
                    )
                };

                // Before the upsert, so a chunk that is about to be rewritten is
                // never a deletion candidate.
                let stale_removed =
                    purge_stale_for_document(backend, &file_collection, &chunks).await;

                // Embed and flush
                flush_chunks(
                    backend,
                    embedder,
                    &file_collection,
                    &mut chunks,
                    &FlushParams {
                        gpu_batch_size: options_arc.gpu_batch_size,
                        target_dim: file_dim,
                        max_chunk_bytes: Some(options_arc.chunking_for(&file_collection).ceiling()),
                        on_oversize: options_arc.on_oversize,
                    },
                    &oversize,
                )
                .await?;
                files_processed = 1;
                if stale_removed > 0 {
                    eprintln!("Removed {stale_removed} superseded chunk(s) for this file.");
                }
            }
            Ok(None) => {
                eprintln!("Skipping: file not processable");
                files_skipped = 1;
            }
            Err(e) => {
                eprintln!("File processing error: {}", e);
            }
        }

        // Guarded for the same reason the ID-resolution block above is: a dry
        // run must not mutate `.vecdb/state.toml`. Writing it here made the
        // NEXT real ingest treat every file as already-ingested and skip it —
        // a dry run silently cancelling the run it was previewing.
        if state_changed && !options_arc.dry_run {
            state.touch_collection(&file_collection);
            let _ = state.save(&root_path);
        }

        eprintln!(
            "Ingestion Summary: Scanned {}, Processed {}, Skipped {}",
            1, files_processed, files_skipped
        );
        if let Some(summary) = oversize.summary(options_arc.on_oversize) {
            eprintln!("warning: {summary}");
        }
        return Ok(());
    }

    // Directory or glob - use walker with filters
    //
    // The fallback is announced, not silent. It is the only path by which a walk
    // honours a file the operator never pointed at, so the run has to say so.
    let gitignore = discovery::resolve_gitignore(&options);
    if gitignore.via_fallback && OUTPUT.is_interactive {
        eprintln!(
            "note: no .vectorignore found (checked {}/.vectorignore and ~/.vectorignore) \
             — falling back to .gitignore for this walk.\n\
             \x20     .gitignore is a build-artifact list, not an indexing policy. Add a \
             .vectorignore to say what should actually be indexed.",
            options.path
        );
    }
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

    // Points dropped because the file that wrote them has since changed.
    // Reported at the end: a re-ingest that silently rewrites half a collection
    // should say so.
    let mut stale_removed: usize = 0;

    let mut chunks_buffer: Vec<crate::types::Chunk> = Vec::new();
    // Which collection the buffered chunks belong to.
    //
    // The buffer accumulates across files, and with `.vecdbrc` routing active
    // consecutive files may target different collections. Without tracking the
    // owner, a flush tags the batch with whatever file happened to arrive next —
    // and `try_join_next` returns tasks in completion order, so which collection
    // a chunk landed in was decided by a race. Files were silently written to the
    // wrong collection roughly half the time.
    //
    // With no routes every file resolves to the same name, so this is simply
    // `Some(collection_name)` throughout and the routed and unrouted paths need
    // no branching between them.
    let mut buffered_coll: Option<String> = None;
    let batch_size = 20;

    let mut files_scanned = 0;
    let mut files_skipped = 0;
    let mut files_processed = 0;

    let options_arc = Arc::new(options);

    let semaphore = Arc::new(tokio::sync::Semaphore::new(
        options_arc.max_concurrent_requests,
    ));
    let mut tasks = tokio::task::JoinSet::new();

    // Pipeline Channel: Decouples parsing from embedding
    // When routing is active, each message carries its target collection.
    // Without routing, all messages use the options.collection default.
    let (tx, mut rx) = tokio::sync::mpsc::channel::<(String, Vec<crate::types::Chunk>)>(10);

    // Dedicated Embedding Worker (routing-aware)
    let backend_embed = backend.clone();
    let embedder_embed = embedder.clone();
    let gpu_batch_size = options_arc.gpu_batch_size;
    // Resolved per destination inside the worker, like the dimension — see
    // `route_chunking`. Chunking is a property of where the chunk is going.
    let options_for_worker = options_arc.clone();
    let allow_quantization_delta = options_arc.allow_quantization_delta;
    let quantization = options_arc.quantization.clone();
    let on_oversize = options_arc.on_oversize;
    let oversize_worker = oversize.clone();
    let embedding_handle = tokio::spawn(async move {
        // Dimension per destination, not per run.
        //
        // `ensure_write_target` returns the dimension the *named* collection is
        // actually built at. Routed ingest fans across collections that need not
        // share one: same model truncated to different Matryoshka widths passes
        // the space guard by design. Embedding every route at the primary
        // target's dimension therefore produced vectors of the wrong width for
        // every destination but the first.
        //
        // Cached because the guard reads the genesis point on every call, and the
        // worker is invoked once per batch, not once per collection.
        let mut route_dims: std::collections::HashMap<String, usize> =
            std::collections::HashMap::new();

        while let Some((coll, mut batch)) = rx.recv().await {
            // Routed destinations get the same guard as the primary target —
            // a .vecdbrc route is not a reason to skip the space check.
            let route_dim = match route_dims.get(&coll) {
                Some(dim) => *dim,
                None => {
                    let dim = ensure_write_target(
                        &backend_embed,
                        &embedder_embed,
                        &coll,
                        quantization.clone(),
                        allow_quantization_delta,
                        target_dim,
                        Some(options_for_worker.chunking_identity(&coll)),
                    )
                    .await?;
                    route_dims.insert(coll.clone(), dim);
                    dim
                }
            };
            flush_chunks(
                &backend_embed,
                &embedder_embed,
                &coll,
                &mut batch,
                &FlushParams {
                    gpu_batch_size,
                    target_dim: Some(route_dim),
                    max_chunk_bytes: Some(options_for_worker.chunking_for(&coll).ceiling()),
                    on_oversize,
                },
                &oversize_worker,
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

                    // File allowlist: if set, only process files that match
                    // one of the explicitly listed paths. Used for multi-file
                    // glob ingestion where the walker walks the common parent
                    // but we only want specific files.
                    if let Some(ref allowlist) = options_arc.file_allowlist {
                        let path_str = path.to_string_lossy().to_string();
                        if !allowlist.iter().any(|allowed| {
                            path_str == *allowed
                                || path_str
                                    .ends_with(&format!("/{}", allowed.trim_start_matches("./")))
                                || path_str
                                    .ends_with(&format!("\\{}", allowed.trim_start_matches("./")))
                        }) {
                            files_skipped += 1;
                            continue;
                        }
                    }

                    let stripped = path.strip_prefix(root_path).unwrap_or(&path);
                    let canonical_root = std::fs::canonicalize(root_path)
                        .unwrap_or_else(|_| root_path.to_path_buf());
                    let project_dir_name = canonical_root
                        .file_name()
                        .unwrap_or_else(|| std::ffi::OsStr::new(""));
                    let rel_path = if project_dir_name.is_empty() {
                        stripped.to_path_buf()
                    } else {
                        std::path::Path::new(project_dir_name).join(stripped)
                    };

                    // Determine target collection via .vecdbrc routing (if active)
                    let file_collection: String =
                        if let Some(ref routes) = options_arc.vecdbrc_routes {
                            let match_path = options_arc
                                .vecdbrc_root
                                .as_ref()
                                .and_then(|root| path.strip_prefix(root).ok())
                                .map(|p| p.to_string_lossy().to_string())
                                .unwrap_or_else(|| rel_path.to_string_lossy().to_string());

                            let coll = crate::vecdbrc::resolve_route(
                                routes,
                                &match_path,
                                Some(&options_arc.collection),
                            )
                            .0;
                            // Fall back to options collection if route returned empty
                            let effective_coll = if coll.is_empty() {
                                options_arc.collection.clone()
                            } else {
                                coll
                            };
                            if OUTPUT.is_interactive {
                                eprintln!(
                                    "  → '{}' → collection '{}'",
                                    rel_path.display(),
                                    effective_coll
                                );
                            }
                            effective_coll
                        } else {
                            options_arc.collection.clone()
                        };

                    if let Ok(meta_hash) = crate::state::compute_file_metadata_hash(&path) {
                        if !state.update_file(&file_collection, rel_path.clone(), meta_hash.clone())
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
                    let coll_for_task = file_collection.clone();

                    let detector = detector.clone();
                    let parser_factory = parser_factory.clone();
                    let rules = compiled_rules.clone();
                    let options_ref = options_arc.clone();
                    let commit_sha = commit_sha.clone();

                    tasks.spawn(async move {
                        let _permit = permit;
                        match process_single_file(
                            path,
                            rel_path,
                            crate::ingestion::processor::FileProcessor {
                                detector,
                                parser_factory,
                                rules,
                            },
                            options_ref,
                            commit_sha,
                            coll_for_task.clone(),
                        )
                        .await
                        {
                            Ok(Some(chunks)) => Ok(Some((coll_for_task, chunks))),
                            Ok(None) => Ok(None),
                            Err(e) => Err(e),
                        }
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
                Ok(Ok(Some((coll, mut file_chunks)))) => {
                    files_processed += 1;
                    stale_removed += purge_stale_for_document(backend, &coll, &file_chunks).await;

                    // A batch belongs to the collection whose chunks are in it,
                    // not to the file that arrived next. Flush under the owner
                    // before switching.
                    if buffered_coll.as_deref().is_some_and(|b| b != coll) {
                        let owner = buffered_coll
                            .take()
                            .unwrap_or_else(|| collection_name.clone());
                        let batch = std::mem::take(&mut chunks_buffer);
                        if (tx.send((owner, batch)).await).is_err() {
                            break 'discovery_loop;
                        }
                    }
                    buffered_coll = Some(coll.clone());
                    chunks_buffer.append(&mut file_chunks);

                    if chunks_buffer.len() >= batch_size {
                        let owner = buffered_coll
                            .clone()
                            .unwrap_or_else(|| collection_name.clone());
                        let batch = std::mem::take(&mut chunks_buffer);
                        if (tx.send((owner, batch)).await).is_err() {
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
            Ok(Ok(Some((coll, mut file_chunks)))) => {
                files_processed += 1;
                stale_removed += purge_stale_for_document(backend, &coll, &file_chunks).await;

                // Same ownership rule as the discovery-loop drain above.
                if buffered_coll.as_deref().is_some_and(|b| b != coll) {
                    let owner = buffered_coll
                        .take()
                        .unwrap_or_else(|| collection_name.clone());
                    let batch = std::mem::take(&mut chunks_buffer);
                    if (tx.send((owner, batch)).await).is_err() {
                        break 'parsing_finish;
                    }
                }
                buffered_coll = Some(coll.clone());
                chunks_buffer.append(&mut file_chunks);

                if chunks_buffer.len() >= batch_size {
                    let owner = buffered_coll
                        .clone()
                        .unwrap_or_else(|| collection_name.clone());
                    let batch = std::mem::take(&mut chunks_buffer);
                    if (tx.send((owner, batch)).await).is_err() {
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
        // The final batch belongs to the last collection written to, not to the
        // CLI fallback. This unconditionally used `collection_name`, so with
        // routing active the tail of every ingest landed in the wrong place.
        let owner = buffered_coll
            .clone()
            .unwrap_or_else(|| collection_name.clone());
        let _ = tx.send((owner, chunks_buffer)).await;
    }

    // Signal completion to embedding worker
    drop(tx);
    embedding_handle
        .await
        .map_err(|e| anyhow::anyhow!("Embedding background task panicked: {}", e))??;

    if let Some(ref j_id) = job_id {
        if let Some(ref r) = job_registry {
            let _ = r.complete(j_id);
        }
    }

    // See the single-file path: a dry run previews, it does not record.
    if state_changed && !options_arc.dry_run {
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
    if stale_removed > 0 {
        eprintln!("Removed {stale_removed} superseded chunk(s) from edited files.");
    }

    // Reported after the counts, so "Processed 40" is never the last word when
    // some of those 40 lost content to the ceiling.
    if let Some(summary) = oversize.summary(options_arc.on_oversize) {
        eprintln!("warning: {summary}");
    }

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
    target_chunk_size: Option<usize>,
    max_chunk_bytes: Option<usize>,
    chunk_overlap: Option<usize>,
    quantization: Option<crate::config::QuantizationType>,
    target_dim: Option<usize>,
) -> Result<()> {
    let options = IngestionOptions {
        path: "memory".to_string(),
        collection: collection.to_string(),
        vecdbrc_routes: None,
        vecdbrc_root: None,
        target_chunk_size: target_chunk_size.unwrap_or(512),
        max_chunk_bytes,
        on_oversize: Default::default(),
        route_chunking: Default::default(),
        chunk_overlap: chunk_overlap.unwrap_or(50),
        respect_gitignore: false,
        ignore_vectorignore: false,
        strategy: "recursive".to_string(),
        tokenizer: "cl100k_base".to_string(),
        git_ref: None,
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

    let mut chunks = process_content(
        content,
        &options,
        Path::new("memory"),
        &metadata,
        FileType::Text,
        collection,
    )
    .await?;

    let resolved_dim = Some(
        ensure_write_target(
            backend,
            embedder,
            collection,
            options.quantization.clone(),
            options.allow_quantization_delta,
            target_dim,
            Some(options.chunking_identity(collection)),
        )
        .await?,
    );

    let oversize = OversizeReport::new();
    flush_chunks(
        backend,
        embedder,
        collection,
        &mut chunks,
        &FlushParams {
            gpu_batch_size: options.gpu_batch_size,
            target_dim: resolved_dim,
            max_chunk_bytes,
            on_oversize: options.on_oversize,
        },
        &oversize,
    )
    .await?;

    if let Some(summary) = oversize.summary(options.on_oversize) {
        eprintln!("warning: {summary}");
    }

    Ok(())
}
