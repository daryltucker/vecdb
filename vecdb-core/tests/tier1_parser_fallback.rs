use anyhow::{anyhow, Result};
use async_trait::async_trait;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use vecdb_common::{FileType, FileTypeDetector};
use vecdb_core::ingestion::{process_single_file, IngestionOptions};
use vecdb_core::parsers::{Parser, ParserFactory};
use vecdb_core::types::Chunk;

// --- Mocks ---

struct MockDetector;
impl FileTypeDetector for MockDetector {
    fn detect(&self, _path: &Path, _content: &[u8]) -> FileType {
        FileType::Python // Force Python to trigger code path
    }
}

struct FailingParser;
#[async_trait]
impl Parser for FailingParser {
    async fn parse(
        &self,
        _content: &str,
        _path: &Path,
        _base_metadata: Option<serde_json::Value>,
    ) -> Result<Vec<Chunk>> {
        // SIMULATE FAILURE
        Err(anyhow!("Simulated Parser Explosion 💥"))
    }

    fn supported_extensions(&self) -> Vec<&str> {
        vec!["py"]
    }
}

struct MockFactory;
impl ParserFactory for MockFactory {
    fn get_parser(&self, _file_type: FileType) -> Option<Box<dyn Parser>> {
        Some(Box::new(FailingParser))
    }

    fn get_streaming_parser(&self, _file_type: FileType) -> Option<Box<dyn Parser>> {
        None
    }
}

// --- Test ---

#[tokio::test]
async fn test_fallback_on_parser_failure() -> Result<()> {
    // Setup
    let detector = Arc::new(MockDetector);
    let parser_factory = Arc::new(MockFactory);
    let rules = vec![];

    let options = Arc::new(IngestionOptions {
        path: ".".to_string(),
        collection: "test".to_string(),
        vecdbrc_routes: None,
        vecdbrc_root: None,
        strategy: "code_aware".to_string(), // Request code aware!
        target_chunk_size: 100,
        max_chunk_bytes: None,
        on_oversize: Default::default(),
        route_chunking: Default::default(),
        chunk_overlap: 0,
        respect_gitignore: false,
        ignore_vectorignore: false,
        tokenizer: "bytes".to_string(),
        git_ref: None,
        extensions: None,
        excludes: None,
        dry_run: false,
        file_allowlist: None,
        project_root: None,
        metadata: None,
        path_rules: vec![],
        max_concurrent_requests: 1,
        gpu_batch_size: 1,
        quantization: None,
        allow_quantization_delta: false,
    });

    let dir = tempfile::tempdir()?;
    let file_path = dir.path().join("broken.py");
    tokio::fs::write(&file_path, "def foo():\n    print('hello')").await?;

    let rel_path = PathBuf::from("broken.py");

    // Execution
    let result = process_single_file(
        file_path.clone(),
        rel_path,
        vecdb_core::ingestion::processor::FileProcessor {
            detector,
            parser_factory,
            rules,
        },
        options,
        None,
        "test_parser_fallback".to_string(),
    )
    .await?;

    // Verification
    assert!(result.is_some(), "Result should be Some(chunks)");
    let chunks = result.unwrap();

    assert!(
        !chunks.is_empty(),
        "Should have produced chunks despite parser failure"
    );

    // Check that we got text chunks (simple metadata)
    let chunk = &chunks[0];

    // Verify it's a valid chunk
    assert_eq!(chunk.content, "def foo():\n    print('hello')");

    println!("Successfully fell back to text chunking!");

    Ok(())
}
