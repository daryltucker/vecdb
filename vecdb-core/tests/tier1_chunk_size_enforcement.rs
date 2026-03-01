use vecdb_core::chunking::{ChunkParams, Chunker, RecursiveChunker};

#[tokio::test]
async fn test_recursive_chunker_enforces_max_size() {
    let chunker = RecursiveChunker;
    let _params = ChunkParams {
        // Unused check params
        chunk_size: 100,
        max_chunk_size: Some(150), // Hard limit
        chunk_overlap: 0,
        tokenizer: "char".to_string(),
        file_extension: None,
    };

    // Create a text with a "long line" that text_splitter might keep together
    // text_splitter usually splits, but let's force it by having no spaces if possible,
    // or just relying on its behavior.
    // Actually, text_splitter splits at char level if forced.
    // To trigger our fallback, we need checking that the *result* of text_splitter
    // respects the limit, which it should if configured correctly, BUT our safety check
    // protects against *configuration errors* or edge cases.

    // Let's create a simulated "oversized" chunk scenario
    // Since we can't easily force text_splitter to fail its own config without bad config,
    // we can test the fallback logic by using a `chunk_size` > `max_chunk_size` (misconfiguration)
    // to see if it catches it.

    let bad_params = ChunkParams {
        chunk_size: 200,           // Requested chunk size
        max_chunk_size: Some(100), // BUT hard limit is smaller!
        chunk_overlap: 0,
        tokenizer: "char".to_string(),
        file_extension: None,
    };

    let text = "a".repeat(300); // 300 chars

    let chunks = chunker
        .chunk(&text, &bad_params)
        .await
        .expect("Chunking failed");

    // With chunk_size=200, text_splitter might give 200-char chunks.
    // But max_chunk_size=100 should force them to be split further by SimpleChunker.

    for (i, chunk) in chunks.iter().enumerate() {
        assert!(
            chunk.content.len() <= 100,
            "Chunk {} size {} exceeds max 100",
            i,
            chunk.content.len()
        );
    }

    // We expect 3 chunks of 100 (or similar)
    assert!(chunks.len() >= 3);
}

#[tokio::test]
async fn test_code_chunker_enforces_max_size() {
    use vecdb_core::chunking::{Chunker, CodeChunker};

    let chunker = CodeChunker;
    let params = ChunkParams {
        chunk_size: 50,
        max_chunk_size: Some(50),
        chunk_overlap: 0,
        tokenizer: "char".to_string(),
        file_extension: Some("rs".to_string()),
    };

    // vecq parser might return a large block if it's a single function
    let code = format!("fn big_function() {{\n{}\n}}", "let x = 1;\n".repeat(10));
    // The content inside is 11 * 10 = 110 chars.
    // CodeChunker tries to keep blocks.

    let chunks = chunker
        .chunk(&code, &params)
        .await
        .expect("Chunking failed");

    for (i, chunk) in chunks.iter().enumerate() {
        assert!(
            chunk.content.len() <= 50,
            "Chunk {} size {} exceeds max 50",
            i,
            chunk.content.len()
        );
    }
}
