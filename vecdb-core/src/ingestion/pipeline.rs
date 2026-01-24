use crate::backend::Backend;
use crate::chunking::Chunker;
use crate::embedder::Embedder;
use crate::output::OUTPUT;
use crate::types::Chunk;
use anyhow::Result;
use std::sync::Arc;
use tracing::debug;
use uuid::Uuid;

const VECDB_NAMESPACE: Uuid = Uuid::from_u128(0xa1a2a3a4_b1b2_c1c2_d1d2_e1e2e3e4e5e6);

pub async fn flush_chunks(
    backend: &Arc<dyn Backend + Send + Sync>,
    embedder: &Arc<dyn Embedder + Send + Sync>,
    collection: &str,
    chunks: &mut Vec<Chunk>,
    gpu_batch_size: usize,
) -> Result<()> {
    if chunks.is_empty() {
        return Ok(());
    }

    let ids: Vec<String> = chunks.iter().map(|c| c.id.clone()).collect();
    let existing_ids = backend.points_exists(collection, ids).await?;

    let mut new_chunks: Vec<Chunk> = Vec::new();
    for chunk in chunks.drain(..) {
        if !existing_ids.contains(&chunk.id) {
            new_chunks.push(chunk);
        }
    }

    if !new_chunks.is_empty() {
        debug!("Embedding {} new chunks...", new_chunks.len());

        const MAX_CHUNK_CHARS: usize = 6000;

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
                debug!(
                    "Warning: Oversized chunk detected ({} chars). Splitting...",
                    chunk.content.len()
                );
                let sub_chunks: Vec<crate::chunking::ChunkResult> = fallback_chunker
                    .chunk(&chunk.content, &fallback_params)
                    .await?;

                for (idx, sub) in sub_chunks.into_iter().enumerate() {
                    let mut part_chunk = chunk.clone();
                    part_chunk.content = sub.content;

                    let seed = format!("{}-part-{}", chunk.id, idx);
                    part_chunk.id =
                        uuid::Uuid::new_v5(&VECDB_NAMESPACE, seed.as_bytes()).to_string();

                    part_chunk
                        .metadata
                        .insert("split_part".to_string(), serde_json::json!(idx));
                    part_chunk.metadata.insert(
                        "original_chunk_id".to_string(),
                        serde_json::Value::String(chunk.id.clone()),
                    );

                    if let (Some(base_start), Some(_base_end)) = (chunk.start_line, chunk.end_line)
                    {
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

        let gpu_batch_size = gpu_batch_size.max(1);
        let texts: Vec<String> = final_chunks.iter().map(|c| c.content.clone()).collect();
        let total_chunks = final_chunks.len();
        let mut all_vectors = Vec::with_capacity(total_chunks);

        for chunk_start in (0..total_chunks).step_by(gpu_batch_size) {
            let chunk_end = std::cmp::min(chunk_start + gpu_batch_size, total_chunks);
            let batch_texts = &texts[chunk_start..chunk_end];
            let batch_vectors = embedder.embed_batch(batch_texts).await?;
            all_vectors.extend(batch_vectors);
        }

        for (i, chunk) in final_chunks.iter_mut().enumerate() {
            if i < all_vectors.len() {
                chunk.vector = Some(all_vectors[i].clone());
                chunk.metadata.insert(
                    "_model_name".to_string(),
                    serde_json::Value::String(embedder.model_name()),
                );
            }
        }

        backend.upsert(collection, final_chunks).await?;
    } else if OUTPUT.is_interactive {
        eprintln!("All chunks already exist. Skipping embedding.");
    }

    Ok(())
}

pub async fn process_content(
    content: &str,
    options: &crate::ingestion::IngestionOptions,
    path: &std::path::Path,
    base_metadata: &std::collections::HashMap<String, serde_json::Value>,
    file_type: vecdb_common::FileType,
) -> Result<Vec<Chunk>> {
    let doc_id = Uuid::new_v4().to_string();
    let commit_sha = base_metadata
        .get("commit_sha")
        .and_then(|v| v.as_str())
        .unwrap_or("HEAD");

    let chunker = crate::chunking::Factory::get(&options.strategy, file_type);
    let ext = path
        .extension()
        .and_then(|s| s.to_str())
        .map(|s| s.to_string());

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
        let mut metadata = base_metadata.clone();
        metadata.insert("chunk_index".to_string(), serde_json::json!(idx));

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
            start_line: text_chunk.line_start,
            end_line: text_chunk.line_end,
        });

        char_count += chunk_len;
    }

    Ok(chunks)
}
