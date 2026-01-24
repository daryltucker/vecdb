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
    async fn embed(&self, _text: &str) -> Result<Vec<f32>> {
        Ok(vec![0.1, 0.2, 0.3])
    }

    async fn embed_batch(&self, texts: &[String]) -> Result<Vec<Vec<f32>>> {
        Ok(vec![vec![0.1, 0.2, 0.3]; texts.len()])
    }

    async fn dimension(&self) -> Result<usize> {
        Ok(3)
    }

    fn model_name(&self) -> String {
        "mock-model".to_string()
    }
}

#[tokio::test]
async fn test_embedder_trait_contract() -> Result<()> {
    let embedder = MockEmbedder;

    // 1. Single embed
    let vec = embedder.embed("test").await?;
    assert_eq!(vec.len(), 3);
    assert_eq!(vec[0], 0.1);

    // 2. Batch embed
    let vecs = embedder
        .embed_batch(&["one".to_string(), "two".to_string()])
        .await?;
    assert_eq!(vecs.len(), 2);
    assert_eq!(vecs[0].len(), 3);

    Ok(())
}
