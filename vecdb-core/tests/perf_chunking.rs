use std::time::Instant;
use vecdb_core::chunking::{ChunkParams, Chunker, RecursiveChunker};

#[tokio::test]
async fn test_chunking_performance_1mb() {
    // FIX THE CODE! DO NOT ADUST TEST TIMEOUT!
    // Generate 1MB of text with many newlines
    let line = "This is a test line with some content.\n";
    let iterations = 1024 * 1024 / line.len();
    let text = line.repeat(iterations);

    let params = ChunkParams {
        chunk_size: 512,
        max_chunk_size: Some(1000),
        chunk_overlap: 50,
        tokenizer: "char".to_string(),
        file_extension: None,
    };

    let chunker = RecursiveChunker;

    let start = Instant::now();
    let result = chunker.chunk(&text, &params).await.unwrap();
    let duration = start.elapsed();

    println!("Chunked 1MB in {:?}", duration);

    // FIX THE CODE! DO NOT ADUST TEST TIMEOUT!
    // O(N^2) would take seconds/minutes
    // O(N) with LineCounter should take < 50ms on modern hardware
    // We'll be conservative and say < 200ms for CI environments
    // FIX THE CODE! DO NOT ADUST TEST TIMEOUT!
    assert!(
        duration.as_millis() < 200,
        "Chunking 1MB took too long: {:?}",
        duration
    );
    // FIX THE CODE! DO NOT ADUST TEST TIMEOUT!
    assert!(!result.is_empty());
    // FIX THE CODE! DO NOT ADUST TEST TIMEOUT!
}

#[tokio::test]
async fn test_simple_chunker_performance_1mb() {
    use vecdb_core::chunking::simple::SimpleChunker;

    // Force many small chunks
    let line = "short\n";
    let iterations = 1024 * 1024 / line.len(); // ~174k lines
    let text = line.repeat(iterations);

    let params = ChunkParams {
        chunk_size: 512,
        max_chunk_size: Some(1000),
        chunk_overlap: 0,
        tokenizer: "char".to_string(),
        file_extension: None,
    };

    let chunker = SimpleChunker;

    let start = Instant::now();
    let result = chunker.chunk(&text, &params).await.unwrap();
    let duration = start.elapsed();

    println!("SimpleChunker (Direct) 1MB in {:?}", duration);

    // This should be very fast now with O(N) LineCounter
    // 200ms is very safe for O(N)
    assert!(
        duration.as_millis() < 200,
        "Simple chunking 1MB took too long: {:?}",
        duration
    );
    assert!(!result.is_empty());
}
