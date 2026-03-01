use vecdb_core::chunking::ChunkParams;
use vecdb_core::chunking::Factory;

#[tokio::test]
async fn test_stability_multibyte_boundaries() {
    // SCENARIO 1: The "Killer" Multibyte String
    // "H\u{FFFD}llo" where \u{FFFD} is 3 bytes (EF BF BD)
    // Indices: H=0, \u{FFFD}=1..4, l=4
    // We force a max_chunk_size of 2, which lands INSIDE the multibyte char (index 2).
    // Previously, this panicked. Now it should split safely.

    let chunker = Factory::get("semantic", vecdb_common::FileType::Text);
    let text = "H\u{FFFD}lloWorld";

    let params = ChunkParams {
        chunk_size: 10,
        max_chunk_size: Some(2), // AGGRESSIVE limit
        chunk_overlap: 0,
        tokenizer: "char".to_string(),
        file_extension: None,
    };

    let result = chunker.chunk(text, &params).await;
    assert!(result.is_ok(), "Chunking panic on multibyte boundary");
    let chunks = result.unwrap();

    // Check forward progress
    assert!(!chunks.is_empty(), "Should produce chunks");
    let _reconstructed: String = chunks.iter().map(|c| c.content.clone()).collect();
    // It's acceptable if the fallback chunker produces slightly different validation,
    // but it MUST NOT crash and should output valid strings.
    // Note: SimpleChunker might split "H" and "\u{FFFD}llo" separately.
    println!(
        "Multibyte chunks: {:?}",
        chunks.iter().map(|c| &c.content).collect::<Vec<_>>()
    );
}

#[tokio::test]
async fn test_stability_binary_simulation() {
    // SCENARIO 2: "Binary-like" text
    // A string with null bytes that isn't caught by the binary detector (e.g. embedded nulls in code)
    // The TextSplitter might behave weirdly, but SimpleChunker must handle it fallback.

    let chunker = Factory::get("semantic", vecdb_common::FileType::Text);
    // "Header\0BodyResult"
    let text = "Header\0BodyStart\nLine2\nEnd";

    let params = ChunkParams {
        chunk_size: 10,
        max_chunk_size: Some(5), // Force fallback frequently
        chunk_overlap: 0,
        tokenizer: "char".to_string(), // Char tokenizer handles nulls better than token types potentially
        file_extension: None,
    };

    let result = chunker.chunk(text, &params).await;
    assert!(result.is_ok(), "Chunking panic on null bytes");
}
