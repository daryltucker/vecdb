use anyhow::Result;
use async_trait::async_trait;
use std::sync::{Arc, Mutex};
use vecdb_common::FileTypeDetector;
use vecdb_core::backend::Backend;
use vecdb_core::embedders::MockEmbedder;
use vecdb_core::parsers::ParserFactory;
use vecdb_core::types::{Chunk, CollectionInfo, SearchResult};
use vecdb_core::Core;

struct MatryoshkaBackend {
    pub expected_dim: usize,
    pub search_called_with_dim: Arc<Mutex<Option<usize>>>,
}

#[async_trait]
impl Backend for MatryoshkaBackend {
    async fn health_check(&self) -> Result<()> {
        Ok(())
    }
    async fn create_collection(
        &self,
        _name: &str,
        _size: u64,
        _q: Option<vecdb_core::config::QuantizationType>,
    ) -> Result<()> {
        Ok(())
    }
    async fn update_collection_quantization(
        &self,
        _name: &str,
        _q: vecdb_core::config::QuantizationType,
    ) -> Result<()> {
        Ok(())
    }
    async fn collection_exists(&self, _name: &str) -> Result<bool> {
        Ok(true)
    }
    async fn delete_collection(&self, _name: &str) -> Result<()> {
        Ok(())
    }
    async fn upsert(&self, _collection: &str, _chunks: Vec<Chunk>) -> Result<()> {
        Ok(())
    }
    async fn search(
        &self,
        _collection: &str,
        vector: &[f32],
        _p: vecdb_core::backend::SearchParams,
    ) -> Result<Vec<SearchResult>> {
        let mut guard = self.search_called_with_dim.lock().unwrap();
        *guard = Some(vector.len());
        Ok(vec![])
    }
    async fn points_exists(&self, _collection: &str, _ids: Vec<String>) -> Result<Vec<String>> {
        Ok(vec![])
    }

    async fn delete_stale_points(
        &self,
        _c: &str,
        _d: &str,
        _k: &[String],
    ) -> anyhow::Result<usize> {
        Ok(0)
    }
    async fn list_collections(&self) -> Result<Vec<String>> {
        Ok(vec!["test".to_string()])
    }
    async fn get_collection_info(&self, name: &str) -> Result<CollectionInfo> {
        Ok(CollectionInfo {
            name: name.to_string(),
            vector_count: Some(100),
            vector_size: Some(self.expected_dim as u64),
            quantization: None,
            vectors_on_disk: None,
            payload_on_disk: None,
        })
    }
    async fn list_metadata_values(&self, _c: &str, _k: &str) -> Result<Vec<String>> {
        Ok(vec![])
    }
    async fn get_collection_id(&self, _c: &str) -> Result<Option<String>> {
        Ok(None)
    }
    async fn set_collection_id(&self, _c: &str, _id: &str) -> Result<()> {
        Ok(())
    }
    async fn list_tasks(&self) -> Result<Vec<vecdb_core::types::TaskInfo>> {
        Ok(vec![])
    }

    async fn write_genesis(
        &self,
        _c: &str,
        _m: &vecdb_core::types::GenesisMetadata,
    ) -> anyhow::Result<()> {
        Ok(())
    }
    async fn read_genesis(&self, _c: &str) -> anyhow::Result<vecdb_core::types::CollectionGenesis> {
        // Mirror MockEmbedder::identity so the space guard sees a matching
        // contract. `present: false` would (correctly) make every ingest here
        // fail as "not created by vecdb".
        Ok(vecdb_core::types::CollectionGenesis {
            collection_id: Some("mock-collection".to_string()),
            model: vecdb_core::types::ModelIdentity {
                name: "mock-embedder".to_string(),
                digest: Some("mock:test-double".to_string()),
                architecture: Some("mock".to_string()),
                family: Some("mock".to_string()),
                parameter_size: Some("0".to_string()),
                quantization_level: Some("none".to_string()),
                embedding_length: None,
                context_length: Some(8192),
            },
            dimension: None,
            distance: Some("Cosine".to_string()),
            created_at: None,
            vecdb_version: Some("test".to_string()),
            vecdb_revision: None,
            chunking: None,
        })
    }
}

struct DummyDetector;
impl FileTypeDetector for DummyDetector {
    fn detect(&self, _path: &std::path::Path, _content: &[u8]) -> vecdb_common::FileType {
        vecdb_common::FileType::Text
    }
}

struct DummyParserFactory;
impl ParserFactory for DummyParserFactory {
    fn get_parser(
        &self,
        _ft: vecdb_common::FileType,
    ) -> Option<Box<dyn vecdb_core::parsers::Parser>> {
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
    core.search("test", "hello", vecdb_core::backend::SearchParams::new(5))
        .await?;

    let final_dim = *search_dim.lock().unwrap();
    assert_eq!(
        final_dim,
        Some(384),
        "Vector should have been truncated to 384"
    );

    Ok(())
}
