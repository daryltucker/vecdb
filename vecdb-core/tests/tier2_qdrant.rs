
use vecdb_core::backend::Backend;
use vecdb_core::backends::qdrant::QdrantBackend;
use vecdb_core::config::Profile;
use vecdb_core::types::Chunk;
use std::env;
use std::collections::HashMap;

// Only run this test if specific env var is set (by test_runner.sh)
// This prevents 'cargo test' from failing in environments without Qdrant
#[tokio::test]
async fn test_qdrant_backend_integration() {
    let qdrant_url = match env::var("VECDB_TEST_QDRANT_URL") {
        Ok(url) => url,
        Err(_) => {
            println!("Skipping tier2_qdrant: VECDB_TEST_QDRANT_URL not set");
            return;
        }
    };

    println!("Running Tier 2 Qdrant Backend Integration against {}", qdrant_url);

    // 1. Config
    let profile = Profile {
        qdrant_url: qdrant_url.clone(),
        default_collection_name: "tier2_rust_test".to_string(), // Unique name
        ollama_url: "http://localhost:11434".to_string(),
        embedding_model: "nomic-embed-text".to_string(),
        embedder_type: "local".to_string(),
        accept_invalid_certs: true,
        qdrant_api_key: None,
        ollama_api_key: None,
        quantization: None,
    };

    // 2. Init Backend
    // Note: QdrantBackend::new takes URL string and is synchronous
    let backend = QdrantBackend::new(&profile.qdrant_url, profile.qdrant_api_key.clone()).expect("Failed to create QdrantBackend");

    // 3. Health Check
    backend.health_check().await.expect("Health check failed");

    // 4. Create Collection (ensure fresh)
    let _ = backend.delete_collection(&profile.default_collection_name).await; // Ignore error if missing
    backend.create_collection(&profile.default_collection_name, 4, None).await.expect("Failed to create collection"); // Size 4 for test

    // 5. Upsert
    let chunk = Chunk {
        id: "chunk-1".to_string(),
        document_id: "doc-1".to_string(),
        content: "rust integration test".to_string(),
        vector: Some(vec![0.1, 0.2, 0.3, 0.4]),
        metadata: HashMap::new(),
        page_num: None,
        char_start: 0,
        char_end: 20,
        start_line: None,
        end_line: None,
    };

    backend.upsert(&profile.default_collection_name, vec![chunk]).await.expect("Upsert failed");

    // Give Qdrant a split second to index? Usually consistent for small data but good practice
    tokio::time::sleep(tokio::time::Duration::from_millis(500)).await;

    // 6. Search
    let results = backend.search(
        &profile.default_collection_name,
        &[0.1, 0.2, 0.3, 0.4],
        10,
        None
    ).await.expect("Search failed");

    assert!(!results.is_empty(), "Should find the inserted chunk");
    assert_eq!(results[0].content, "rust integration test");

    // 7. Cleanup
    backend.delete_collection(&profile.default_collection_name).await.expect("Cleanup failed");
}
