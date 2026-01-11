/*
 * PURPOSE:
 *   Unit tests for LocalEmbedder functionality.
 *   Verifies ONNX-based local embeddings work correctly.
 *
 * REQUIREMENTS:
 *   - LocalEmbedder must create without error (feature enabled)
 *   - Must generate embeddings of correct dimension
 *   - Must handle batch embedding correctly
 *
 * NOTE: Tests run serially to avoid concurrent model downloads.
 *       The model is downloaded on first use and cached.
 */

#[cfg(feature = "local-embed")]
mod local_embedder_tests {
    use vecdb_core::embedder::Embedder;
    use vecdb_core::embedders::LocalEmbedder;
    use std::sync::OnceLock;
    
    // Shared embedder instance to avoid concurrent downloads
    static EMBEDDER: OnceLock<LocalEmbedder> = OnceLock::new();
    
    fn get_embedder() -> &'static LocalEmbedder {
        EMBEDDER.get_or_init(|| {
            LocalEmbedder::new(None, false).expect("Failed to create LocalEmbedder")
        })
    }

    /// Basic creation test - runs first to download model
    #[tokio::test]
    async fn test_01_local_embedder_creation() {
        let embedder = get_embedder();
        // If we got here, creation succeeded
        assert!(embedder.model_name().len() > 0);
    }

    #[tokio::test]
    async fn test_02_local_embedder_dimension() {
        let embedder = get_embedder();
        
        // AllMiniLM-L6-v2 has 384 dimensions
        let dim = embedder.dimension().await.expect("Failed to get dimension");
        assert_eq!(dim, 384, "Expected 384 dimensions for AllMiniLM-L6-v2");
    }

    #[tokio::test]
    async fn test_03_local_embedder_single_embed() {
        let embedder = get_embedder();
        
        let text = "Hello, this is a test sentence for embedding.";
        let embedding = embedder.embed(text).await;
        
        assert!(embedding.is_ok(), "Embedding failed: {:?}", embedding.err());
        
        let vec = embedding.unwrap();
        assert_eq!(vec.len(), 384, "Embedding should have 384 dimensions");
        
        // Embeddings should be normalized (values between -1 and 1)
        for val in &vec {
            assert!(*val >= -1.0 && *val <= 1.0, "Value {} out of normalized range", val);
        }
    }

    #[tokio::test]
    async fn test_04_local_embedder_batch_embed() {
        let embedder = get_embedder();
        
        let texts = vec![
            "First sentence about vectors.".to_string(),
            "Second sentence about embeddings.".to_string(),
            "Third sentence about search.".to_string(),
        ];
        
        let embeddings = embedder.embed_batch(&texts).await;
        
        assert!(embeddings.is_ok(), "Batch embedding failed: {:?}", embeddings.err());
        
        let vecs = embeddings.unwrap();
        assert_eq!(vecs.len(), 3, "Should have 3 embeddings");
        
        for (i, vec) in vecs.iter().enumerate() {
            assert_eq!(vec.len(), 384, "Embedding {} should have 384 dimensions", i);
        }
    }

    #[tokio::test]
    async fn test_05_local_embedder_similarity() {
        let embedder = get_embedder();
        
        // Semantically similar sentences
        let similar1 = "The cat sat on the mat.";
        let similar2 = "A cat was sitting on the carpet.";
        
        // Semantically different sentence
        let different = "Quantum mechanics describes subatomic particles.";
        
        let emb1 = embedder.embed(similar1).await.unwrap();
        let emb2 = embedder.embed(similar2).await.unwrap();
        let emb3 = embedder.embed(different).await.unwrap();
        
        // Cosine similarity helper
        fn cosine(a: &[f32], b: &[f32]) -> f32 {
            let dot: f32 = a.iter().zip(b.iter()).map(|(x, y)| x * y).sum();
            let norm_a: f32 = a.iter().map(|x| x * x).sum::<f32>().sqrt();
            let norm_b: f32 = b.iter().map(|x| x * x).sum::<f32>().sqrt();
            dot / (norm_a * norm_b)
        }
        
        let sim_similar = cosine(&emb1, &emb2);
        let sim_different = cosine(&emb1, &emb3);
        
        // Similar sentences should have higher similarity
        assert!(
            sim_similar > sim_different,
            "Similar sentences ({:.4}) should have higher similarity than different ({:.4})",
            sim_similar,
            sim_different
        );
    }

    #[tokio::test]
    async fn test_06_local_embedder_empty_text() {
        let embedder = get_embedder();
        
        // Empty text should still produce an embedding (model handles it)
        let embedding = embedder.embed("").await;
        assert!(embedding.is_ok(), "Empty text embedding should succeed");
    }

    #[tokio::test]
    async fn test_07_local_embedder_model_name() {
        let embedder = get_embedder();
        
        let name = embedder.model_name();
        assert!(!name.is_empty(), "Model name should not be empty");
    }
}

// Test that features compile correctly when disabled
#[cfg(not(feature = "local-embed"))]
mod local_embedder_disabled_tests {
    #[test]
    fn test_local_embedder_disabled_compiles() {
        // This test just verifies the code compiles when feature is disabled
        // The LocalEmbedder struct exists but new() returns Err
    }
}
