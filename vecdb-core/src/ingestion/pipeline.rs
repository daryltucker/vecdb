use crate::backend::Backend;
use crate::chunking::Chunker;
use crate::embedder::Embedder;
use crate::output::OUTPUT;
use crate::types::Chunk;
use anyhow::Result;
use std::sync::Arc;
use tracing::debug;
use uuid::Uuid;

/// Running tally of chunks that hit the oversize ceiling.
///
/// Collected rather than printed per-chunk: on a large ingest the per-chunk form
/// is a wall of text that scrolls past, which is functionally the same as being
/// silent. One summary at the end, naming files, is what an operator can act on.
#[derive(Default)]
pub struct OversizeReport {
    inner: std::sync::Mutex<OversizeInner>,
}

#[derive(Default)]
struct OversizeInner {
    count: usize,
    ceiling: usize,
    largest: usize,
    documents: std::collections::BTreeSet<String>,
}

/// The file a chunk came from, for reporting.
///
/// `document_id` is a UUID — correct as an identity, useless in a warning. The
/// operator needs the path they can go and look at, so prefer the metadata the
/// parser attached and fall back to the id only when there is nothing better.
fn describe_source(chunk: &Chunk) -> String {
    for key in ["path", "source", "full_path"] {
        if let Some(v) = chunk.metadata.get(key).and_then(|v| v.as_str()) {
            if !v.is_empty() {
                return v.to_string();
            }
        }
    }
    chunk.document_id.clone()
}

impl OversizeReport {
    pub fn new() -> Self {
        Self::default()
    }

    fn record(&self, document: &str, bytes: usize, ceiling: usize) {
        if let Ok(mut inner) = self.inner.lock() {
            inner.count += 1;
            inner.ceiling = ceiling;
            inner.largest = inner.largest.max(bytes);
            inner.documents.insert(document.to_string());
        }
    }

    /// Human-readable summary, or `None` when nothing tripped the ceiling.
    pub fn summary(&self, policy: crate::config::OversizePolicy) -> Option<String> {
        let inner = self.inner.lock().ok()?;
        if inner.count == 0 {
            return None;
        }
        let verb = match policy {
            crate::config::OversizePolicy::Split => "split into labelled parts",
            crate::config::OversizePolicy::Skip => "NOT indexed",
        };
        let mut out = format!(
            "{} chunk(s) across {} file(s) exceeded max_chunk_bytes {} bytes \
             (largest {} bytes) and were {}:",
            inner.count,
            inner.documents.len(),
            inner.ceiling,
            inner.largest,
            verb
        );
        for d in inner.documents.iter().take(10) {
            out.push_str(&format!("\n    {d}"));
        }
        if inner.documents.len() > 10 {
            out.push_str(&format!(
                "\n    ... and {} more",
                inner.documents.len() - 10
            ));
        }
        Some(out)
    }
}

const VECDB_NAMESPACE: Uuid = Uuid::from_u128(0xa1a2a3a4_b1b2_c1c2_d1d2_e1e2e3e4e5e6);

/// The knobs `flush_chunks` needs, grouped.
///
/// Same reasoning as `SearchParams` on the backend trait: this grew from five
/// positional arguments to nine over two changes, and the next knob would have
/// churned every call site again. A struct with defaults means adding one
/// touches only the code that cares about it.
pub struct FlushParams {
    pub gpu_batch_size: usize,
    /// Dimension to embed at — the *destination's*, not the run's. Routed
    /// ingest fans across collections that need not share one.
    pub target_dim: Option<usize>,
    /// Byte ceiling. `None` falls back to a value derived from the default
    /// chunk size; callers in `ingestion::mod` always resolve it properly.
    pub max_chunk_bytes: Option<usize>,
    pub on_oversize: crate::config::OversizePolicy,
}

impl Default for FlushParams {
    fn default() -> Self {
        Self {
            gpu_batch_size: 1,
            target_dim: None,
            max_chunk_bytes: None,
            on_oversize: crate::config::OversizePolicy::default(),
        }
    }
}

pub async fn flush_chunks(
    backend: &Arc<dyn Backend + Send + Sync>,
    embedder: &Arc<dyn Embedder + Send + Sync>,
    collection: &str,
    chunks: &mut Vec<Chunk>,
    params: &FlushParams,
    report: &OversizeReport,
) -> Result<()> {
    let FlushParams {
        gpu_batch_size,
        target_dim,
        max_chunk_bytes,
        on_oversize,
    } = *params;
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

        // Callers in `ingestion::mod` always resolve this, so the fallback is a
        // last resort for direct library use rather than the common path. It is
        // derived from the default chunk size for the same reason the configured
        // one is (see `config::BYTES_PER_CHUNK_UNIT`): the previous hardcoded
        // 6000 was *below* the 6144 that real profiles configure, which turned a
        // safety ceiling into an unconditional second chunking pass.
        let active_max_chunk_bytes = max_chunk_bytes.unwrap_or_else(|| {
            crate::config::default_max_chunk_bytes(crate::config::DEFAULT_TARGET_CHUNK_SIZE)
        });

        let mut final_chunks: Vec<Chunk> = Vec::with_capacity(new_chunks.len());
        let fallback_chunker = crate::chunking::FixedWidthChunker;
        let fallback_params = crate::chunking::ChunkParams {
            target_chunk_size: active_max_chunk_bytes,
            max_chunk_bytes: Some(active_max_chunk_bytes),
            chunk_overlap: 0,
            tokenizer: "bytes".to_string(),
            file_extension: None,
        };

        for chunk in new_chunks {
            if chunk.content.len() > active_max_chunk_bytes {
                // Never truncate. The invariant is that a stored chunk must not
                // claim more than its content contains: a chunk labelled
                // `main.rs:1-400` holding 60% of that range is a lie no reader
                // can detect. Both policies below preserve the invariant —
                // `split` keeps the content and labels the parts, `skip` keeps
                // the content out. Neither aborts the run.
                report.record(
                    &describe_source(&chunk),
                    chunk.content.len(),
                    active_max_chunk_bytes,
                );

                if on_oversize == crate::config::OversizePolicy::Skip {
                    tracing::warn!(
                        document = %describe_source(&chunk),
                        chunk_bytes = chunk.content.len(),
                        max_chunk_bytes = active_max_chunk_bytes,
                        "oversized chunk skipped (on_oversize = skip); content not indexed"
                    );
                    continue;
                }

                tracing::warn!(
                    document = %describe_source(&chunk),
                    chunk_bytes = chunk.content.len(),
                    max_chunk_bytes = active_max_chunk_bytes,
                    "oversized chunk split; structural boundaries discarded for the split parts"
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

        // `skip` can empty the batch entirely — a file whose only chunk is
        // oversized leaves nothing to write. Qdrant rejects an empty upsert
        // ("Empty update request"), which would turn the gentler policy into the
        // one that aborts the run. Nothing to write is a success.
        // `skip` can empty the batch entirely — a file whose only chunk is
        // oversized leaves nothing to write. Qdrant rejects an empty upsert
        // ("Empty update request"), which would turn the gentler policy into the
        // one that aborts the run. Nothing to write is a success.
        if final_chunks.is_empty() {
            return Ok(());
        }

        let gpu_batch_size = gpu_batch_size.max(1);
        let texts: Vec<String> = final_chunks.iter().map(|c| c.content.clone()).collect();
        let total_chunks = final_chunks.len();
        let mut all_vectors = Vec::with_capacity(total_chunks);

        for chunk_start in (0..total_chunks).step_by(gpu_batch_size) {
            let chunk_end = std::cmp::min(chunk_start + gpu_batch_size, total_chunks);
            let batch_texts = &texts[chunk_start..chunk_end];
            let batch_vectors = embedder.embed_batch(batch_texts, target_dim).await?;
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
    // The collection this content is headed for. Chunking is a property of the
    // destination, not of the run: a `.vecdbrc` fans one run across collections
    // whose configured `target_chunk_size` may differ by 16x.
    collection: &str,
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

    let spec = options.chunking_for(collection);
    let params = crate::chunking::ChunkParams {
        target_chunk_size: spec.target_chunk_size,
        max_chunk_bytes: spec.max_chunk_bytes,
        chunk_overlap: spec.chunk_overlap,
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
            byte_start: char_count,
            byte_end: char_count + chunk_len,
            start_line: text_chunk.line_start,
            end_line: text_chunk.line_end,
        });

        char_count += chunk_len;
    }

    Ok(chunks)
}
