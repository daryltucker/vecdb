use anyhow::Result;
use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use text_splitter::{Characters, ChunkConfig, TextSplitter};
use tiktoken_rs::cl100k_base;

pub mod simple;
pub use simple::FixedWidthChunker;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChunkParams {
    pub target_chunk_size: usize,
    pub max_chunk_bytes: Option<usize>, // Hard limit for chunk size
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

static TOKENIZER_CACHE: Lazy<Option<CoreBPE>> = Lazy::new(|| cl100k_base().ok());

pub struct RecursiveChunker;

#[async_trait]
impl Chunker for RecursiveChunker {
    async fn chunk(&self, text: &str, params: &ChunkParams) -> Result<Vec<ChunkResult>> {
        let target_chunk_size = params.target_chunk_size;

        let indices: Vec<(usize, &str)> = if params.tokenizer == "cl100k_base" {
            if let Some(tokenizer) = TOKENIZER_CACHE.as_ref() {
                let sizer = tokenizer.clone();
                let config = ChunkConfig::new(target_chunk_size)
                    .with_sizer(sizer)
                    .with_trim(true);
                let splitter = TextSplitter::new(config);
                splitter.chunk_indices(text).collect()
            } else {
                let config = ChunkConfig::new(target_chunk_size)
                    .with_sizer(Characters)
                    .with_trim(true);
                let splitter = TextSplitter::new(config);
                splitter.chunk_indices(text).collect()
            }
        } else if params.tokenizer == "bytes" {
            // Byte-window slicing, with the window nudged back to the nearest
            // UTF-8 boundary so the output is always valid.
            //
            // Formerly spelled "char", which it never was: `target_chunk_size` is added
            // to a byte offset here. On ASCII the two coincide; on anything else
            // the window is shorter in characters than the number suggests.
            // Faster than the text_splitter path, which is why it exists.
            let mut indices = Vec::new();
            let mut start = 0;
            let step = target_chunk_size.saturating_sub(params.chunk_overlap);
            if step == 0 {
                // Avoid infinite loop if overlap >= target_chunk_size
                indices.push((0, text));
            } else {
                while start < text.len() {
                    let mut end = (start + target_chunk_size).min(text.len());
                    // Ensure we split at a valid char boundary
                    while end > start && !text.is_char_boundary(end) {
                        end -= 1;
                    }
                    // If end == start, advance to next char
                    if end == start && end < text.len() {
                        if let Some(next_boundary) =
                            text[end..].char_indices().next().map(|(i, _)| end + i)
                        {
                            end = next_boundary;
                        } else {
                            end = text.len();
                        }
                    }
                    indices.push((start, &text[start..end]));
                    if end == text.len() {
                        break;
                    }
                    start += step;
                    // Ensure start is at char boundary
                    while start < text.len() && !text.is_char_boundary(start) {
                        start += 1;
                    }
                }
            }
            indices
        } else {
            let config = ChunkConfig::new(target_chunk_size)
                .with_sizer(Characters)
                .with_trim(true);
            let splitter = TextSplitter::new(config);
            splitter.chunk_indices(text).collect()
        };

        let line_counter = vecdb_common::LineCounter::new(text);

        let chunks: Vec<ChunkResult> = indices
            .into_iter()
            .map(|(offset, s)| {
                let line_start = line_counter.get_line_number(offset);
                let line_end = line_counter
                    .get_line_number(offset + s.len().saturating_sub(1))
                    .max(line_start);

                ChunkResult {
                    content: s.to_string(),
                    offset_bytes: offset,
                    line_start: Some(line_start),
                    line_end: Some(line_end),
                }
            })
            .collect();

        // ENFORCE MAX SIZE - use FixedWidthChunker as fallback for oversized chunks
        if let Some(max) = params.max_chunk_bytes {
            let mut safe_chunks = Vec::new();
            let fallback = FixedWidthChunker;

            for chunk in chunks {
                if chunk.content.len() <= max {
                    safe_chunks.push(chunk);
                } else {
                    // Chunk exceeds max, split it forcefully
                    if crate::output::OUTPUT.is_interactive {
                        eprintln!("RecursiveChunker: Chunk size {} exceeds max {}, splitting with FixedWidthChunker", 
                                 chunk.content.len(), max);
                    }
                    // Note: This sub-chunking loses precise line tracking relative to original file for the split parts
                    // but maintain offset approximate.
                    // Ideally FixedWidthChunker also returns ChunkResult.
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

// `CodeChunker` was here, reachable only via `strategy = "code_aware"`.
//
// It could not do its job, by construction. `processor.rs` uses a *parser's*
// chunks when one exists for the file type, and only falls back to a chunker
// when none does; `VecqParserFactory` claims every type except `Unknown`. So a
// chunker never sees a source file, and `code_aware` could only ever apply to
// files with no recognised type — where AST-aware splitting is meaningless.
//
// AST-aware chunking is real and is what vecdb does: it happens in the parser
// path, per vecq element, automatically and for every supported language. It
// was never this chunker's doing.
//
// It also carried a unit bug — comparing `current_chunk.len()` (bytes) against
// `target_chunk_size` (tokens under the default `cl100k_base`), so a config
// asking for 512-token chunks got roughly a fifth of that. Unreachable, so it
// never mattered; deleted rather than fixed, because fixing it would have
// implied the strategy worked.
//
// `Config::load` now rejects `strategy = "code_aware"` with an explanation
// rather than silently selecting a chunker that does nothing.

pub struct Factory;

impl Factory {
    pub fn get(strategy: &str, file_type: vecdb_common::FileType) -> Box<dyn Chunker> {
        // ENFORCED RULE: For types with "Simple" capability (e.g. Unknown/Lua),
        // we FORCE FixedWidthChunker if strategy is recursive/semantic to avoid
        // performance hangs on files that don't benefit from sentence-level splitting.
        if matches!(
            file_type.capability(),
            vecdb_common::ParsingCapability::Simple
        ) && (strategy == "recursive" || strategy == "semantic")
        {
            return Box::new(FixedWidthChunker);
        }

        match strategy {
            "semantic" | "recursive" => Box::new(RecursiveChunker),
            "simple" => Box::new(FixedWidthChunker),
            // Unreachable in practice: `Config::load` rejects any other value,
            // including the retired "code_aware". Kept as a total function so a
            // caller constructing a strategy string by hand degrades to the
            // default rather than panicking mid-ingest.
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
        // Rule: Unknown + semantic/recursive -> FixedWidthChunker
        let _chunker_unk = Factory::get("semantic", FileType::Unknown);

        // Rule: Text (Simple) + semantic -> FixedWidthChunker
        let _chunker_txt = Factory::get("semantic", FileType::Text);

        // Let's verify it doesn't break known types
        let _chunker_rs = Factory::get("semantic", FileType::Rust);
        // This should be RecursiveChunker (logic check)
    }

    #[test]
    fn test_all_strategies_resolved() {
        let types = [FileType::Rust, FileType::Unknown, FileType::Text];
        let strategies = [
            "semantic",
            "recursive",
            "simple",
            "code_aware",
            "unknown_bogus",
        ];

        for t in types {
            for s in strategies.iter() {
                let _ = Factory::get(s, t);
            }
        }
    }
}
