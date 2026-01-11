/*
 * PURPOSE:
 *   Handles directory walking, file reading, chunking, and ingestion orchestration.
 *   Respects .ignore/.gitignore files via `ignore` crate.
 */

use crate::backend::Backend;
use crate::embedder::Embedder;
use crate::types::Chunk;
use crate::state::IngestionState;
use crate::output::OUTPUT;
use anyhow::Result;
use crate::chunking::Chunker;
use ignore::WalkBuilder;
use std::fs;
use std::path::Path;
use std::sync::Arc;
use uuid::Uuid;
use vecdb_common::{FileType, FileTypeDetector};
use crate::parsers::ParserFactory;

const VECDB_NAMESPACE: Uuid = Uuid::from_u128(0xa1a2a3a4_b1b2_c1c2_d1d2_e1e2e3e4e5e6);

pub struct IngestionOptions {
    pub path: String,
    pub collection: String,
    pub chunk_size: usize,
    pub max_chunk_size: Option<usize>,
    pub chunk_overlap: usize,
    pub respect_gitignore: bool,
    pub strategy: String,
    pub tokenizer: String,
    pub git_ref: Option<String>,
    // Stank Hunt: Globbing Support
    pub extensions: Option<Vec<String>>, // e.g. ["rs", "md"]
    pub excludes: Option<Vec<String>>,   // e.g. ["*.tmp", "target/"]
    pub dry_run: bool,                   // If true, list files but do not chunk/embed
    pub metadata: Option<std::collections::HashMap<String, serde_json::Value>>, // Global metadata for all files
}

/// Orchestrate ingestion of a path
#[allow(clippy::too_many_arguments)]
pub async fn ingest_path(
    backend: &Arc<dyn Backend + Send + Sync>,
    embedder: &Arc<dyn Embedder + Send + Sync>,
    detector: &Arc<dyn FileTypeDetector>,
    parser_factory: &Arc<dyn ParserFactory>,
    options: IngestionOptions,
) -> Result<()> {
    if OUTPUT.is_interactive {
        eprintln!("Ingesting path: {}", options.path);
    }

    // 0. Ensure collection exists
    if !backend.collection_exists(&options.collection).await? {
        if OUTPUT.is_interactive {
            eprintln!("Collection {} does not exist. Creating...", options.collection);
        }
        let dim = embedder.dimension().await?;
        backend.create_collection(&options.collection, dim as u64).await?;
    }

    // Check for git SHA at the root of the ingestion path
    let commit_sha = crate::git::get_head_sha(Path::new(&options.path)).unwrap_or(None);
    if let Some(ref sha) = commit_sha {
        if OUTPUT.is_interactive {
            eprintln!("Detected Git Repo. Injecting commit_sha: {}", sha);
        }
    }

    // Load State
    let root_path = Path::new(&options.path);
    let mut state = match IngestionState::load(root_path) {
        Ok(s) => s,
        Err(e) => {
            if OUTPUT.is_interactive {
                eprintln!("Warning: Failed to load ingestion state: {}. Starting fresh.", e);
            }
            IngestionState::default()
        }
    };
    let mut state_changed = false;

    // 1. Build walker respecting .ignore, .gitignore, etc.
    // UPDATE: User requested NO .gitignore support by default. Only .vectorignore.
    // UNLESS: explicitly requested via options.
    // 1. Build walker respecting .ignore, .vectorignore always, and .gitignore optionally
    let mut builder = WalkBuilder::new(&options.path);
    builder
        .standard_filters(false) // Disable standard filters to control them manually
        .git_ignore(options.respect_gitignore) // Optional .gitignore
        .ignore(true)            // Always respect .ignore (and custom ignore files)
        .parents(true)           // Look in parent directories for ignore files
        .hidden(false)           // Allow hidden files
        .add_custom_ignore_filename(".vectorignore"); // Prioritize .vectorignore

    let walker = builder.build();

    let mut chunks_buffer = Vec::new();
    let batch_size = 20;
    
    // Stats
    let mut files_scanned = 0;
    let mut files_skipped = 0;
    let mut files_processed = 0;

    for result in walker {
        match result {
            Ok(entry) => {
                if entry.file_type().is_some_and(|ft| ft.is_file()) {
                    files_scanned += 1;
                    let path = entry.path();
                    
                    // Ignore internal .vecdb directory
                    if path.components().any(|c| c.as_os_str() == ".vecdb") {
                        continue;
                    }
                    
                    // Smart Detection
                    let content_preview = match fs::read(path) {
                        Ok(bytes) => bytes,
                        Err(e) => {
                            if OUTPUT.is_interactive {
                                eprintln!("Failed to read {}: {}", path.display(), e);
                            }
                            continue;
                        }
                    };

                    let file_type = detector.detect(path, &content_preview);
                    
                    // Skip if explicit binary or unknown (and we want to be strict)
                    // Note: FileType::Text includes generic text files.
                    if !file_type.is_supported() {
                        // Fallback check: is it really binary?
                        if is_binary(&content_preview) {
                             files_skipped += 1;
                             continue;
                        }
                        // If textual but unknown type, treat as generic text
                    }

                    let parser = parser_factory.get_parser(file_type);

                    if parser.is_some() || file_type != FileType::Unknown || !is_binary(&content_preview) {
                        // FILTER: Extensions
                        if let Some(ref exts) = options.extensions {
                            let current_ext = path.extension().and_then(|s| s.to_str()).unwrap_or("");
                            if !exts.iter().any(|e| e.eq_ignore_ascii_case(current_ext)) {
                                continue; // Skip if extension not in whitelist
                            }
                        }
                        
                        // FILTER: Manual Excludes (Globbing)
                        if let Some(ref excludes) = options.excludes {
                            // Simple glob check
                            let path_str = path.to_string_lossy();
                            let mut excluded = false;
                            for pattern in excludes {
                                if let Ok(glob) = glob::Pattern::new(pattern) {
                                    if glob.matches(&path_str) || glob.matches(path.file_name().unwrap_or_default().to_str().unwrap_or("")) {
                                        excluded = true;
                                        break;
                                    }
                                }
                            }
                            if excluded {
                                continue;
                            }
                        }

                        // DRY RUN
                        if options.dry_run {
                            if OUTPUT.is_interactive {
                                if let Some(meta) = &options.metadata {
                                    println!("[Dry Run] Would ingest: {} (Metadata: {:?})", path.display(), meta);
                                } else {
                                    println!("[Dry Run] Would ingest: {}", path.display());
                                }
                            } else {
                                // Machine readable-ish
                                if let Some(meta) = &options.metadata {
                                    println!("{} : {:?}", path.display(), meta);
                                } else {
                                    println!("{}", path.display());
                                }
                            }
                            continue;
                        }

                        // OPTIMIZATION: Check metadata hash BEFORE reading content
                        // This is 100-1000x faster for unchanged files (no I/O required)
                        let rel_path = path.strip_prefix(root_path).unwrap_or(path).to_path_buf();
                        
                        if let Ok(meta_hash) = crate::state::compute_file_metadata_hash(path) {
                            if !state.update_file(rel_path.clone(), meta_hash.clone()) {
                                files_skipped += 1;
                                continue; // File unchanged, skip entirely
                            }
                            state_changed = true;
                        }
                        
                        // File changed or new: process content
                        // We already read content_preview, but that might be partial?
                        // Actually let's assume content_preview was full read if file small, 
                        // but detecting on full file is expensive if large.
                        // Detector usually takes &[u8].
                        // Re-reading as string for processing is safer for encoding handling.
                        let content = String::from_utf8_lossy(&content_preview).to_string();
                        
                        files_processed += 1;

                        let mut metadata = std::collections::HashMap::new();
                        metadata.insert("path".to_string(), serde_json::Value::String(rel_path.display().to_string()));
                        metadata.insert("source_type".to_string(), serde_json::Value::String("file".to_string()));
                        metadata.insert("full_path".to_string(), serde_json::Value::String(path.display().to_string()));
                        metadata.insert("language".to_string(), serde_json::Value::String(file_type.to_string().to_lowercase())); // Enriched Metadata
                        if let Some(ref sha) = commit_sha {
                            metadata.insert("commit_sha".to_string(), serde_json::Value::String(sha.clone()));
                        }
                        if let Some(ref git_ref) = options.git_ref {
                            metadata.insert("git_ref".to_string(), serde_json::Value::String(git_ref.clone()));
                        }

                        // Stank Hunt: Support global metadata via CLI
                        if let Some(ref global_meta) = options.metadata {
                            for (k, v) in global_meta {
                                metadata.insert(k.clone(), v.clone());
                            }
                        }

                        let chunks_result = if let Some(p) = parser {
                            let meta_val = serde_json::Value::Object(metadata.clone().into_iter().collect());
                            p.parse(&content, path, Some(meta_val)).await
                        } else {
                            process_content(&content, &options, path, &metadata, file_type).await
                        };

                        match chunks_result {
                            Ok(chunks) => {
                                for chunk in chunks {
                                    chunks_buffer.push(chunk);
                                    
                                    if chunks_buffer.len() >= batch_size {
                                        flush_chunks(backend, embedder, &options.collection, &mut chunks_buffer).await?;
                                    }
                                }
                            }
                            Err(e) => {
                                if OUTPUT.is_interactive {
                                    eprintln!("Failed to process {}: {}", path.display(), e);
                                }
                            }
                        }
                    }
                }
            }
            Err(err) => {
                if OUTPUT.is_interactive {
                    eprintln!("Error walking directory: {}", err);
                }
            }
        }
    }

    // Flush remaining
    if !chunks_buffer.is_empty() {
        flush_chunks(backend, embedder, &options.collection, &mut chunks_buffer).await?;
    }

    // Save State
    if state_changed {
        if let Err(e) = state.save(root_path) {
            if OUTPUT.is_interactive {
                eprintln!("Warning: Failed to save ingestion state: {}", e);
            }
        } else if OUTPUT.is_interactive {
            eprintln!("Updated ingestion state.");
        }
    }
    
    // Ingestion summary should always be printed to stderr for automation/logging
    eprintln!("Ingestion Summary: Scanned {}, Processed {}, Skipped {}", files_scanned, files_processed, files_skipped);

    Ok(())
}

async fn flush_chunks(
    backend: &Arc<dyn Backend + Send + Sync>,
    embedder: &Arc<dyn Embedder + Send + Sync>,
    collection: &str,
    chunks: &mut Vec<Chunk>,
) -> Result<()> {
    if chunks.is_empty() { return Ok(()); }

    // 1. Check which chunks already exist in the backend
    let ids: Vec<String> = chunks.iter().map(|c| c.id.clone()).collect();
    let existing_ids = backend.points_exists(collection, ids).await?;
    
    // 2. Separate chunks that need embeddings
    let mut new_chunks: Vec<Chunk> = Vec::new();
    for chunk in chunks.drain(..) {
        if !existing_ids.contains(&chunk.id) {
            new_chunks.push(chunk);
        }
    }

    if !new_chunks.is_empty() {
        if OUTPUT.is_interactive {
            eprintln!("Embedding {} new chunks...", new_chunks.len());
        }
        
        // Safety: Cap chunk size to avoid exceeding embedding model context
        // Qwen3-Embedding/nomic-embed-text: ~8192 tokens
        // At ~3-4 chars/token for code, 6K chars is ~1500-2000 tokens (safe margin)
        const MAX_CHUNK_CHARS: usize = 6000;
        
        // 2a. Pre-process chunks: Split oversized ones ("Dumb Chunking" fallback)
        let mut final_chunks: Vec<Chunk> = Vec::with_capacity(new_chunks.len());
        let fallback_chunker = crate::chunking::SimpleChunker;
        let fallback_params = crate::chunking::ChunkParams {
            chunk_size: MAX_CHUNK_CHARS,
            max_chunk_size: Some(MAX_CHUNK_CHARS),
            chunk_overlap: 0,
            tokenizer: "char".to_string(),
            file_extension: None,
        };
        
        for chunk in new_chunks {
            if chunk.content.len() > MAX_CHUNK_CHARS {
                if OUTPUT.is_interactive {
                    eprintln!("Warning: Oversized chunk detected ({} chars). Splitting into smaller parts...", chunk.content.len());
                }
                
                // Use SimpleChunker to split efficiently and maintain structural integrity (newlines)
                // Note: chunk() is async, but SimpleChunker is computationally bound here.
                let sub_chunks = fallback_chunker.chunk(&chunk.content, &fallback_params).await?;
                
                for (idx, sub) in sub_chunks.into_iter().enumerate() {
                    let mut part_chunk = chunk.clone();
                    part_chunk.content = sub.content;
                    
                    // Generate stable UUID for part based on original ID + index
                    let seed = format!("{}-part-{}", chunk.id, idx);
                    part_chunk.id = uuid::Uuid::new_v5(&VECDB_NAMESPACE, seed.as_bytes()).to_string();
                    
                    // Update metadata to track provenance
                    part_chunk.metadata.insert("split_part".to_string(), serde_json::json!(idx));
                    part_chunk.metadata.insert("original_chunk_id".to_string(), serde_json::Value::String(chunk.id.clone()));
                    
                    // Approximate line numbers if available
                    if let (Some(base_start), Some(_base_end)) = (chunk.start_line, chunk.end_line) {
                         if let (Some(sub_start), Some(sub_end)) = (sub.line_start, sub.line_end) {
                             part_chunk.start_line = Some(base_start + sub_start - 1);
                             part_chunk.end_line = Some(base_start + sub_end - 1);
                         }
                    }
                    
                    final_chunks.push(part_chunk);
                }
            } else {
                final_chunks.push(chunk);
            }
        }

        let texts: Vec<String> = final_chunks.iter().map(|c| c.content.clone()).collect();
        let vectors = embedder.embed_batch(&texts).await?;

        for (i, chunk) in final_chunks.iter_mut().enumerate() {
            if i < vectors.len() {
                chunk.vector = Some(vectors[i].clone());
                chunk.metadata.insert("_model_name".to_string(), serde_json::Value::String(embedder.model_name()));
            }
        }
        
        // 3. Upsert only the new (potentially split) chunks
        backend.upsert(collection, final_chunks).await?;
    } else if OUTPUT.is_interactive {
        eprintln!("All chunks already exist. Skipping embedding.");
    }

    // chunks is already cleared by drain
    Ok(())
}

fn is_binary(content: &[u8]) -> bool {
    // Check for null bytes in the first 8KB
    let len = std::cmp::min(content.len(), 8192);
    content[0..len].contains(&0)
}

/// Ingest raw content from memory (e.g., from SearXNG or Agents)
#[allow(clippy::too_many_arguments)]
pub async fn ingest_memory(
    backend: &Arc<dyn Backend + Send + Sync>,
    embedder: &Arc<dyn Embedder + Send + Sync>,
    content: &str,
    metadata: std::collections::HashMap<String, serde_json::Value>,
    collection: &str,
    chunk_size: Option<usize>,
    max_chunk_size: Option<usize>,
    chunk_overlap: Option<usize>,
) -> Result<()> {
    // For memory ingestion, use defaults
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
    };
    // Generate chunks from content
    let mut chunks = process_content(content, &options, Path::new("memory"), &metadata, FileType::Text).await?;
    
    // 0. Ensure collection exists
    if !backend.collection_exists(collection).await? {
        eprintln!("Collection {} does not exist. Creating...", collection);
        let dim = embedder.dimension().await?;
        backend.create_collection(collection, dim as u64).await?;
    }

    // Flush to backend
    flush_chunks(backend, embedder, collection, &mut chunks).await?;
    
    Ok(())
}

async fn process_content(
    content: &str, 
    options: &IngestionOptions,
    path: &Path,
    base_metadata: &std::collections::HashMap<String, serde_json::Value>,
    file_type: FileType,
) -> Result<Vec<Chunk>> {
    let doc_id = Uuid::new_v4().to_string();
    let commit_sha = base_metadata.get("commit_sha").and_then(|v| v.as_str()).unwrap_or("HEAD");
    

    // Use Chunker Factory
    // FIX: For "Unknown" file types (e.g. Lua, Code without parser), RecursiveChunker (text-splitter)
    // can be extremely slow (30s+ for 5MB). Default to SimpleChunker (lines) for these cases 
    // to treat them as "Code/Text" lines rather than "Prose/Sentences".
    let chunker = if matches!(file_type, FileType::Unknown) && options.strategy == "recursive" {
         Box::new(crate::chunking::SimpleChunker)
    } else {
         crate::chunking::Factory::get(&options.strategy)
    };

    let ext = path.extension().and_then(|s| s.to_str()).map(|s| s.to_string());
    
    let params = crate::chunking::ChunkParams {
        chunk_size: options.chunk_size,
        max_chunk_size: options.max_chunk_size,
        chunk_overlap: options.chunk_overlap,
        tokenizer: options.tokenizer.clone(),
        file_extension: ext,
    };
    
    let text_chunks = chunker.chunk(content, &params).await?;
    
    let mut chunks = Vec::new();
    let mut char_count = 0;
    
    for (idx, text_chunk) in text_chunks.iter().enumerate() {
        let chunk_len = text_chunk.content.len();
        
        // Metadata
        let mut metadata = base_metadata.clone();
        metadata.insert("chunk_index".to_string(), serde_json::json!(idx));
        
        // ID Generation: Path + Commit + Content
        let composite_key = format!("{}::{}::{}", path.display(), commit_sha, text_chunk.content);
        let chunk_id = Uuid::new_v5(&VECDB_NAMESPACE, composite_key.as_bytes()).to_string();

        chunks.push(Chunk {
            id: chunk_id,
            document_id: doc_id.clone(),
            content: text_chunk.content.clone(),
            vector: None,
            metadata,
            page_num: None,
            char_start: char_count,
            char_end: char_count + chunk_len,
            start_line: text_chunk.line_start, // RESTORED
            end_line: text_chunk.line_end,     // RESTORED
        });
        
        char_count += chunk_len;
    }
    
    Ok(chunks)
}
