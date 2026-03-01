use anyhow::Result;
use std::path::PathBuf;
use vecdb_core::parsers::json::JsonParser;
use vecdb_core::parsers::Parser;
use vecdb_core::types::Chunk;

#[tokio::test]
async fn test_json_parsing_basics() -> Result<()> {
    // Direct instantiation reduces dependency on Factory logic for unit testing the parser itself
    let parser = JsonParser::new();

    let json_content = r#"{
    "name": "vecdb",
    "version": "0.1.0",
    "dependencies": ["serde", "tokio"]
}"#;

    let path = PathBuf::from("test.json");
    let chunks: Vec<Chunk> = parser.parse(json_content, &path, None).await?;

    // Basic assertions
    assert!(!chunks.is_empty(), "Should produce chunks");
    let combined = chunks
        .iter()
        .map(|c| c.content.as_str())
        .collect::<Vec<_>>()
        .join(" ");
    // Note: derived JSON strings are quoted
    assert!(combined.contains("\"name\": \"vecdb\"") || combined.contains("name: \"vecdb\""));

    Ok(())
}
