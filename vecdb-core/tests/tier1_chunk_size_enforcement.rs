use vecdb_core::chunking::{ChunkParams, Chunker, RecursiveChunker};

#[tokio::test]
async fn test_recursive_chunker_enforces_max_size() {
    let chunker = RecursiveChunker;
    let _params = ChunkParams {
        // Unused check params
        target_chunk_size: 100,
        max_chunk_bytes: Some(150), // Hard limit
        chunk_overlap: 0,
        tokenizer: "bytes".to_string(),
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
    // we can test the fallback logic by using a `target_chunk_size` > `max_chunk_bytes` (misconfiguration)
    // to see if it catches it.

    let bad_params = ChunkParams {
        target_chunk_size: 200,     // Requested chunk size
        max_chunk_bytes: Some(100), // BUT hard limit is smaller!
        chunk_overlap: 0,
        tokenizer: "bytes".to_string(),
        file_extension: None,
    };

    let text = "a".repeat(300); // 300 chars

    let chunks = chunker
        .chunk(&text, &bad_params)
        .await
        .expect("Chunking failed");

    // With target_chunk_size=200, text_splitter might give 200-char chunks.
    // But max_chunk_bytes=100 should force them to be split further by FixedWidthChunker.

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

// `test_code_chunker_enforces_max_size` was here.
//
// CodeChunker is gone: it was reachable only via `strategy = "code_aware"`, and
// a chunker never sees a source file — the parser's AST elements are used
// instead. `test_recursive_chunker_enforces_max_size` above covers the same
// property for the chunker that actually runs.
