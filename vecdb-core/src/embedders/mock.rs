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

    /// A complete, fixed identity shared by every test double.
    ///
    /// Deliberately not the name-only default: the guard refuses to write when
    /// it cannot establish identity, so a name-only mock would make every
    /// ingestion test fail for the *right* reason while testing nothing. The
    /// shared sentinel digest makes all test doubles one space, which is what
    /// tests that are not about the guard actually want. Tests that ARE about
    /// the guard construct `ModelIdentity` values directly.
    async fn identity(&self) -> Result<crate::types::ModelIdentity> {
        Ok(crate::types::ModelIdentity {
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
