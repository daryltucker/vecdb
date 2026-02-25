use anyhow::Result;
use async_trait::async_trait;
use std::sync::{Arc, Mutex};
use vecdb_core::backend::Backend;
use vecdb_core::embedder::Embedder;
use vecdb_core::types::{Chunk, CollectionInfo, SearchResult};

pub struct MockBackend {
    pub storage: Arc<Mutex<Vec<Chunk>>>,
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

    async fn collection_exists(&self, _name: &str) -> Result<bool> {
        Ok(true)
    }

    async fn delete_collection(&self, _name: &str) -> Result<()> {
        Ok(())
    }

    async fn upsert(&self, _collection: &str, chunks: Vec<Chunk>) -> Result<()> {
        let mut store = self.storage.lock().unwrap();
        // Simple append for mock
        store.extend(chunks);
        Ok(())
    }

    async fn search(
        &self,
        _c: &str,
        _v: &[f32],
        _l: u64,
        _f: Option<serde_json::Value>,
    ) -> Result<Vec<SearchResult>> {
        let store = self.storage.lock().unwrap();
        Ok(store
            .iter()
            .map(|c| SearchResult {
                id: c.id.clone(),
                score: 0.99,
                content: c.content.clone(),
                document_id: c.document_id.clone(),
                metadata: c.metadata.clone(),
            })
            .collect())
    }

    async fn points_exists(&self, _c: &str, ids: Vec<String>) -> Result<Vec<String>> {
        let store = self.storage.lock().unwrap();
        Ok(store
            .iter()
            .filter(|c| ids.contains(&c.id))
            .map(|c| c.id.clone())
            .collect())
    }

    async fn list_collections(&self) -> Result<Vec<String>> {
        Ok(vec!["docs".to_string()])
    }

    async fn get_collection_info(&self, name: &str) -> Result<CollectionInfo> {
        Ok(CollectionInfo {
            name: name.to_string(),
            vector_count: Some(100),
            vector_size: Some(3),
            quantization: None,
        })
    }

    async fn list_metadata_values(&self, _: &str, _: &str) -> anyhow::Result<Vec<String>> {
        Ok(vec![])
    }

    async fn update_collection_quantization(
        &self,
        _: &str,
        _: vecdb_core::config::QuantizationType,
    ) -> anyhow::Result<()> {
        Ok(())
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
}

pub struct MockEmbedder;

#[async_trait]
impl Embedder for MockEmbedder {
    async fn embed(&self, _text: &str, target_dim: Option<usize>) -> Result<Vec<f32>> {
        let dim = target_dim.unwrap_or(384);
        Ok(vec![0.1; dim])
    }

    async fn embed_batch(
        &self,
        texts: &[String],
        target_dim: Option<usize>,
    ) -> Result<Vec<Vec<f32>>> {
        let dim = target_dim.unwrap_or(384);
        Ok(vec![vec![0.1; dim]; texts.len()])
    }

    async fn dimension(&self) -> Result<usize> {
        Ok(3)
    }

    fn model_name(&self) -> String {
        "mock-model".to_string()
    }
}
