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
    async fn embed(&self, _text: &str, target_dim: Option<usize>) -> Result<Vec<f32>> {
        let dim = target_dim.unwrap_or(self.dimension);
        Ok(vec![0.1; dim])
    }

    async fn embed_batch(
        &self,
        texts: &[String],
        target_dim: Option<usize>,
    ) -> Result<Vec<Vec<f32>>> {
        let dim = target_dim.unwrap_or(self.dimension);
        let mut vecs = Vec::with_capacity(texts.len());
        for _ in texts {
            vecs.push(vec![0.1; dim]);
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
