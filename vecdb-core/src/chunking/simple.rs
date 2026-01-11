/*
 * PURPOSE:
 *   Simple "dumb" chunker that enforces hard size limits by splitting at byte boundaries.
 *   Used as a safety fallback when smart chunkers produce oversized chunks.
 *
 * REQUIREMENTS:
 *   - MUST respect max_chunk_size absolutely
 *   - Should split at newlines when possible (avoid mid-line splits)
 *   - Fast and simple (no complex logic)
 *
 * USAGE:
 *   Called by RecursiveChunker and CodeChunker when they produce chunks > max_size
 */

use crate::chunking::{ChunkParams, Chunker, ChunkResult};
use anyhow::Result;
use async_trait::async_trait;

/// Simple chunker that forcefully splits text at size boundaries.
/// Guarantees no chunk exceeds max_chunk_size.
pub struct SimpleChunker;

#[async_trait]
impl Chunker for SimpleChunker {
    async fn chunk(&self, text: &str, params: &ChunkParams) -> Result<Vec<ChunkResult>> {
        let max_size = params.max_chunk_size.unwrap_or(6000);
        let mut chunks = Vec::new();
        let mut start = 0;
        
        let line_counter = vecdb_common::LineCounter::new(text);
        
        while start < text.len() {
            let mut end = start + max_size;
            
            if end >= text.len() {
                end = text.len();
            } else {
                // Ensure we split at a valid char boundary
                while !text.is_char_boundary(end) {
                    end -= 1;
                }
                
                // Safety: If max_size is smaller than a single multi-byte char,
                // we might end up with end == start. We must advance by at least one char
                // to avoid infinite loops, even if it violates max_chunk_size slightly.
                if end == start {
                    if let Some(next_char_boundary) = text[start..].char_indices().nth(1).map(|(i, _)| start + i) {
                        end = next_char_boundary;
                    } else {
                        end = text.len();
                    }
                }
            }

            // Try to split at last newline before max to avoid mid-line splits
            // We search in [start..end]
            let split_point = if end < text.len() {
                 match text[start..end].rfind('\n') {
                     Some(pos) => start + pos + 1, // +1 to include newline
                     None => end,
                 }
            } else {
                end
            };
            
            let content = text[start..split_point].to_string();
            
            let line_start = line_counter.get_line_number(start);
            let line_end = line_counter.get_line_number(split_point.saturating_sub(1)).max(line_start);

            chunks.push(ChunkResult {
                content,
                offset_bytes: start,
                line_start: Some(line_start),
                line_end: Some(line_end),
            });
            start = split_point;
        }
        
        Ok(chunks)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_simple_chunker_enforces_max_size() {
        let chunker = SimpleChunker;
        let text = "a".repeat(10000);  // 10KB of 'a'
        let params = ChunkParams {
            chunk_size: 512,
            max_chunk_size: Some(1000),
            chunk_overlap: 0,
            tokenizer: "char".to_string(),
            file_extension: None,
        };
        
        let result = chunker.chunk(&text, &params).await.unwrap();
        
        // Should split into ~10 chunks of 1000 chars each
        assert!(result.len() >= 10);
        for chunk in result {
            assert!(chunk.content.len() <= 1000, "Chunk size {} exceeds max 1000", chunk.content.len());
        }
    }
    
    #[tokio::test]
    async fn test_simple_chunker_splits_at_newlines() {
        let chunker = SimpleChunker;
        let text = "Line 1\n".repeat(100);  // 700 chars with newlines
        let params = ChunkParams {
            chunk_size: 512,
            max_chunk_size: Some(50),
            chunk_overlap: 0,
            tokenizer: "char".to_string(),
            file_extension: None,
        };
        
        let result = chunker.chunk(&text, &params).await.unwrap();
        
        // Each chunk should end with a newline (except possibly the last)
        for chunk in &result[..result.len()-1] {
            assert!(chunk.content.ends_with('\n'), "Chunk should end with newline: {:?}", chunk);
        }
    }
}
