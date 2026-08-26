use crate::ingestion::pipeline::process_content;
use crate::ingestion::IngestionOptions;
use crate::output::OUTPUT;
use crate::parsers::ParserFactory;
use crate::types::Chunk;
use anyhow::Result;
use regex::Regex;
use std::path::PathBuf;
use std::sync::Arc;
use tokio::io::AsyncReadExt;
use tracing::debug;
use vecdb_common::{FileType, FileTypeDetector};

/// Per-run machinery, identical for every file in an ingest.
///
/// Grouped so the per-file arguments stay legible. Cloned per spawned task; all
/// three fields are cheap to clone (two `Arc`s and a compiled rule list).
#[derive(Clone)]
pub struct FileProcessor {
    pub detector: Arc<dyn FileTypeDetector>,
    pub parser_factory: Arc<dyn ParserFactory>,
    pub rules: Vec<Regex>,
}

pub async fn process_single_file(
    path: PathBuf,
    rel_path: PathBuf,
    processor: FileProcessor,
    options: Arc<IngestionOptions>,
    commit_sha: Option<String>,
    // Destination collection, so chunking uses the parameters configured for
    // where the file is actually going.
    collection: String,
) -> Result<Option<Vec<Chunk>>> {
    let FileProcessor {
        detector,
        parser_factory,
        rules,
    } = processor;
    let metadata_fs = tokio::fs::metadata(&path)
        .await
        .map_err(|e| anyhow::anyhow!("Failed to stat {}: {}", path.display(), e))?;

    let file_size = metadata_fs.len();
    const LARGE_FILE_THRESHOLD: u64 = 50 * 1024 * 1024; // 50 MB
    let is_large = file_size > LARGE_FILE_THRESHOLD;

    let mut file = tokio::fs::File::open(&path).await?;
    let mut header_buffer = vec![0u8; 8192];
    let n = file.read(&mut header_buffer).await?;
    let content_preview = &header_buffer[..n];

    let file_type = detector.detect(&path, content_preview);

    // Early reject on the 8 KiB header, before the file is read in full — the
    // point of this pass is to avoid pulling a large binary into memory. Same
    // rule as the full-content check below: applied whatever the detected type.
    if FileType::is_binary_content(content_preview) {
        if OUTPUT.is_interactive {
            eprintln!("Skipping binary file: {}", path.display());
        }
        return Ok(None);
    }

    if let Some(ref exts) = options.extensions {
        let current_ext = path.extension().and_then(|s| s.to_str()).unwrap_or("");
        if !exts.iter().any(|e| e.eq_ignore_ascii_case(current_ext)) {
            return Ok(None);
        } // Skipped
    }

    if let Some(ref excludes) = options.excludes {
        let path_str = path.to_string_lossy();
        for pattern in excludes {
            if let Ok(glob) = glob::Pattern::new(pattern) {
                if glob.matches(&path_str)
                    || glob.matches(path.file_name().unwrap_or_default().to_str().unwrap_or(""))
                {
                    return Ok(None); // Skipped
                }
            }
        }
    }

    let mut metadata = std::collections::HashMap::new();
    metadata.insert(
        "path".to_string(),
        serde_json::Value::String(rel_path.display().to_string()),
    );
    metadata.insert(
        "source_type".to_string(),
        serde_json::Value::String("file".to_string()),
    );
    metadata.insert(
        "full_path".to_string(),
        serde_json::Value::String(path.display().to_string()),
    );
    // `source` was previously written only by the AST parsers, so chunks that
    // took the plain-text path carried `path` and `full_path` but no `source`.
    // Anything reading or filtering on `source` therefore saw a subset of the
    // corpus with nothing indicating a subset — the same failure shape as a
    // silently truncated result list. Set it here so every chunk has it,
    // matching the parsers' meaning (the full path).
    metadata.insert(
        "source".to_string(),
        serde_json::Value::String(path.display().to_string()),
    );
    metadata.insert(
        "language".to_string(),
        serde_json::Value::String(file_type.to_string().to_lowercase()),
    );
    metadata.insert("size_bytes".to_string(), serde_json::json!(file_size));

    let path_str = rel_path.to_string_lossy();
    for rule in &rules {
        if let Some(caps) = rule.captures(&path_str) {
            for name in rule.capture_names().flatten() {
                if let Some(match_val) = caps.name(name) {
                    metadata.insert(
                        name.to_string(),
                        serde_json::Value::String(match_val.as_str().to_string()),
                    );
                }
            }
        }
    }

    if let Some(ref sha) = commit_sha {
        metadata.insert(
            "commit_sha".to_string(),
            serde_json::Value::String(sha.clone()),
        );
    }
    if let Some(ref git_ref) = options.git_ref {
        metadata.insert(
            "git_ref".to_string(),
            serde_json::Value::String(git_ref.clone()),
        );
    }
    if let Some(ref global_meta) = options.metadata {
        for (k, v) in global_meta {
            metadata.insert(k.clone(), v.clone());
        }
    }
    let meta_val = Some(serde_json::Value::Object(
        metadata.clone().into_iter().collect(),
    ));

    if is_large {
        if let Some(streaming_parser) = parser_factory.get_streaming_parser(file_type) {
            if OUTPUT.is_interactive {
                eprintln!(
                    "Info: Streaming large file ({} MB): {}",
                    file_size / 1024 / 1024,
                    rel_path.display()
                );
            }
            return Ok(Some(streaming_parser.parse("", &path, meta_val).await?));
        } else {
            if OUTPUT.is_interactive {
                eprintln!(
                    "Info: Using Two-Pass Ingestion for large file ({} MB): {}",
                    file_size / 1024 / 1024,
                    rel_path.display()
                );
            }
            return Ok(Some(
                crate::ingestion::twopass::TwoPassIngestor::process_large_file(
                    &path,
                    &rel_path,
                    parser_factory,
                    options,
                    file_type,
                    meta_val,
                )
                .await?,
            ));
        }
    }

    let full_bytes = tokio::fs::read(&path).await?;

    // Applied to every file, not only those of unrecognised type. The previous
    // condition was `!file_type.is_supported() && is_binary(..)`, which skipped
    // the check entirely whenever the detector produced a type — so a blob
    // named `.json` or `.txt` was ingested without ever being looked at. An
    // extension is a claim about content, not evidence of it.
    if FileType::is_binary_content(&full_bytes) {
        if OUTPUT.is_interactive {
            eprintln!("Skipping binary file: {}", path.display());
        }
        return Ok(None);
    }

    // Lossy conversion is safe only because the check above ran: without it,
    // binary arrives here as a wall of U+FFFD and embeds as confident garbage
    // that is indistinguishable from real content at search time.
    let content = String::from_utf8_lossy(&full_bytes).to_string();

    debug!(
        "Parsers: {:?}",
        parser_factory.get_parser(file_type).is_some()
    );

    let chunks = if let Some(p) = parser_factory.get_parser(file_type) {
        match p.parse(&content, &path, meta_val).await {
            Ok(c) => c,
            Err(e) => {
                if OUTPUT.is_interactive {
                    eprintln!(
                        "Warning: Parser failed for {}: {}. Falling back to text-based chunking.",
                        rel_path.display(),
                        e
                    );
                }
                process_content(
                    &content,
                    &options,
                    &path,
                    &metadata,
                    vecdb_common::FileType::Text,
                    &collection,
                )
                .await?
            }
        }
    } else {
        process_content(&content, &options, &path, &metadata, file_type, &collection).await?
    };

    if options.dry_run {
        // Reported after chunking, not before.
        //
        // "Would ingest: <file>" answers a question nobody has — the shell
        // already listed the files. What is not knowable without doing the work
        // is how many chunks come out, and whether any of them will trip the
        // oversize ceiling. Parsing and chunking are local and cheap; embedding
        // and upserting are the expensive parts, and neither happens here.
        let display_path = if rel_path.as_os_str().is_empty() {
            path.file_name()
                .and_then(|n| n.to_str())
                .unwrap_or(path.to_str().unwrap_or(""))
        } else {
            rel_path.to_str().unwrap_or("")
        };

        let ceiling = options.chunking_for(&collection).ceiling();
        let oversized = chunks.iter().filter(|c| c.content.len() > ceiling).count();

        let bytes: usize = chunks.iter().map(|c| c.content.len()).sum();
        print!(
            "Would ingest: {display_path} — {} chunk(s), {} KB",
            chunks.len(),
            bytes / 1024
        );
        if oversized > 0 {
            // The same policy that would apply for real, named up front rather
            // than discovered in the run summary afterwards.
            print!(
                " [{oversized} over max_chunk_bytes {ceiling} → {}]",
                options.on_oversize
            );
        }
        println!();

        if let Some(ref meta) = options.metadata {
            for (k, v) in meta {
                println!("  Metadata: {}={}", k, v);
            }
        }

        // Empty, so nothing downstream can embed or upsert it.
        return Ok(Some(Vec::new()));
    }

    Ok(Some(chunks))
}

// `is_binary` lived here and scanned only for NUL bytes. It is replaced by
// `FileType::is_binary_content` (vecdb-common), which additionally checks
// magic numbers and UTF-8 validity and is applied to every file rather than
// only to files of unrecognised type. Removed rather than left unused so there
// is one binary check in the codebase, not two with different answers.
