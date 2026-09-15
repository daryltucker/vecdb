use std::collections::HashMap;
use std::env;
use vecdb_core::backend::Backend;
use vecdb_core::backends::qdrant::QdrantBackend;
use vecdb_core::config::Profile;
use vecdb_core::types::Chunk;

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

    println!(
        "Running Tier 2 Qdrant Backend Integration against {}",
        qdrant_url
    );

    // 1. Config
    let profile = Profile {
        pack_target_bytes: None,
        embedder: "default".to_string(),
        store: None,
        qdrant_url: Some(qdrant_url.clone()),
        qdrant_api_key: None,
        default_collection_name: Some("test_tier2_rust".to_string()),
        quantization: None,
        target_chunk_size: None,
        max_chunk_bytes: None,
        chunk_overlap: None,
        resolved_profile_name: "test".to_string(),
    };

    // 2. Init Backend
    // Note: QdrantBackend::new takes URL string and is synchronous
    let backend = QdrantBackend::new(
        profile.qdrant_url.as_deref().unwrap(),
        profile.qdrant_api_key.clone(),
    )
    .expect("Failed to create QdrantBackend");
    let collection = profile.default_collection_name.as_deref().unwrap();

    // 3. Health Check
    backend.health_check().await.expect("Health check failed");

    // 4. Create Collection (ensure fresh)
    let _ = backend.delete_collection(collection).await; // Ignore error if missing
    backend
        .create_collection(collection, 4, None)
        .await
        .expect("Failed to create collection"); // Size 4 for test

    // 5. Upsert
    //
    // A REAL UUID. This said `"chunk-1"`, which is not one — and the backend
    // used to fall back to `Uuid::default()`, the NIL uuid, which is the genesis
    // point. So this test spent its life writing its chunk into the genesis slot
    // and reading it back out again: green, for the wrong reason, while
    // demonstrating the exact corruption the backend now refuses. See the
    // refusal asserted at the end.
    let chunk = Chunk {
        id: "11111111-2222-4333-8444-555555555555".to_string(),
        document_id: "doc-1".to_string(),
        content: "rust integration test".to_string(),
        vector: Some(vec![0.1, 0.2, 0.3, 0.4]),
        metadata: HashMap::new(),
        page_num: None,
        byte_start: 0,
        byte_end: 20,
        start_line: None,
        end_line: None,
    };

    backend
        .upsert(collection, vec![chunk])
        .await
        .expect("Upsert failed");

    // Give Qdrant a split second to index? Usually consistent for small data but good practice
    tokio::time::sleep(tokio::time::Duration::from_millis(500)).await;

    // 6. Search
    let results = backend
        .search(
            collection,
            &[0.1, 0.2, 0.3, 0.4],
            vecdb_core::backend::SearchParams::new(10),
        )
        .await
        .expect("Search failed");

    assert!(!results.is_empty(), "Should find the inserted chunk");
    assert_eq!(results[0].content, "rust integration test");

    // 7. A chunk id that is not a UUID must be REFUSED, not silently rewritten.
    //
    // `Uuid::default()` is the nil UUID, which is where genesis lives. Accepting
    // an unparseable id meant replacing a collection's model and chunking record
    // with a content chunk — after which `read_genesis` finds no `__meta_vecdb`
    // marker, every guard concludes the collection belongs to another tool, and
    // nothing can be recovered. Unreachable in production is not a reason to
    // keep a fallback whose failure mode is unrecoverable.
    let bad = Chunk {
        id: "chunk-1".to_string(),
        document_id: "doc-1".to_string(),
        content: "not a uuid".to_string(),
        vector: Some(vec![0.1, 0.2, 0.3, 0.4]),
        metadata: HashMap::new(),
        page_num: None,
        byte_start: 0,
        byte_end: 10,
        start_line: None,
        end_line: None,
    };
    let err = backend
        .upsert(collection, vec![bad])
        .await
        .expect_err("a non-UUID chunk id must be refused, not mapped onto genesis")
        .to_string();
    assert!(
        err.contains("genesis"),
        "the refusal must say WHY it matters, so nobody \"fixes\" it by \
         restoring the fallback. Got: {err}"
    );

    // And a chunk that reached the backend without a vector, for the same reason:
    // an empty vector is not a vector, and Qdrant would fail on dimension several
    // layers away from the chunk that caused it.
    let unembedded = Chunk {
        id: "99999999-8888-4777-8666-555555555555".to_string(),
        document_id: "doc-1".to_string(),
        content: "never embedded".to_string(),
        vector: None,
        metadata: HashMap::new(),
        page_num: None,
        byte_start: 0,
        byte_end: 14,
        start_line: None,
        end_line: None,
    };
    backend
        .upsert(collection, vec![unembedded])
        .await
        .expect_err("a chunk with no vector must be refused at the backend boundary");

    // 8. Cleanup
    backend
        .delete_collection(collection)
        .await
        .expect("Cleanup failed");
}
