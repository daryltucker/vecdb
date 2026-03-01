use anyhow::Result;
use async_trait::async_trait;
use std::sync::{Arc, Mutex};
use vecdb_core::backend::Backend;
use vecdb_core::embedders::MockEmbedder;
use vecdb_core::types::{Chunk, SearchResult, CollectionInfo};
use vecdb_core::Core;
use vecdb_common::FileTypeDetector;
use vecdb_core::parsers::ParserFactory;

struct MatryoshkaBackend {
    pub expected_dim: usize,
    pub search_called_with_dim: Arc<Mutex<Option<usize>>>,
}

#[async_trait]
impl Backend for MatryoshkaBackend {
    async fn health_check(&self) -> Result<()> { Ok(()) }
    async fn create_collection(&self, _name: &str, _size: u64, _q: Option<vecdb_core::config::QuantizationType>) -> Result<()> { Ok(()) }
    async fn update_collection_quantization(&self, _name: &str, _q: vecdb_core::config::QuantizationType) -> Result<()> { Ok(()) }
    async fn collection_exists(&self, _name: &str) -> Result<bool> { Ok(true) }
    async fn delete_collection(&self, _name: &str) -> Result<()> { Ok(()) }
    async fn upsert(&self, _collection: &str, _chunks: Vec<Chunk>) -> Result<()> { Ok(()) }
    async fn search(&self, _collection: &str, vector: &[f32], _limit: u64, _filter: Option<serde_json::Value>) -> Result<Vec<SearchResult>> {
        let mut guard = self.search_called_with_dim.lock().unwrap();
        *guard = Some(vector.len());
        Ok(vec![])
    }
    async fn points_exists(&self, _collection: &str, _ids: Vec<String>) -> Result<Vec<String>> { Ok(vec![]) }
    async fn list_collections(&self) -> Result<Vec<String>> { Ok(vec!["test".to_string()]) }
    async fn get_collection_info(&self, name: &str) -> Result<CollectionInfo> {
        Ok(CollectionInfo {
            name: name.to_string(),
            vector_count: Some(100),
            vector_size: Some(self.expected_dim as u64),
            quantization: None,
        })
    }
    async fn list_metadata_values(&self, _c: &str, _k: &str) -> Result<Vec<String>> { Ok(vec![]) }
    async fn get_collection_id(&self, _c: &str) -> Result<Option<String>> { Ok(None) }
    async fn set_collection_id(&self, _c: &str, _id: &str) -> Result<()> { Ok(()) }
    async fn list_tasks(&self) -> Result<Vec<vecdb_core::types::TaskInfo>> { Ok(vec![]) }
}

struct DummyDetector;
impl FileTypeDetector for DummyDetector {
    fn detect(&self, _path: &std::path::Path, _content: &[u8]) -> vecdb_common::FileType {
        vecdb_common::FileType::Text
    }
}

struct DummyParserFactory;
impl ParserFactory for DummyParserFactory {
    fn get_parser(&self, _ft: vecdb_common::FileType) -> Option<Box<dyn vecdb_core::parsers::Parser>> {
        None
    }
}

#[tokio::test]
async fn test_matryoshka_truncation_on_search() -> Result<()> {
    let search_dim = Arc::new(Mutex::new(None));
    let backend = Arc::new(MatryoshkaBackend {
        expected_dim: 384,
        search_called_with_dim: search_dim.clone(),
    });

    // Embedder is initialized as 768-dim
    let embedder = Arc::new(MockEmbedder::new(768));

    let core = Core::with_backends(
        backend,
        embedder,
        Arc::new(DummyDetector),
        Arc::new(DummyParserFactory),
        vec![],
        vec![],
        4,
        2,
    );

    // Search should trigger truncation to 384
    core.search("test", "hello", 5, None).await?;

    let final_dim = *search_dim.lock().unwrap();
    assert_eq!(final_dim, Some(384), "Vector should have been truncated to 384");

    Ok(())
}
