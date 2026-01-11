use text_splitter::{TextSplitter, ChunkConfig, Characters};
use tiktoken_rs::cl100k_base;
use serde::{Deserialize, Serialize};
use async_trait::async_trait;
use anyhow::Result;

pub mod simple;
pub use simple::SimpleChunker;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChunkParams {
    pub chunk_size: usize,
    pub max_chunk_size: Option<usize>,  // Hard limit for chunk size
    pub chunk_overlap: usize, 
    pub tokenizer: String, // "char", "cl100k_base"
    pub file_extension: Option<String>,
}

#[derive(Debug, Clone)]
pub struct ChunkResult {
    pub content: String,
    pub offset_bytes: usize,
    pub line_start: Option<usize>,
    pub line_end: Option<usize>,
}

#[async_trait]
pub trait Chunker: Send + Sync {
    async fn chunk(&self, text: &str, params: &ChunkParams) -> Result<Vec<ChunkResult>>;
}

use once_cell::sync::Lazy;
use tiktoken_rs::CoreBPE;

static TOKENIZER_CACHE: Lazy<Option<CoreBPE>> = Lazy::new(|| {
    cl100k_base().ok()
});

pub struct RecursiveChunker;

#[async_trait]
impl Chunker for RecursiveChunker {
    async fn chunk(&self, text: &str, params: &ChunkParams) -> Result<Vec<ChunkResult>> {
        let chunk_size = params.chunk_size;
        
        let indices: Vec<(usize, &str)> = if params.tokenizer == "cl100k_base" {
            if let Some(tokenizer) = TOKENIZER_CACHE.as_ref() {
                let sizer = tokenizer.clone();
                let config = ChunkConfig::new(chunk_size)
                    .with_sizer(sizer)
                    .with_trim(true);
                let splitter = TextSplitter::new(config);
                splitter.chunk_indices(text).collect()
            } else {
                let config = ChunkConfig::new(chunk_size)
                    .with_sizer(Characters)
                    .with_trim(true);
                let splitter = TextSplitter::new(config);
                splitter.chunk_indices(text).collect()
            }
        } else {
            let config = ChunkConfig::new(chunk_size)
                .with_sizer(Characters)
                .with_trim(true);
            let splitter = TextSplitter::new(config);
            splitter.chunk_indices(text).collect()
        };

        let line_counter = vecdb_common::LineCounter::new(text);

        let chunks: Vec<ChunkResult> = indices.into_iter().map(|(offset, s)| {
             let line_start = line_counter.get_line_number(offset);
             let line_end = line_counter.get_line_number(offset + s.len().saturating_sub(1)).max(line_start);
             
             ChunkResult {
                 content: s.to_string(),
                 offset_bytes: offset,
                 line_start: Some(line_start),
                 line_end: Some(line_end),
             }
        }).collect();
        
        // ENFORCE MAX SIZE - use SimpleChunker as fallback for oversized chunks
        if let Some(max) = params.max_chunk_size {
            let mut safe_chunks = Vec::new();
            let fallback = SimpleChunker;
            
            for chunk in chunks {
                if chunk.content.len() <= max {
                    safe_chunks.push(chunk);
                } else {
                    // Chunk exceeds max, split it forcefully
                    eprintln!("RecursiveChunker: Chunk size {} exceeds max {}, splitting with SimpleChunker", 
                             chunk.content.len(), max);
                    // Note: This sub-chunking loses precise line tracking relative to original file for the split parts
                    // but maintain offset approximate.
                    // Ideally SimpleChunker also returns ChunkResult.
                    let sub_chunks = fallback.chunk(&chunk.content, params).await?;
                     // Adjust offsets for sub-chunks
                    for mut sub in sub_chunks {
                        sub.offset_bytes += chunk.offset_bytes;
                        // Approximate line numbers? complex.
                        // Assume same line range for now or clear them?
                        // Let's clear them to avoid misleading info, or keep original?
                        // Keep original range is safer "this subchunk is WITHIN this range"
                        sub.line_start = chunk.line_start;
                        sub.line_end = chunk.line_end;
                        safe_chunks.push(sub);
                    }
                }
            }
            Ok(safe_chunks)
        } else {
            Ok(chunks)
        }
    }
}

pub struct CodeChunker;

#[async_trait]
impl Chunker for CodeChunker {
    async fn chunk(&self, text: &str, params: &ChunkParams) -> Result<Vec<ChunkResult>> {
        // LEGACY: vecq based chunking is replaced by Smart Ingestion (ParserFactory).
        // This remains as a fallback for the "code_aware" strategy if selected manually,
        // but now mostly delegates to RecursiveChunker to avoid hard dependency on vecq binary.
        RecursiveChunker.chunk(text, params).await
    }
}

pub struct Factory;

impl Factory {
    pub fn get(strategy: &str) -> Box<dyn Chunker> {
        match strategy {
            "code_aware" => Box::new(CodeChunker),
            "semantic" => Box::new(RecursiveChunker), 
            _ => Box::new(RecursiveChunker),
        }
    }
}
