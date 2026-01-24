use anyhow::Result;
use serde_json::json;
use std::path::Path;
use vecdb_core::parsers::json::JsonParser;
use vecdb_core::parsers::Parser;
use vecdb_core::types::Chunk;

#[tokio::test]
async fn test_json_parser_invalid_metadata() -> Result<()> {
    let parser = JsonParser::new();
    let content = r#"{ "key": "value" }"#;
    let path = Path::new("test.json");

    // Case 1: Metadata is Null (should work, empty map)
    let chunks: Vec<Chunk> = parser.parse(content, path, None).await?;
    assert!(!chunks.is_empty());

    // Case 2: Metadata is not an object (e.g. Array) - Should gracefully handle it
    // Our fix forces it to be an empty map if it's not an object.
    let bad_metadata = json!(["not", "an", "object"]);
    let chunks: Vec<Chunk> = parser.parse(content, path, Some(bad_metadata)).await?;
    assert!(!chunks.is_empty());

    // Verify metadata was ignored/reset
    let first_chunk = &chunks[0];
    assert!(first_chunk.metadata.contains_key("source"));
    // Should NOT contain keys from the bad metadata since it wasn't a map

    Ok(())
}
