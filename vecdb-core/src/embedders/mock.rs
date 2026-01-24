use crate::embedder::Embedder;
use anyhow::Result;
use async_trait::async_trait;

/// A Mock Embedder for testing functionality without loading heavy ML models.
/// Can be configured to return deterministic vectors.
pub struct MockEmbedder {
    pub dimension: usize,
}

impl MockEmbedder {
    pub fn new(dimension: usize) -> Self {
        Self { dimension }
    }
}

#[async_trait]
impl Embedder for MockEmbedder {
    async fn embed(&self, _text: &str) -> Result<Vec<f32>> {
        // Return a deterministic vector based on dimension (e.g., all 0.1s)
        // Or hash the text to make it slightly deterministic but distinct
        Ok(vec![0.1; self.dimension])
    }

    async fn embed_batch(&self, texts: &[String]) -> Result<Vec<Vec<f32>>> {
        let mut vecs = Vec::with_capacity(texts.len());
        for _ in texts {
            vecs.push(vec![0.1; self.dimension]);
        }
        Ok(vecs)
    }

    async fn dimension(&self) -> Result<usize> {
        Ok(self.dimension)
    }

    fn model_name(&self) -> String {
        "mock-embedder".to_string()
    }
}
