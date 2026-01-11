/*
 * PURPOSE:
 *   Tier 1 integration test to verify the `Backend` trait contract.
 *   Ensures that the trait is implementable and usable by consumers.
 *   Functions as a "compile-time proof" of the abstraction.
 *
 * REQUIREMENTS:
 *   User-specified:
 *   - "Tests are Law" (TESTING_PHILOSOPHY.md)
 *   - Tier 1 must be fast (<5s) (TESTING_GUIDE.md)
 *
 *   Implementation-discovered:
 *   - Needs a simple Mock struct to implement the trait
 *
 * IMPLEMENTATION RULES:
 *   1. Use `Result` return types to match trait signature
 *   2. Use `async` test runtime (tokio::test)
 *
 * USAGE:
 *   `cargo test --test tier1_backend_mock`
 *
 * SELF-HEALING INSTRUCTIONS:
 *   - If trait definition changes: Update the `MockBackend` impl to match new signature.
 *
 * RELATED FILES:
 *   - vecdb-core/src/backend.rs
 *
 * MAINTENANCE:
 *   Update whenever `Backend` trait signatures change.
 */

use anyhow::Result;
use async_trait::async_trait;
use std::sync::{Arc, Mutex};
use vecdb_core::backend::Backend;
use vecdb_core::types::{Chunk, SearchResult};

// Simple in-memory mock backend
struct MockBackend {
    storage: Arc<Mutex<Vec<Chunk>>>,
}

impl MockBackend {
    fn new() -> Self {
        Self {
            storage: Arc::new(Mutex::new(Vec::new())),
        }
    }
}

#[async_trait]
impl Backend for MockBackend {
    async fn health_check(&self) -> Result<()> {
        Ok(())
    }

    async fn create_collection(&self, _name: &str, _vector_size: u64) -> Result<()> {
        Ok(())
    }

    async fn collection_exists(&self, _name: &str) -> Result<bool> {
        Ok(true)
    }

    async fn delete_collection(&self, _name: &str) -> Result<()> {
        let mut store = self.storage.lock().unwrap();
        store.clear();
        Ok(())
    }

    async fn upsert(&self, _collection: &str, chunks: Vec<Chunk>) -> Result<()> {
        let mut store = self.storage.lock().unwrap();
        store.extend(chunks);
        Ok(())
    }

    async fn search(
        &self,
        _collection: &str,
        _vector: &[f32],
        _limit: u64,
        _filter: Option<serde_json::Value>,
    ) -> Result<Vec<SearchResult>> {
        let store = self.storage.lock().unwrap();
        // Mock search just returns everything mapped to SearchResult with dummy score
        let results = store
            .iter()
            .map(|chunk| SearchResult {
                id: chunk.id.clone(),
                score: 1.0,
                content: chunk.content.clone(),
                document_id: chunk.document_id.clone(),
                metadata: chunk.metadata.clone(),
            })
            .collect();
        Ok(results)
    }

    async fn points_exists(&self, _collection: &str, ids: Vec<String>) -> Result<Vec<String>> {
        let store = self.storage.lock().unwrap();
        let existing: Vec<String> = store.iter()
            .filter(|c| ids.contains(&c.id))
            .map(|c| c.id.clone())
            .collect();
        Ok(existing)
    }

    async fn list_collections(&self) -> Result<Vec<String>> {
        Ok(vec!["default".to_string()])
    }

    async fn get_collection_info(&self, name: &str) -> Result<vecdb_core::types::CollectionInfo> {
        Ok(vecdb_core::types::CollectionInfo {
            name: name.to_string(),
            vector_count: Some(0),
            vector_size: Some(768),
        })
    }

    async fn list_metadata_values(&self, _collection: &str, _key: &str) -> Result<Vec<String>> {
        Ok(vec![])
    }
}

#[tokio::test]
async fn test_backend_trait_contract() -> Result<()> {
    // 1. Instantiate Mock
    let backend = MockBackend::new();

    // 2. Health Check
    backend.health_check().await?;

    // 3. Create Collection
    backend.create_collection("test", 768).await?;

    // 4. Upsert
    let chunk = Chunk::new("doc-1", "This is a test.");
    backend.upsert("test", vec![chunk.clone()]).await?;

    // 5. Search
    let results = backend.search("test", &[0.0; 768], 5, None).await?;
    
    // 6. Verify Interaction
    assert_eq!(results.len(), 1);
    assert_eq!(results[0].content, "This is a test.");

    Ok(())
}
