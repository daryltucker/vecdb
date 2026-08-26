/*
 * PURPOSE:
 *   Tier 1 integration test to verify the `Embedder` trait contract.
 *   Ensures that the trait is implementable and usable by consumers.
 */

use anyhow::Result;
use async_trait::async_trait;
use vecdb_core::embedder::Embedder;

struct MockEmbedder;

#[async_trait]
impl Embedder for MockEmbedder {
    async fn embed(&self, _text: &str, target_dim: Option<usize>) -> Result<Vec<f32>> {
        let dim = target_dim.unwrap_or(3);
        Ok(vec![0.1; dim])
    }

    async fn embed_batch(
        &self,
        texts: &[String],
        target_dim: Option<usize>,
    ) -> Result<Vec<Vec<f32>>> {
        let dim = target_dim.unwrap_or(3);
        Ok(vec![vec![0.1; dim]; texts.len()])
    }

    async fn dimension(&self) -> Result<usize> {
        Ok(3)
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

#[tokio::test]
async fn test_embedder_trait_contract() -> Result<()> {
    let embedder = MockEmbedder;

    // 1. Single embed
    let vec = embedder.embed("test", None).await?;
    assert_eq!(vec.len(), 3);
    assert_eq!(vec[0], 0.1);

    // 2. Batch embed
    let vecs = embedder
        .embed_batch(&["one".to_string(), "two".to_string()], None)
        .await?;
    assert_eq!(vecs.len(), 2);
    assert_eq!(vecs[0].len(), 3);

    Ok(())
}
