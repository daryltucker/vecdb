use std::path::Path;
use vecdb_core::parsers::json::JsonParser;
use vecdb_core::parsers::Parser;

#[tokio::test]
async fn test_json_parser_handles_comments() {
    let parser = JsonParser;

    let json_content = r#"{
        "key": "value",
        // This is a comment
        "number": 123
    }"#;

    let path = Path::new("test.json");

    // This should fail with standard serde_json, but pass with our new implementation
    let result = parser.parse(json_content, path, None).await;

    assert!(result.is_ok(), "Parser should handle JSONC");
    let chunks = result.unwrap();
    assert!(!chunks.is_empty());
}

#[tokio::test]
async fn test_json_parser_handles_standard_json() {
    let parser = JsonParser;
    let json_content = r#"{"key": "value"}"#;
    let path = Path::new("standard.json");
    let result = parser.parse(json_content, path, None).await;
    assert!(result.is_ok());
}

#[tokio::test]
async fn test_json_parser_handles_trailing_commas() {
    let parser = JsonParser;
    let json_content = r#"{
        "key": "value",
        "list": [1, 2, ],
    }"#;

    let path = Path::new("trailing.json");
    // json5 handles trailing commas
    let result = parser.parse(json_content, path, None).await;
    assert!(
        result.is_ok(),
        "Should handle trailing commas via json5 fallback"
    );
}
