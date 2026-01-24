use anyhow::Result;
use std::path::PathBuf;
use vecdb_core::parsers::yaml::YamlParser;
use vecdb_core::parsers::Parser;
use vecdb_core::types::Chunk;

#[tokio::test]
async fn test_yaml_parsing_basics() -> Result<()> {
    let parser = YamlParser::new();

    let yaml_content = "name: vecdb\n";

    let path = PathBuf::from("test.yaml");
    let chunks: Vec<Chunk> = parser.parse(yaml_content, &path, None).await?;

    assert!(!chunks.is_empty(), "Should produce chunks");

    // Debug: print what chunks we got
    println!("Chunks:");
    for (i, chunk) in chunks.iter().enumerate() {
        println!("  {}: {:?}", i, chunk.content);
    }

    let combined = chunks
        .iter()
        .map(|c| c.content.as_str())
        .collect::<Vec<_>>()
        .join(" ");
    println!("Combined: {:?}", combined);
    // The YAML parser converts YAML to structured format, so check for the parsed content
    assert!(combined.contains("vecdb"));

    Ok(())
}
