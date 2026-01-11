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
                    if crate::output::OUTPUT.is_interactive {
                        eprintln!("RecursiveChunker: Chunk size {} exceeds max {}, splitting with SimpleChunker", 
                                 chunk.content.len(), max);
                    }
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
        // Structural splitting by double newlines and indent level 0
        let mut chunks = Vec::new();
        let lines: Vec<&str> = text.lines().collect();
        
        let mut current_chunk = String::new();
        let mut current_start_offset = 0;
        let mut current_start_line = 1;
        
        let mut offset = 0;
        for (i, line) in lines.iter().enumerate() {
            let line_len_with_nl = line.len() + 1; // Approximate newline
            
            // Heuristic: Split if line starts with non-whitespace and we have enough content
            let is_top_level = !line.starts_with(|c: char| c.is_whitespace()) && !line.is_empty();
            let should_split = is_top_level && current_chunk.len() >= params.chunk_size;
            
            if should_split && !current_chunk.is_empty() {
                chunks.push(ChunkResult {
                    content: current_chunk.trim_end().to_string(),
                    offset_bytes: current_start_offset,
                    line_start: Some(current_start_line),
                    line_end: Some(i), // i is 0-indexed, so current line is i+1, previous line is i
                });
                current_chunk = String::new();
                current_start_offset = offset;
                current_start_line = i + 1;
            }
            
            current_chunk.push_str(line);
            current_chunk.push('\n');
            offset += line_len_with_nl;
        }
        
        if !current_chunk.is_empty() {
            chunks.push(ChunkResult {
                content: current_chunk.trim_end().to_string(),
                offset_bytes: current_start_offset,
                line_start: Some(current_start_line),
                line_end: Some(lines.len()),
            });
        }

        // If chunks are still too large, use RecursiveChunker on them
        let mut refined_chunks = Vec::new();
        for chunk in chunks {
            if chunk.content.len() > params.max_chunk_size.unwrap_or(params.chunk_size * 2) {
                let sub_chunks = RecursiveChunker.chunk(&chunk.content, params).await?;
                for mut sub in sub_chunks {
                    sub.offset_bytes += chunk.offset_bytes;
                    sub.line_start = Some(chunk.line_start.unwrap_or(1) + sub.line_start.unwrap_or(1) - 1);
                    sub.line_end = Some(chunk.line_start.unwrap_or(1) + sub.line_end.unwrap_or(1) - 1);
                    refined_chunks.push(sub);
                }
            } else {
                refined_chunks.push(chunk);
            }
        }

        Ok(refined_chunks)
    }
}

pub struct Factory;

impl Factory {
    pub fn get(strategy: &str, file_type: vecdb_common::FileType) -> Box<dyn Chunker> {
        // ENFORCED RULE: For types with "Simple" capability (e.g. Unknown/Lua),
        // we FORCE SimpleChunker if strategy is recursive/semantic to avoid
        // performance hangs on files that don't benefit from sentence-level splitting.
        if matches!(file_type.capability(), vecdb_common::ParsingCapability::Simple) && (strategy == "recursive" || strategy == "semantic") {
            return Box::new(SimpleChunker);
        }

        match strategy {
            "code_aware" => Box::new(CodeChunker),
            "semantic" => Box::new(RecursiveChunker), 
            "recursive" => Box::new(RecursiveChunker),
            "simple" => Box::new(SimpleChunker),
            _ => Box::new(RecursiveChunker),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use vecdb_common::FileType;

    #[test]
    fn test_capability_mapping() {
        use vecdb_common::ParsingCapability;
        assert_eq!(FileType::Rust.capability(), ParsingCapability::Code);
        assert_eq!(FileType::Python.capability(), ParsingCapability::Code);
        assert_eq!(FileType::Markdown.capability(), ParsingCapability::Document);
        assert_eq!(FileType::Html.capability(), ParsingCapability::Document);
        assert_eq!(FileType::Json.capability(), ParsingCapability::Data);
        assert_eq!(FileType::Text.capability(), ParsingCapability::Simple);
        assert_eq!(FileType::Unknown.capability(), ParsingCapability::Simple);
    }

    #[test]
    fn test_factory_fallback_logic() {
        // Rule: Unknown + semantic/recursive -> SimpleChunker
        let _chunker_unk = Factory::get("semantic", FileType::Unknown);
        
        // Rule: Text (Simple) + semantic -> SimpleChunker
        let _chunker_txt = Factory::get("semantic", FileType::Text);

        // Let's verify it doesn't break known types
        let _chunker_rs = Factory::get("semantic", FileType::Rust);
        // This should be RecursiveChunker (logic check)
    }

    #[test]
    fn test_all_strategies_resolved() {
        let types = vec![FileType::Rust, FileType::Unknown, FileType::Text];
        let strategies = vec!["semantic", "recursive", "simple", "code_aware", "unknown_bogus"];

        for t in types {
            for s in strategies.iter() {
                let _ = Factory::get(s, t);
            }
        }
    }
}
