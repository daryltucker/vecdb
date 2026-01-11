use crate::types::Chunk;
use anyhow::Result;
use std::path::PathBuf;
use std::sync::Arc;
use vecdb_common::FileTypeDetector;
use crate::parsers::ParserFactory;
use crate::ingestion::IngestionOptions;
use crate::ingestion::pipeline::process_content;
use regex::Regex;
use tokio::io::AsyncReadExt;
use tracing::debug;
use crate::output::OUTPUT;

pub async fn process_single_file(
    path: PathBuf,
    rel_path: PathBuf,
    detector: Arc<dyn FileTypeDetector>,
    parser_factory: Arc<dyn ParserFactory>,
    rules: Vec<Regex>,
    options: Arc<IngestionOptions>,
    commit_sha: Option<String>,
) -> Result<Option<Vec<Chunk>>> {
    let metadata_fs = tokio::fs::metadata(&path).await
        .map_err(|e| anyhow::anyhow!("Failed to stat {}: {}", path.display(), e))?;
    
    let file_size = metadata_fs.len();
    const LARGE_FILE_THRESHOLD: u64 = 50 * 1024 * 1024; // 50 MB
    let is_large = file_size > LARGE_FILE_THRESHOLD;

    let mut file = tokio::fs::File::open(&path).await?;
    let mut header_buffer = vec![0u8; 8192];
    let n = file.read(&mut header_buffer).await?;
    let content_preview = &header_buffer[..n];

    let file_type = detector.detect(&path, content_preview);
    
    if !file_type.is_supported() {
        if is_binary(content_preview) { return Ok(None); }
    }

    if let Some(ref exts) = options.extensions {
        let current_ext = path.extension().and_then(|s| s.to_str()).unwrap_or("");
        if !exts.iter().any(|e| e.eq_ignore_ascii_case(current_ext)) { return Ok(None); }
    }
    
    if let Some(ref excludes) = options.excludes {
        let path_str = path.to_string_lossy();
        for pattern in excludes {
            if let Ok(glob) = glob::Pattern::new(pattern) {
                if glob.matches(&path_str) || glob.matches(path.file_name().unwrap_or_default().to_str().unwrap_or("")) {
                    return Ok(None);
                }
            }
        }
    }

    if options.dry_run { return Ok(None); }

    let mut metadata = std::collections::HashMap::new();
    metadata.insert("path".to_string(), serde_json::Value::String(rel_path.display().to_string()));
    metadata.insert("source_type".to_string(), serde_json::Value::String("file".to_string()));
    metadata.insert("full_path".to_string(), serde_json::Value::String(path.display().to_string()));
    metadata.insert("language".to_string(), serde_json::Value::String(file_type.to_string().to_lowercase()));
    metadata.insert("size_bytes".to_string(), serde_json::json!(file_size));

    let path_str = rel_path.to_string_lossy();
    for rule in &rules {
        if let Some(caps) = rule.captures(&path_str) {
            for name in rule.capture_names().flatten() {
                if let Some(match_val) = caps.name(name) {
                    metadata.insert(name.to_string(), serde_json::Value::String(match_val.as_str().to_string()));
                }
            }
        }
    }
    
    if let Some(ref sha) = commit_sha {
        metadata.insert("commit_sha".to_string(), serde_json::Value::String(sha.clone()));
    }
    if let Some(ref git_ref) = options.git_ref {
        metadata.insert("git_ref".to_string(), serde_json::Value::String(git_ref.clone()));
    }
    if let Some(ref global_meta) = options.metadata {
        for (k, v) in global_meta {
            metadata.insert(k.clone(), v.clone());
        }
    }
    let meta_val = Some(serde_json::Value::Object(metadata.clone().into_iter().collect()));

    if is_large {
        if let Some(streaming_parser) = parser_factory.get_streaming_parser(file_type) {
            if OUTPUT.is_interactive {
                eprintln!("Info: Streaming large file ({} MB): {}", file_size / 1024 / 1024, rel_path.display());
            }
            return Ok(Some(streaming_parser.parse("", &path, meta_val).await?));
        } else {
            if OUTPUT.is_interactive {
                eprintln!("Info: Using Two-Pass Ingestion for large file ({} MB): {}", file_size / 1024 / 1024, rel_path.display());
            }
            return Ok(Some(crate::ingestion::twopass::TwoPassIngestor::process_large_file(
                &path,
                &rel_path,
                parser_factory,
                options,
                file_type,
                meta_val
            ).await?));
        }
    }

    let full_bytes = tokio::fs::read(&path).await?;
    if !file_type.is_supported() && is_binary(&full_bytes) {
         return Ok(None);
    }
    
    let content = String::from_utf8_lossy(&full_bytes).to_string();
    
    debug!("Parsers: {:?}", parser_factory.get_parser(file_type).is_some());

    let chunks = if let Some(p) = parser_factory.get_parser(file_type) {
        match p.parse(&content, &path, meta_val).await {
            Ok(c) => c,
            Err(e) => {
                if OUTPUT.is_interactive {
                    eprintln!("Warning: Parser failed for {}: {}. Falling back to simple chunking.", rel_path.display(), e);
                }
                process_content(&content, &options, &path, &metadata, file_type).await?
            }
        }
    } else {
        process_content(&content, &options, &path, &metadata, file_type).await?
    };
    
    Ok(Some(chunks))
}

fn is_binary(content: &[u8]) -> bool {
    let len = std::cmp::min(content.len(), 8192);
    content[0..len].contains(&0)
}
