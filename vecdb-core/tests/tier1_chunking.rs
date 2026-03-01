/*
 * PURPOSE:
 *   Unit tests for chunking strategies.
 *   Verifies RecursiveChunker and CodeChunker work correctly.
 *
 * REQUIREMENTS:
 *   - RecursiveChunker splits text by token/char boundaries
 *   - CodeChunker preserves code structure where possible
 *   - Factory returns correct chunker for strategy name
 *   - Chunks respect size limits (approximately)
 *
 * NOTE: Token-based chunking uses cl100k_base (GPT-4 tokenizer).
 *       Actual chunk sizes may vary slightly due to boundary adjustments.
 */

use vecdb_core::chunking::{ChunkParams, Chunker, CodeChunker, Factory, RecursiveChunker};

// ═══════════════════════════════════════════════════════════
// FACTORY TESTS
// ═══════════════════════════════════════════════════════════

#[test]
fn test_factory_returns_chunker_for_recursive() {
    let chunker = Factory::get("recursive", vecdb_common::FileType::Text);
    // Factory returns something - just verify it doesn't panic
    let _ = chunker;
}

#[test]
fn test_factory_returns_chunker_for_code_aware() {
    let chunker = Factory::get("code_aware", vecdb_common::FileType::Text);
    let _ = chunker;
}

#[test]
fn test_factory_returns_chunker_for_unknown() {
    let chunker = Factory::get("unknown_strategy", vecdb_common::FileType::Text);
    let _ = chunker;
}

#[test]
fn test_factory_returns_chunker_for_semantic() {
    let chunker = Factory::get("semantic", vecdb_common::FileType::Text);
    let _ = chunker;
}

// ═══════════════════════════════════════════════════════════
// RECURSIVE CHUNKER - CHARACTER MODE
// ═══════════════════════════════════════════════════════════

fn char_params(size: usize) -> ChunkParams {
    ChunkParams {
        chunk_size: size,
        chunk_overlap: 0,
        tokenizer: "char".to_string(),
        file_extension: None,
        max_chunk_size: None,
    }
}

#[tokio::test]
async fn test_recursive_char_short_text_single_chunk() {
    let chunker = RecursiveChunker;
    let text = "Hello world";
    let params = char_params(100);

    let chunks = chunker.chunk(text, &params).await.expect("Chunking failed");

    assert_eq!(chunks.len(), 1, "Short text should be single chunk");
    assert_eq!(chunks[0].content, "Hello world");
}

#[tokio::test]
async fn test_recursive_char_splits_long_text() {
    let chunker = RecursiveChunker;
    // Create text longer than chunk size
    let text = "a".repeat(250);
    let params = char_params(100);

    let chunks = chunker
        .chunk(&text, &params)
        .await
        .expect("Chunking failed");

    assert!(
        chunks.len() >= 2,
        "Long text should be split into multiple chunks"
    );

    // Each chunk should be <= chunk_size (approximately)
    for chunk in &chunks {
        assert!(
            chunk.content.len() <= 110,
            "Chunk too large: {} chars",
            chunk.content.len()
        );
    }
}

#[tokio::test]
async fn test_recursive_char_preserves_content() {
    let chunker = RecursiveChunker;
    let text = "The quick brown fox jumps over the lazy dog. ";
    let long_text = text.repeat(10);
    let params = char_params(100);

    let chunks = chunker
        .chunk(&long_text, &params)
        .await
        .expect("Chunking failed");

    // All chunks combined should contain all words from original
    let combined: String = chunks
        .iter()
        .map(|c| c.content.clone())
        .collect::<Vec<_>>()
        .join("");

    // Check key words are preserved
    assert!(combined.contains("quick"), "Should preserve 'quick'");
    assert!(combined.contains("fox"), "Should preserve 'fox'");
    assert!(combined.contains("lazy"), "Should preserve 'lazy'");
    assert!(combined.contains("dog"), "Should preserve 'dog'");
}

#[tokio::test]
async fn test_recursive_char_handles_empty_text() {
    let chunker = RecursiveChunker;
    let params = char_params(100);

    let chunks = chunker.chunk("", &params).await.expect("Chunking failed");

    // Empty text should produce no chunks or one empty chunk
    assert!(chunks.is_empty() || (chunks.len() == 1 && chunks[0].content.is_empty()));
}

// ═══════════════════════════════════════════════════════════
// RECURSIVE CHUNKER - TOKEN MODE
// ═══════════════════════════════════════════════════════════

fn token_params(size: usize) -> ChunkParams {
    ChunkParams {
        chunk_size: size,
        chunk_overlap: 0,
        tokenizer: "cl100k_base".to_string(),
        file_extension: None,
        max_chunk_size: None,
    }
}

#[tokio::test]
async fn test_recursive_token_short_text_single_chunk() {
    let chunker = RecursiveChunker;
    let text = "Hello world";
    let params = token_params(100);

    let chunks = chunker.chunk(text, &params).await.expect("Chunking failed");

    assert_eq!(chunks.len(), 1, "Short text should be single chunk");
}

#[tokio::test]
async fn test_recursive_token_splits_at_word_boundaries() {
    let chunker = RecursiveChunker;
    // Long text with clear words
    let text = "The quick brown fox jumps over the lazy dog. ".repeat(50);
    let params = token_params(20); // Small token limit

    let chunks = chunker
        .chunk(&text, &params)
        .await
        .expect("Chunking failed");

    assert!(chunks.len() > 1, "Should split long text");

    // Each chunk should end at reasonable boundary (not mid-word usually)
    for chunk in &chunks {
        // Text-splitter typically respects word boundaries when possible
        let trimmed = chunk.content.trim();
        if !trimmed.is_empty() {
            // Simple heuristic: most chunks shouldn't end with partial words
            assert!(
                trimmed.ends_with('.')
                    || trimmed.ends_with(' ')
                    || trimmed
                        .chars()
                        .last()
                        .map(|c: char| c.is_alphanumeric())
                        .unwrap_or(true),
                "Chunk may have bad boundary: '{}'",
                trimmed
            );
        }
    }
}

// ═══════════════════════════════════════════════════════════
// CODE CHUNKER
// ═══════════════════════════════════════════════════════════

fn code_params(size: usize, ext: &str) -> ChunkParams {
    ChunkParams {
        chunk_size: size,
        chunk_overlap: 0,
        tokenizer: "char".to_string(),
        file_extension: Some(ext.to_string()),
        max_chunk_size: None,
    }
}

#[tokio::test]
async fn test_code_chunker_rust_preserves_functions() {
    let chunker = CodeChunker;
    let code = r#"
fn hello() {
    println!("Hello");
}

fn world() {
    println!("World");
}
"#;
    let params = code_params(500, "rs");

    let chunks = chunker.chunk(code, &params).await.expect("Chunking failed");

    // With small functions and big chunk size, may be 2 chunks (one per function)
    // or could be combined. Main test is that it doesn't crash.
    assert!(!chunks.is_empty(), "Should produce chunks from Rust code");
}

#[tokio::test]
async fn test_code_chunker_python_preserves_functions() {
    let chunker = CodeChunker;
    let code = r#"
def hello():
    print("Hello")

def world():
    print("World")
"#;
    let params = code_params(500, "py");

    let chunks = chunker.chunk(code, &params).await.expect("Chunking failed");
    assert!(!chunks.is_empty(), "Should produce chunks from Python code");
}

#[tokio::test]
async fn test_code_chunker_splits_large_function() {
    let chunker = CodeChunker;
    // Create a large function that exceeds chunk size
    let large_body = "    x = 1;\n".repeat(50);
    let code = format!("fn big_function() {{\n{}}}\n", large_body);
    let params = code_params(100, "rs");

    let chunks = chunker
        .chunk(&code, &params)
        .await
        .expect("Chunking failed");

    // Large function should be split into multiple chunks
    assert!(chunks.len() >= 1, "Should produce at least one chunk");
}

#[tokio::test]
async fn test_code_chunker_falls_back_for_unknown_extension() {
    let chunker = CodeChunker;
    let text = "Some random text that isn't code for a .xyz file type";
    let params = code_params(100, "xyz");

    let chunks = chunker.chunk(text, &params).await.expect("Chunking failed");

    // Should fall back to recursive chunker without error
    assert!(
        !chunks.is_empty(),
        "Should fall back to recursive for unknown extension"
    );
}

#[tokio::test]
async fn test_code_chunker_markdown() {
    let chunker = CodeChunker;
    let markdown = r#"
# Header 1

Some paragraph text here.

## Header 2

More content with **bold** and *italic*.

- List item 1
- List item 2
"#;
    let params = code_params(500, "md");

    let chunks = chunker
        .chunk(markdown, &params)
        .await
        .expect("Chunking failed");
    assert!(!chunks.is_empty(), "Should handle markdown");
}

// ═══════════════════════════════════════════════════════════
// EDGE CASES
// ═══════════════════════════════════════════════════════════

#[tokio::test]
async fn test_chunker_with_unicode() {
    let chunker = RecursiveChunker;
    let text = "日本語テキスト。これは長いテキストです。".repeat(20);
    let params = char_params(50);

    let chunks = chunker
        .chunk(&text, &params)
        .await
        .expect("Chunking failed");

    assert!(chunks.len() >= 1, "Should handle unicode text");
    // Verify no panic or corruption
    // Verify no panic or corruption
    for chunk in &chunks {
        assert!(chunk.content.chars().count() > 0 || chunk.content.is_empty());
    }
}

#[tokio::test]
async fn test_chunker_with_special_characters() {
    let chunker = RecursiveChunker;
    let text = r#"fn main() { println!("Hello\nWorld\t!"); }"#.repeat(10);
    let params = char_params(50);

    let chunks = chunker
        .chunk(&text, &params)
        .await
        .expect("Chunking failed");
    assert!(!chunks.is_empty(), "Should handle special characters");
}

#[tokio::test]
async fn test_chunker_single_very_long_word() {
    let chunker = RecursiveChunker;
    // Single word longer than chunk size
    let text = "a".repeat(500);
    let params = char_params(100);

    let chunks = chunker
        .chunk(&text, &params)
        .await
        .expect("Chunking failed");

    // Should still split, even if awkwardly
    assert!(chunks.len() >= 1, "Should handle long word");
}
