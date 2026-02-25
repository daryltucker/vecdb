use anyhow::Result;
use vecdb_core::parsers::streaming_json::StreamingJsonParser;
use vecdb_core::parsers::Parser;
use vecdb_core::types::Chunk;

#[tokio::test]
async fn test_streaming_json_parsing_basics() -> Result<()> {
    let parser = StreamingJsonParser::new();

    // Streaming parser handles large arrays usually
    let json_content = r#"[
        {"id": 1, "text": "foo"},
        {"id": 2, "text": "bar"}
    ]"#;

    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("large_stream.json");
    tokio::fs::write(&path, json_content).await.unwrap();

    // Pass the real path
    let chunks: Vec<Chunk> = parser.parse(json_content, &path, None).await?;

    // The streaming parser might produce different chunks structure, but should handle this.
    // If it fails on small content, that's a good test finding.
    assert!(!chunks.is_empty(), "Should produce chunks");

    // Check content
    let combined = chunks
        .iter()
        .map(|c| c.content.as_str())
        .collect::<Vec<_>>()
        .join(" ");
    assert!(combined.contains("\"foo\"") || combined.contains("id: 1"));

    Ok(())
}
