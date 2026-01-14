/*
 * PURPOSE:
 *   Tier 1 integration test for standard ingestion flow via `Core`.
 */

use anyhow::Result;
use async_trait::async_trait;
use std::sync::{Arc, Mutex};
use vecdb_core::backend::Backend;
use vecdb_core::embedder::Embedder;
use vecdb_core::types::{Chunk, SearchResult};
use vecdb_core::Core;

struct MockBackend {
    storage: Arc<Mutex<Vec<Chunk>>>,
}

#[async_trait]
impl Backend for MockBackend {
    async fn health_check(&self) -> Result<()> { Ok(()) }
    async fn create_collection(&self, _name: &str, _v: u64, _q: Option<vecdb_core::config::QuantizationType>) -> Result<()> { Ok(()) }
    async fn update_collection_quantization(&self, _: &str, _: vecdb_core::config::QuantizationType) -> Result<()> { Ok(()) }
    async fn collection_exists(&self, _name: &str) -> Result<bool> { Ok(true) }
    async fn delete_collection(&self, _name: &str) -> Result<()> { Ok(()) }
    async fn upsert(&self, _collection: &str, chunks: Vec<Chunk>) -> Result<()> {
        let mut store = self.storage.lock().unwrap();
        store.extend(chunks);
        Ok(())
    }
    async fn search(&self, _c: &str, _v: &[f32], _l: u64, _f: Option<serde_json::Value>) -> Result<Vec<SearchResult>> {
        let store = self.storage.lock().unwrap();
        Ok(store.iter().map(|c| SearchResult {
            id: c.id.clone(),
            score: 1.0,
            content: c.content.clone(),
            document_id: c.document_id.clone(),
            metadata: c.metadata.clone(),
        }).collect())
    }
    async fn points_exists(&self, _c: &str, ids: Vec<String>) -> Result<Vec<String>> {
        let store = self.storage.lock().unwrap();
        Ok(store.iter().filter(|c| ids.contains(&c.id)).map(|c| c.id.clone()).collect())
    }
    async fn list_collections(&self) -> Result<Vec<String>> { Ok(vec![]) }
    async fn get_collection_info(&self, name: &str) -> Result<vecdb_core::types::CollectionInfo> {
        Ok(vecdb_core::types::CollectionInfo {
            name: name.to_string(),
            vector_count: Some(0),
            vector_size: Some(768),
            quantization: None,
        })
    }
    async fn list_metadata_values(&self, _c: &str, _k: &str) -> Result<Vec<String>> { Ok(vec![]) }
}

struct MockEmbedder;
#[async_trait]
impl Embedder for MockEmbedder {
    async fn embed(&self, _t: &str) -> Result<Vec<f32>> { Ok(vec![1.0, 2.0]) }
    async fn embed_batch(&self, texts: &[String]) -> Result<Vec<Vec<f32>>> { 
        Ok(vec![vec![1.0, 2.0]; texts.len()]) 
    }
    async fn dimension(&self) -> Result<usize> { Ok(2) }
    fn model_name(&self) -> String { "mock-model".to_string() }
}

struct MockFileTypeDetector;
impl vecdb_common::detection::FileTypeDetector for MockFileTypeDetector {
    fn detect(&self, _path: &std::path::Path, _content: &[u8]) -> vecdb_common::FileType { vecdb_common::FileType::Text }
}
struct MockParserFactory;
impl vecdb_core::parsers::ParserFactory for MockParserFactory {
    fn get_parser(&self, _file_type: vecdb_common::FileType) -> Option<Box<dyn vecdb_core::parsers::Parser>> { None }
}

#[tokio::test]
async fn test_core_ingestion_and_search() -> Result<()> {
    let backend = Arc::new(MockBackend { storage: Arc::new(Mutex::new(Vec::new())) });
    let embedder = Arc::new(MockEmbedder);
    let detector = Arc::new(MockFileTypeDetector);
    let parser_factory = Arc::new(MockParserFactory);
    let core = Core::with_backends(backend, embedder, detector, parser_factory, Vec::new(), Vec::new(), 1, 10);

    // 1. Ingest content
    let mut metadata = std::collections::HashMap::new();
    metadata.insert("source".to_string(), serde_json::json!("test"));
    core.ingest_content("Hello world this is a test content for embedding.", metadata, "test_collection", None, None, None, None).await?;

    // 2. Search
    let results: Vec<SearchResult> = core.search("test_collection", "world", 5, None).await?;

    // 3. Verify
    assert!(!results.is_empty());
    assert!(results[0].content.contains("world"));

    Ok(())
}
