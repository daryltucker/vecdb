/*
 * PURPOSE:
 *   Tier 1 verification of ingestion idempotency.
 *   Ensures that identical content is not re-embedded.
 */

use anyhow::Result;
use async_trait::async_trait;
use std::sync::atomic::{AtomicUsize, Ordering};
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
    async fn health_check(&self) -> Result<()> {
        Ok(())
    }
    async fn create_collection(
        &self,
        _name: &str,
        _v: u64,
        _q: Option<vecdb_core::config::QuantizationType>,
    ) -> Result<()> {
        Ok(())
    }
    async fn update_collection_quantization(
        &self,
        _: &str,
        _: vecdb_core::config::QuantizationType,
    ) -> Result<()> {
        Ok(())
    }
    async fn collection_exists(&self, _name: &str) -> Result<bool> {
        Ok(true)
    }
    async fn delete_collection(&self, _name: &str) -> Result<()> {
        Ok(())
    }
    async fn upsert(&self, _collection: &str, chunks: Vec<Chunk>) -> Result<()> {
        let mut store = self.storage.lock().unwrap();
        // Dedup by ID in mock upsert to mimic Qdrant behavior
        for chunk in chunks {
            if !store.iter().any(|c| c.id == chunk.id) {
                store.push(chunk);
            }
        }
        Ok(())
    }
    async fn search(
        &self,
        _c: &str,
        _v: &[f32],
        _p: vecdb_core::backend::SearchParams,
    ) -> Result<Vec<SearchResult>> {
        Ok(vec![])
    }
    async fn points_exists(&self, _collection: &str, ids: Vec<String>) -> Result<Vec<String>> {
        let store = self.storage.lock().unwrap();
        Ok(store
            .iter()
            .filter(|c| ids.contains(&c.id))
            .map(|c| c.id.clone())
            .collect())
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
        Ok(vec![])
    }
    async fn get_collection_info(&self, name: &str) -> Result<vecdb_core::types::CollectionInfo> {
        Ok(vecdb_core::types::CollectionInfo {
            name: name.to_string(),
            vector_count: Some(0),
            vector_size: Some(768),
            quantization: None,
            vectors_on_disk: None,
            payload_on_disk: None,
        })
    }
    async fn list_metadata_values(&self, _c: &str, _k: &str) -> Result<Vec<String>> {
        Ok(vec![])
    }
    async fn get_collection_id(&self, _collection: &str) -> anyhow::Result<Option<String>> {
        Ok(None)
    }
    async fn set_collection_id(&self, _collection: &str, _id: &str) -> anyhow::Result<()> {
        Ok(())
    }
    async fn list_tasks(&self) -> anyhow::Result<Vec<vecdb_core::types::TaskInfo>> {
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

struct CountingEmbedder {
    count: Arc<AtomicUsize>,
}
#[async_trait]
impl Embedder for CountingEmbedder {
    async fn embed(&self, _text: &str, target_dim: Option<usize>) -> Result<Vec<f32>> {
        self.count.fetch_add(1, Ordering::SeqCst);
        let dim = target_dim.unwrap_or(2);
        Ok(vec![0.1; dim])
    }

    async fn embed_batch(
        &self,
        texts: &[String],
        target_dim: Option<usize>,
    ) -> Result<Vec<Vec<f32>>> {
        self.count.fetch_add(texts.len(), Ordering::SeqCst);
        let dim = target_dim.unwrap_or(2);
        Ok(vec![vec![0.1; dim]; texts.len()])
    }
    async fn dimension(&self) -> Result<usize> {
        Ok(2)
    }
    fn model_name(&self) -> String {
        "mock-model".to_string()
    }

    async fn identity(&self) -> anyhow::Result<vecdb_core::types::ModelIdentity> {
        // Shared sentinel: every test double is one embedding space, so the
        // compatibility guard passes and these tests exercise what they are
        // actually about. Guard behaviour has its own dedicated tests.
        Ok(vecdb_core::types::ModelIdentity {
            name: "mock-embedder".to_string(),
            digest: Some("mock:test-double".to_string()),
            architecture: Some("mock".to_string()),
            family: Some("mock".to_string()),
            parameter_size: Some("0".to_string()),
            quantization_level: Some("none".to_string()),
            embedding_length: None,
            context_length: Some(8192),
        })
    }
}

struct MockFileTypeDetector;
impl vecdb_common::detection::FileTypeDetector for MockFileTypeDetector {
    fn detect(&self, _path: &std::path::Path, _content: &[u8]) -> vecdb_common::FileType {
        vecdb_common::FileType::Text
    }
}
struct MockParserFactory;
impl vecdb_core::parsers::ParserFactory for MockParserFactory {
    fn get_parser(
        &self,
        _file_type: vecdb_common::FileType,
    ) -> Option<Box<dyn vecdb_core::parsers::Parser>> {
        None
    }
    fn get_streaming_parser(
        &self,
        _file_type: vecdb_common::FileType,
    ) -> Option<Box<dyn vecdb_core::parsers::Parser>> {
        None
    }
}

#[tokio::test]
async fn test_ingestion_idempotency() -> Result<()> {
    let backend = Arc::new(MockBackend {
        storage: Arc::new(Mutex::new(Vec::new())),
    });
    let count = Arc::new(AtomicUsize::new(0));
    let embedder = Arc::new(CountingEmbedder {
        count: count.clone(),
    });
    let detector = Arc::new(MockFileTypeDetector);
    let parser_factory = Arc::new(MockParserFactory);
    let core = Core::with_backends(
        backend,
        embedder,
        detector,
        parser_factory,
        Vec::new(),
        Vec::new(),
        1,
        10,
    );

    let content = "This is a unique string that should only be embedded once.";
    let metadata = std::collections::HashMap::new();

    // 1. First ingestion
    core.ingest_content(
        content,
        metadata.clone(),
        "test",
        None,
        None,
        None,
        None,
        None,
    )
    .await?;
    let first_count = count.load(Ordering::SeqCst);
    assert!(first_count > 0, "Should have embedded content once");

    // 2. Second ingestion (identical content)
    core.ingest_content(content, metadata, "test", None, None, None, None, None)
        .await?;
    let second_count = count.load(Ordering::SeqCst);

    // 3. Verify count didn't increase
    assert_eq!(
        first_count, second_count,
        "Should NOT have re-embedded identical content"
    );

    Ok(())
}
